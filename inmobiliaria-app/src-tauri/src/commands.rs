use crate::config::SupabaseConfig;
use crate::icl;
use crate::models::*;
use crate::supabase::{Cliente, ErrorSupabase, FOREIGN_KEY_VIOLATION, UNIQUE_VIOLATION};
use chrono::{Datelike, NaiveDate};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{Manager, State};
use tokio::sync::RwLock;

pub struct DbState(pub RwLock<Option<Cliente>>);

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Para deserializar la fila devuelta por un INSERT cuando solo hace falta
/// el id (p.ej. contratos, que trae columnas que no están en el modelo
/// `Contrato` usado por el frontend, como `garante_ids`).
#[derive(Deserialize)]
struct FilaId {
    id: i64,
}

async fn obtener_cliente(state: &DbState) -> Result<Cliente, String> {
    state
        .0
        .read()
        .await
        .clone()
        .ok_or_else(|| "La aplicación todavía no está conectada a Supabase. Configurá la conexión primero.".to_string())
}

fn parse_date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap_or_else(|_| chrono::Local::now().date_naive())
}

fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let next_month = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .unwrap();
    let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    (next_month - first).num_days() as u32
}

fn add_months(date: NaiveDate, months: i64) -> NaiveDate {
    let total = date.year() as i64 * 12 + (date.month() as i64 - 1) + months;
    let year = (total.div_euclid(12)) as i32;
    let month = (total.rem_euclid(12) + 1) as u32;
    let day = date.day().min(days_in_month(year, month));
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

fn due_date_for_periodo(periodo: &str, dia_pago: i64) -> NaiveDate {
    let partes: Vec<&str> = periodo.split('-').collect();
    let year: i32 = partes[0].parse().unwrap_or(today().year());
    let month: u32 = partes[1].parse().unwrap_or(1);
    let dia = (dia_pago as u32).clamp(1, days_in_month(year, month));
    NaiveDate::from_ymd_opt(year, month, dia).unwrap()
}

fn periodo_de(date: NaiveDate) -> String {
    format!("{:04}-{:02}", date.year(), date.month())
}

/// Próxima fecha (hoy o en el futuro) en que la persona cumple años.
/// 29 de febrero en años no bisiestos se festeja el 28.
fn proxima_fecha_cumpleanos(nacimiento: NaiveDate, hoy: NaiveDate) -> NaiveDate {
    let dia = nacimiento.day().min(days_in_month(hoy.year(), nacimiento.month()));
    let candidato = NaiveDate::from_ymd_opt(hoy.year(), nacimiento.month(), dia).unwrap();
    if candidato >= hoy {
        candidato
    } else {
        let anio = hoy.year() + 1;
        let dia = nacimiento.day().min(days_in_month(anio, nacimiento.month()));
        NaiveDate::from_ymd_opt(anio, nacimiento.month(), dia).unwrap()
    }
}

#[derive(Debug, Deserialize, Clone)]
struct ActualizacionBase {
    fecha_vigencia: String,
    monto_nuevo: f64,
}

/// Monto de alquiler vigente a una fecha dada, según la actualización
/// registrada más reciente con vigencia en o antes de esa fecha.
fn monto_vigente_de(actualizaciones: &[ActualizacionBase], monto_inicial: f64, fecha: NaiveDate) -> f64 {
    actualizaciones
        .iter()
        .filter(|a| parse_date(&a.fecha_vigencia) <= fecha)
        .max_by(|a, b| a.fecha_vigencia.cmp(&b.fecha_vigencia))
        .map(|a| a.monto_nuevo)
        .unwrap_or(monto_inicial)
}

/// Fecha de vigencia de la última actualización registrada (la más reciente,
/// sin importar si es pasada o futura respecto de "hoy"), o la fecha de
/// inicio del contrato si todavía no tuvo ninguna. Es el punto de referencia
/// contra el que se mide la variación del índice para la próxima actualización.
fn fecha_base_de(actualizaciones: &[ActualizacionBase], fecha_inicio: NaiveDate) -> NaiveDate {
    actualizaciones
        .iter()
        .max_by(|a, b| a.fecha_vigencia.cmp(&b.fecha_vigencia))
        .map(|a| parse_date(&a.fecha_vigencia))
        .unwrap_or(fecha_inicio)
}

fn proxima_fecha_actualizacion_de(actualizaciones: &[ActualizacionBase], fecha_inicio: NaiveDate, frecuencia_meses: i64) -> NaiveDate {
    add_months(fecha_base_de(actualizaciones, fecha_inicio), frecuencia_meses.max(1))
}

async fn actualizaciones_de(cliente: &Cliente, contrato_id: i64) -> Result<Vec<ActualizacionBase>, String> {
    cliente
        .select("actualizaciones", &format!("contrato_id=eq.{}&select=fecha_vigencia,monto_nuevo", contrato_id))
        .await
        .map_err(map_err)
}

// ---------- Conexión a Supabase ----------

/// URL y clave "anon" de fábrica, incluidas en el instalador en tiempo de
/// compilación (variables de entorno INMOBILIARIA_SUPABASE_URL e
/// INMOBILIARIA_SUPABASE_ANON_KEY en el build de GitHub Actions). Permiten
/// que la app venga lista para usar sin que la persona que la instala tenga
/// que pegar ningún dato técnico. Si no están presentes (build local de
/// desarrollo), la app pide los datos a mano la primera vez, como antes.
const SUPABASE_URL_DE_FABRICA: Option<&str> = option_env!("INMOBILIARIA_SUPABASE_URL");
const SUPABASE_ANON_KEY_DE_FABRICA: Option<&str> = option_env!("INMOBILIARIA_SUPABASE_ANON_KEY");

async fn conectar_y_guardar(app: &tauri::AppHandle, state: &State<'_, DbState>, config: SupabaseConfig) -> Result<(), String> {
    let cliente = Cliente::nuevo(&config);
    cliente.probar().await.map_err(|e| {
        format!(
            "No se pudo conectar con Supabase. Verificá la URL, la clave y tu conexión a internet. Detalle: {}",
            e
        )
    })?;
    let data_dir = app.path().app_data_dir().map_err(map_err)?;
    std::fs::create_dir_all(&data_dir).map_err(map_err)?;
    crate::config::guardar(&data_dir, &config)?;
    *state.0.write().await = Some(cliente);
    Ok(())
}

/// true si ya hay una conexión activa, si existe una configuración guardada
/// de una sesión anterior y se pudo restablecer, o si se pudo conectar con
/// los datos de fábrica incluidos en el instalador. false solo si hace falta
/// pedirlos a mano (primera vez en un build sin datos de fábrica).
#[tauri::command]
pub async fn hay_configuracion_conexion(app: tauri::AppHandle, state: State<'_, DbState>) -> Result<bool, String> {
    if state.0.read().await.is_some() {
        return Ok(true);
    }
    let data_dir = app.path().app_data_dir().map_err(map_err)?;
    if let Some(config) = crate::config::leer_guardada(&data_dir) {
        conectar_y_guardar(&app, &state, config).await?;
        return Ok(true);
    }
    if let (Some(url), Some(anon_key)) = (SUPABASE_URL_DE_FABRICA, SUPABASE_ANON_KEY_DE_FABRICA) {
        conectar_y_guardar(&app, &state, SupabaseConfig { url: url.to_string(), anon_key: anon_key.to_string() }).await?;
        return Ok(true);
    }
    Ok(false)
}

#[tauri::command]
pub async fn configurar_conexion(app: tauri::AppHandle, state: State<'_, DbState>, url: String, anon_key: String) -> Result<(), String> {
    let config = SupabaseConfig { url: crate::config::limpiar(&url), anon_key: crate::config::limpiar(&anon_key) };
    conectar_y_guardar(&app, &state, config).await
}

// ---------- Propietarios ----------

#[tauri::command]
pub async fn get_propietarios(state: State<'_, DbState>) -> Result<Vec<Propietario>, String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.select("propietarios", "select=*&order=nombre").await.map_err(map_err)
}

#[tauri::command]
pub async fn guardar_propietario(state: State<'_, DbState>, propietario: Propietario) -> Result<i64, String> {
    let cliente = obtener_cliente(&state).await?;
    let body = json!({
        "nombre": propietario.nombre, "dni_cuit": propietario.dni_cuit, "fecha_nacimiento": propietario.fecha_nacimiento,
        "telefono": propietario.telefono, "email": propietario.email, "direccion": propietario.direccion,
        "datos_bancarios": propietario.datos_bancarios, "notas": propietario.notas,
    });
    match propietario.id {
        Some(id) => {
            cliente.update("propietarios", &format!("id=eq.{}", id), &body).await.map_err(map_err)?;
            Ok(id)
        }
        None => {
            let fila: Propietario = cliente.insert("propietarios", &body).await.map_err(map_err)?;
            Ok(fila.id.unwrap())
        }
    }
}

#[tauri::command]
pub async fn eliminar_propietario(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.delete("propietarios", &format!("id=eq.{}", id)).await.map_err(mensaje_borrado_restringido("propietario"))
}

// ---------- Inquilinos ----------

#[tauri::command]
pub async fn get_inquilinos(state: State<'_, DbState>) -> Result<Vec<Inquilino>, String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.select("inquilinos", "select=*&order=nombre").await.map_err(map_err)
}

#[tauri::command]
pub async fn guardar_inquilino(state: State<'_, DbState>, inquilino: Inquilino) -> Result<i64, String> {
    let cliente = obtener_cliente(&state).await?;
    let body = json!({
        "nombre": inquilino.nombre, "dni_cuit": inquilino.dni_cuit, "fecha_nacimiento": inquilino.fecha_nacimiento,
        "telefono": inquilino.telefono, "email": inquilino.email, "direccion": inquilino.direccion, "notas": inquilino.notas,
    });
    match inquilino.id {
        Some(id) => {
            cliente.update("inquilinos", &format!("id=eq.{}", id), &body).await.map_err(map_err)?;
            Ok(id)
        }
        None => {
            let fila: Inquilino = cliente.insert("inquilinos", &body).await.map_err(map_err)?;
            Ok(fila.id.unwrap())
        }
    }
}

#[tauri::command]
pub async fn eliminar_inquilino(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.delete("inquilinos", &format!("id=eq.{}", id)).await.map_err(mensaje_borrado_restringido("inquilino"))
}

// ---------- Garantes ----------

#[tauri::command]
pub async fn get_garantes(state: State<'_, DbState>) -> Result<Vec<Garante>, String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.select("garantes", "select=*&order=nombre").await.map_err(map_err)
}

#[tauri::command]
pub async fn guardar_garante(state: State<'_, DbState>, garante: Garante) -> Result<i64, String> {
    let cliente = obtener_cliente(&state).await?;
    let body = json!({
        "nombre": garante.nombre, "dni_cuit": garante.dni_cuit, "fecha_nacimiento": garante.fecha_nacimiento,
        "telefono": garante.telefono, "email": garante.email, "direccion": garante.direccion, "notas": garante.notas,
    });
    match garante.id {
        Some(id) => {
            cliente.update("garantes", &format!("id=eq.{}", id), &body).await.map_err(map_err)?;
            Ok(id)
        }
        None => {
            let fila: Garante = cliente.insert("garantes", &body).await.map_err(map_err)?;
            Ok(fila.id.unwrap())
        }
    }
}

#[tauri::command]
pub async fn eliminar_garante(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.delete("garantes", &format!("id=eq.{}", id)).await.map_err(mensaje_borrado_restringido("garante"))
}

// ---------- Inmuebles ----------

#[derive(Debug, Deserialize)]
struct PropietarioNombre {
    nombre: String,
}

#[derive(Debug, Deserialize)]
struct InmuebleEmbed {
    id: i64,
    propietario_id: i64,
    direccion: String,
    tipo: Option<String>,
    superficie: Option<f64>,
    ambientes: Option<i64>,
    notas: Option<String>,
    propietarios: PropietarioNombre,
}

#[tauri::command]
pub async fn get_inmuebles(state: State<'_, DbState>) -> Result<Vec<InmuebleDetallado>, String> {
    let cliente = obtener_cliente(&state).await?;
    let filas: Vec<InmuebleEmbed> = cliente
        .select("inmuebles", "select=*,propietarios(nombre)&order=direccion")
        .await
        .map_err(map_err)?;
    Ok(filas
        .into_iter()
        .map(|i| InmuebleDetallado {
            id: i.id,
            propietario_id: i.propietario_id,
            propietario_nombre: i.propietarios.nombre,
            direccion: i.direccion,
            tipo: i.tipo,
            superficie: i.superficie,
            ambientes: i.ambientes,
            notas: i.notas,
        })
        .collect())
}

#[tauri::command]
pub async fn guardar_inmueble(state: State<'_, DbState>, inmueble: Inmueble) -> Result<i64, String> {
    let cliente = obtener_cliente(&state).await?;
    let body = json!({
        "propietario_id": inmueble.propietario_id, "direccion": inmueble.direccion, "tipo": inmueble.tipo,
        "superficie": inmueble.superficie, "ambientes": inmueble.ambientes, "notas": inmueble.notas,
    });
    match inmueble.id {
        Some(id) => {
            cliente.update("inmuebles", &format!("id=eq.{}", id), &body).await.map_err(map_err)?;
            Ok(id)
        }
        None => {
            let fila: Inmueble = cliente.insert("inmuebles", &body).await.map_err(map_err)?;
            Ok(fila.id.unwrap())
        }
    }
}

#[tauri::command]
pub async fn eliminar_inmueble(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.delete("inmuebles", &format!("id=eq.{}", id)).await.map_err(mensaje_borrado_restringido("inmueble"))
}

// ---------- Contratos ----------

#[derive(Debug, Deserialize)]
struct InquilinoNombre {
    nombre: String,
}

#[derive(Debug, Deserialize)]
struct GaranteEmbed {
    id: i64,
    nombre: String,
}

#[derive(Debug, Deserialize)]
struct ContratoGaranteEmbed {
    garantes: GaranteEmbed,
}

#[derive(Debug, Deserialize)]
struct InmuebleDeContrato {
    direccion: String,
    propietario_id: i64,
    propietarios: PropietarioNombre,
}

#[derive(Debug, Deserialize)]
struct ContratoEmbed {
    id: i64,
    inmueble_id: i64,
    inquilino_id: i64,
    fecha_inicio: String,
    fecha_fin: String,
    dia_pago: i64,
    monto_inicial: f64,
    comision_porcentaje: f64,
    tasa_mora_diaria: f64,
    frecuencia_actualizacion_meses: i64,
    tipo_actualizacion: String,
    porcentaje_actualizacion: f64,
    estado: String,
    notas: Option<String>,
    inmuebles: InmuebleDeContrato,
    inquilinos: InquilinoNombre,
    contrato_garantes: Vec<ContratoGaranteEmbed>,
    actualizaciones: Vec<ActualizacionBase>,
}

const SELECT_CONTRATO_DETALLADO: &str = "select=*,inmuebles(direccion,propietario_id,propietarios(nombre)),inquilinos(nombre),contrato_garantes(garantes(id,nombre)),actualizaciones(fecha_vigencia,monto_nuevo)";

fn contrato_embed_a_detallado(c: ContratoEmbed, hoy: NaiveDate) -> ContratoDetallado {
    let fecha_inicio = parse_date(&c.fecha_inicio);
    let monto_vig = monto_vigente_de(&c.actualizaciones, c.monto_inicial, hoy);
    let proxima = proxima_fecha_actualizacion_de(&c.actualizaciones, fecha_inicio, c.frecuencia_actualizacion_meses);
    let mut garantes: Vec<GaranteEmbed> = c.contrato_garantes.into_iter().map(|g| g.garantes).collect();
    garantes.sort_by(|a, b| a.nombre.cmp(&b.nombre));
    ContratoDetallado {
        id: c.id,
        inmueble_id: c.inmueble_id,
        inmueble_direccion: c.inmuebles.direccion,
        inquilino_id: c.inquilino_id,
        inquilino_nombre: c.inquilinos.nombre,
        propietario_id: c.inmuebles.propietario_id,
        propietario_nombre: c.inmuebles.propietarios.nombre,
        garantes: garantes.iter().map(|g| g.nombre.clone()).collect(),
        garante_ids: garantes.iter().map(|g| g.id).collect(),
        fecha_inicio: c.fecha_inicio,
        fecha_fin: c.fecha_fin,
        dia_pago: c.dia_pago,
        monto_inicial: c.monto_inicial,
        monto_vigente: monto_vig,
        comision_porcentaje: c.comision_porcentaje,
        tasa_mora_diaria: c.tasa_mora_diaria,
        frecuencia_actualizacion_meses: c.frecuencia_actualizacion_meses,
        tipo_actualizacion: c.tipo_actualizacion,
        porcentaje_actualizacion: c.porcentaje_actualizacion,
        proxima_actualizacion: Some(proxima.format("%Y-%m-%d").to_string()),
        estado: c.estado,
        notas: c.notas,
    }
}

#[tauri::command]
pub async fn get_contratos(state: State<'_, DbState>) -> Result<Vec<ContratoDetallado>, String> {
    let cliente = obtener_cliente(&state).await?;
    let filas: Vec<ContratoEmbed> = cliente
        .select("contratos", &format!("{}&order=fecha_fin", SELECT_CONTRATO_DETALLADO))
        .await
        .map_err(map_err)?;
    let hoy = today();
    Ok(filas.into_iter().map(|c| contrato_embed_a_detallado(c, hoy)).collect())
}

#[tauri::command]
pub async fn guardar_contrato(state: State<'_, DbState>, contrato: Contrato) -> Result<i64, String> {
    let cliente = obtener_cliente(&state).await?;
    let body = json!({
        "inmueble_id": contrato.inmueble_id, "inquilino_id": contrato.inquilino_id,
        "fecha_inicio": contrato.fecha_inicio, "fecha_fin": contrato.fecha_fin, "dia_pago": contrato.dia_pago,
        "monto_inicial": contrato.monto_inicial, "comision_porcentaje": contrato.comision_porcentaje,
        "tasa_mora_diaria": contrato.tasa_mora_diaria, "frecuencia_actualizacion_meses": contrato.frecuencia_actualizacion_meses,
        "tipo_actualizacion": contrato.tipo_actualizacion, "porcentaje_actualizacion": contrato.porcentaje_actualizacion,
        "estado": contrato.estado, "notas": contrato.notas,
    });
    let id = match contrato.id {
        Some(id) => {
            cliente.update("contratos", &format!("id=eq.{}", id), &body).await.map_err(map_err)?;
            cliente.delete("contrato_garantes", &format!("contrato_id=eq.{}", id)).await.map_err(map_err)?;
            id
        }
        None => {
            let fila: FilaId = cliente.insert("contratos", &body).await.map_err(map_err)?;
            fila.id
        }
    };
    for garante_id in contrato.garante_ids {
        cliente
            .insert_ignorando_conflicto("contrato_garantes", &json!({"contrato_id": id, "garante_id": garante_id}), "contrato_id,garante_id")
            .await
            .map_err(map_err)?;
    }
    Ok(id)
}

#[tauri::command]
pub async fn eliminar_contrato(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.delete("contratos", &format!("id=eq.{}", id)).await.map_err(mensaje_borrado_restringido("contrato"))
}

// ---------- Actualizaciones de alquiler ----------

#[tauri::command]
pub async fn get_actualizaciones(state: State<'_, DbState>, contrato_id: i64) -> Result<Vec<Actualizacion>, String> {
    let cliente = obtener_cliente(&state).await?;
    cliente
        .select("actualizaciones", &format!("contrato_id=eq.{}&select=*&order=fecha_vigencia.desc", contrato_id))
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn agregar_actualizacion(state: State<'_, DbState>, actualizacion: Actualizacion) -> Result<i64, String> {
    let cliente = obtener_cliente(&state).await?;
    let body = json!({
        "contrato_id": actualizacion.contrato_id, "fecha_vigencia": actualizacion.fecha_vigencia,
        "monto_nuevo": actualizacion.monto_nuevo, "motivo": actualizacion.motivo,
    });
    let fila: Actualizacion = cliente.insert("actualizaciones", &body).await.map_err(map_err)?;
    Ok(fila.id.unwrap())
}

#[tauri::command]
pub async fn eliminar_actualizacion(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.delete("actualizaciones", &format!("id=eq.{}", id)).await.map_err(map_err)
}

// ---------- Ingresos (pagos de inquilinos / recibos) ----------

#[tauri::command]
pub async fn get_pagos(state: State<'_, DbState>) -> Result<Vec<Pago>, String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.select("pagos", "select=*&order=fecha_pago.desc").await.map_err(map_err)
}

#[derive(Debug, Deserialize)]
struct ContratoParaPago {
    dia_pago: i64,
    tasa_mora_diaria: f64,
    monto_inicial: f64,
}

#[tauri::command]
pub async fn registrar_pago(state: State<'_, DbState>, nuevo: NuevoPago) -> Result<i64, String> {
    let cliente = obtener_cliente(&state).await?;
    let contrato: ContratoParaPago = cliente
        .select_uno("contratos", &format!("id=eq.{}&select=dia_pago,tasa_mora_diaria,monto_inicial", nuevo.contrato_id))
        .await
        .map_err(map_err)?
        .ok_or_else(|| "El contrato no existe".to_string())?;
    let actualizaciones = actualizaciones_de(&cliente, nuevo.contrato_id).await?;

    let fecha_pago = parse_date(&nuevo.fecha_pago);
    let vencimiento = due_date_for_periodo(&nuevo.periodo, contrato.dia_pago);
    let monto_alquiler = monto_vigente_de(&actualizaciones, contrato.monto_inicial, vencimiento);
    let dias_mora = (fecha_pago - vencimiento).num_days().max(0);
    let monto_mora = monto_alquiler * (contrato.tasa_mora_diaria / 100.0) * dias_mora as f64;
    let monto_total = monto_alquiler + monto_mora;

    let numero_recibo = siguiente_numero(&cliente, "pagos", "numero_recibo").await?;

    let body = json!({
        "contrato_id": nuevo.contrato_id, "periodo": nuevo.periodo, "fecha_pago": nuevo.fecha_pago,
        "monto_alquiler": monto_alquiler, "dias_mora": dias_mora, "monto_mora": monto_mora, "monto_total": monto_total,
        "metodo_pago": nuevo.metodo_pago, "numero_recibo": numero_recibo, "notas": nuevo.notas,
    });
    let fila: Pago = cliente.insert("pagos", &body).await.map_err(map_err)?;
    Ok(fila.id)
}

/// Siguiente valor para una columna de numeración secuencial (equivalente a
/// `COALESCE(MAX(columna),0)+1`), para numerar recibos y comprobantes.
async fn siguiente_numero(cliente: &Cliente, tabla: &str, columna: &str) -> Result<i64, String> {
    #[derive(Deserialize)]
    struct Fila {
        #[serde(flatten)]
        valor: std::collections::HashMap<String, i64>,
    }
    let filas: Vec<Fila> = cliente
        .select(tabla, &format!("select={}&order={}.desc&limit=1", columna, columna))
        .await
        .map_err(map_err)?;
    Ok(filas.first().and_then(|f| f.valor.get(columna)).copied().unwrap_or(0) + 1)
}

#[tauri::command]
pub async fn eliminar_pago(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.delete("liquidaciones", &format!("pago_id=eq.{}", id)).await.map_err(map_err)?;
    cliente.delete("pagos", &format!("id=eq.{}", id)).await.map_err(map_err)?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ContratoDePago {
    inquilinos: InquilinoNombre,
    inmuebles: InmuebleDeContratoSimple,
}

#[derive(Debug, Deserialize)]
struct InmuebleDeContratoSimple {
    direccion: String,
    propietarios: PropietarioNombre,
}

#[derive(Debug, Deserialize)]
struct PagoEmbed {
    #[serde(flatten)]
    pago: Pago,
    contratos: ContratoDePago,
}

#[tauri::command]
pub async fn get_recibo(state: State<'_, DbState>, pago_id: i64) -> Result<ReciboCompleto, String> {
    let cliente = obtener_cliente(&state).await?;
    let fila: PagoEmbed = cliente
        .select_uno("pagos", &format!("id=eq.{}&select=*,contratos(inquilinos(nombre),inmuebles(direccion,propietarios(nombre)))", pago_id))
        .await
        .map_err(map_err)?
        .ok_or_else(|| "El pago no existe".to_string())?;
    Ok(ReciboCompleto {
        inquilino_nombre: fila.contratos.inquilinos.nombre,
        inmueble_direccion: fila.contratos.inmuebles.direccion.clone(),
        propietario_nombre: fila.contratos.inmuebles.propietarios.nombre,
        pago: fila.pago,
    })
}

// ---------- Egresos (liquidaciones a propietarios / comisiones) ----------

#[tauri::command]
pub async fn get_liquidaciones(state: State<'_, DbState>) -> Result<Vec<Liquidacion>, String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.select("liquidaciones", "select=*&order=fecha.desc").await.map_err(map_err)
}

#[derive(Debug, Deserialize)]
struct PagoParaLiquidacion {
    contrato_id: i64,
    monto_alquiler: f64,
    monto_total: f64,
}

#[tauri::command]
pub async fn generar_liquidacion(state: State<'_, DbState>, pago_id: i64, comision_porcentaje: Option<f64>, fecha: String, notas: Option<String>) -> Result<i64, String> {
    let cliente = obtener_cliente(&state).await?;
    let pago: PagoParaLiquidacion = cliente
        .select_uno("pagos", &format!("id=eq.{}&select=contrato_id,monto_alquiler,monto_total", pago_id))
        .await
        .map_err(map_err)?
        .ok_or_else(|| "El pago no existe".to_string())?;

    let ya_existe: Option<Value> = cliente.select_uno("liquidaciones", &format!("pago_id=eq.{}&select=id", pago_id)).await.map_err(map_err)?;
    if ya_existe.is_some() {
        return Err("Este pago ya tiene un comprobante de liquidacion generado".to_string());
    }

    let comision_pct = match comision_porcentaje {
        Some(v) => v,
        None => {
            #[derive(Deserialize)]
            struct ComisionContrato {
                comision_porcentaje: f64,
            }
            let c: ComisionContrato = cliente
                .select_uno("contratos", &format!("id=eq.{}&select=comision_porcentaje", pago.contrato_id))
                .await
                .map_err(map_err)?
                .ok_or_else(|| "El contrato no existe".to_string())?;
            c.comision_porcentaje
        }
    };

    let monto_comision = pago.monto_alquiler * (comision_pct / 100.0);
    let monto_neto = pago.monto_total - monto_comision;
    let numero_comprobante = siguiente_numero(&cliente, "liquidaciones", "numero_comprobante").await?;

    let body = json!({
        "pago_id": pago_id, "fecha": fecha, "monto_alquiler": pago.monto_alquiler, "comision_porcentaje": comision_pct,
        "monto_comision": monto_comision, "monto_neto": monto_neto, "numero_comprobante": numero_comprobante, "notas": notas,
    });
    let fila: Liquidacion = cliente.insert("liquidaciones", &body).await.map_err(map_err)?;
    Ok(fila.id)
}

#[tauri::command]
pub async fn eliminar_liquidacion(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.delete("liquidaciones", &format!("id=eq.{}", id)).await.map_err(map_err)
}

#[derive(Debug, Deserialize)]
struct PropietarioNombreYBanco {
    nombre: String,
    datos_bancarios: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InmuebleDeLiquidacion {
    direccion: String,
    propietarios: PropietarioNombreYBanco,
}

#[derive(Debug, Deserialize)]
struct ContratoDeLiquidacion {
    inquilinos: InquilinoNombre,
    inmuebles: InmuebleDeLiquidacion,
}

#[derive(Debug, Deserialize)]
struct PagoDeLiquidacion {
    periodo: String,
    contratos: ContratoDeLiquidacion,
}

#[derive(Debug, Deserialize)]
struct LiquidacionEmbed {
    #[serde(flatten)]
    liquidacion: Liquidacion,
    pagos: PagoDeLiquidacion,
}

#[tauri::command]
pub async fn get_comprobante(state: State<'_, DbState>, liquidacion_id: i64) -> Result<ComprobanteCompleto, String> {
    let cliente = obtener_cliente(&state).await?;
    let fila: LiquidacionEmbed = cliente
        .select_uno(
            "liquidaciones",
            &format!(
                "id=eq.{}&select=*,pagos(periodo,contratos(inquilinos(nombre),inmuebles(direccion,propietarios(nombre,datos_bancarios))))",
                liquidacion_id
            ),
        )
        .await
        .map_err(map_err)?
        .ok_or_else(|| "El comprobante no existe".to_string())?;
    Ok(ComprobanteCompleto {
        periodo: fila.pagos.periodo,
        propietario_nombre: fila.pagos.contratos.inmuebles.propietarios.nombre,
        propietario_datos_bancarios: fila.pagos.contratos.inmuebles.propietarios.datos_bancarios,
        inmueble_direccion: fila.pagos.contratos.inmuebles.direccion,
        inquilino_nombre: fila.pagos.contratos.inquilinos.nombre,
        liquidacion: fila.liquidacion,
    })
}

// ---------- Tablero de control ----------

#[derive(Debug, Deserialize)]
struct PagoPeriodo {
    contrato_id: i64,
    periodo: String,
}

#[derive(Debug, Deserialize)]
struct PersonaCumple {
    nombre: String,
    fecha_nacimiento: Option<String>,
}

#[tauri::command]
pub async fn get_dashboard(state: State<'_, DbState>, dias_vencimiento: i64, dias_actualizacion: i64, dias_cumpleanos: i64) -> Result<ResumenDashboard, String> {
    let cliente = obtener_cliente(&state).await?;
    let hoy = today();

    #[derive(Debug, Deserialize)]
    struct ContratoDashRaw {
        id: i64,
        fecha_inicio: String,
        fecha_fin: String,
        dia_pago: i64,
        monto_inicial: f64,
        tasa_mora_diaria: f64,
        frecuencia_actualizacion_meses: i64,
        tipo_actualizacion: String,
        inmuebles: InmuebleDeContratoSimple,
        inquilinos: InquilinoNombre,
        actualizaciones: Vec<ActualizacionBase>,
    }

    let filas: Vec<ContratoDashRaw> = cliente
        .select(
            "contratos",
            "estado=eq.activo&select=id,fecha_inicio,fecha_fin,dia_pago,monto_inicial,tasa_mora_diaria,frecuencia_actualizacion_meses,tipo_actualizacion,inmuebles(direccion,propietarios(nombre)),inquilinos(nombre),actualizaciones(fecha_vigencia,monto_nuevo)",
        )
        .await
        .map_err(map_err)?;

    let pagos: Vec<PagoPeriodo> = cliente.select("pagos", "select=contrato_id,periodo").await.map_err(map_err)?;
    let periodos_pagados: std::collections::HashSet<(i64, String)> = pagos.into_iter().map(|p| (p.contrato_id, p.periodo)).collect();

    let mut deudas = Vec::new();
    let mut vencimientos = Vec::new();
    let mut actualizaciones_pendientes = Vec::new();
    let mut total_adeudado = 0.0;
    let total_contratos_activos = filas.len() as i64;

    for c in &filas {
        let fecha_inicio = parse_date(&c.fecha_inicio);
        let fecha_fin = parse_date(&c.fecha_fin);

        // --- deudas ---
        let mut periodos_impagos = Vec::new();
        let mut monto_adeudado = 0.0;
        let mut cursor = NaiveDate::from_ymd_opt(fecha_inicio.year(), fecha_inicio.month(), 1).unwrap();
        let limite = NaiveDate::from_ymd_opt(hoy.year(), hoy.month(), 1).unwrap();
        while cursor <= limite {
            let periodo = periodo_de(cursor);
            let vencimiento = due_date_for_periodo(&periodo, c.dia_pago);
            if vencimiento <= hoy && !periodos_pagados.contains(&(c.id, periodo.clone())) {
                let monto = monto_vigente_de(&c.actualizaciones, c.monto_inicial, vencimiento);
                let dias_mora = (hoy - vencimiento).num_days().max(0);
                let mora = monto * (c.tasa_mora_diaria / 100.0) * dias_mora as f64;
                periodos_impagos.push(periodo.clone());
                monto_adeudado += monto + mora;
            }
            cursor = add_months(cursor, 1);
        }
        if !periodos_impagos.is_empty() {
            total_adeudado += monto_adeudado;
            deudas.push(DeudaContrato {
                contrato_id: c.id,
                inmueble_direccion: c.inmuebles.direccion.clone(),
                inquilino_nombre: c.inquilinos.nombre.clone(),
                periodos_impagos,
                monto_adeudado,
            });
        }

        // --- vencimientos de contrato ---
        let dias_restantes = (fecha_fin - hoy).num_days();
        if dias_restantes <= dias_vencimiento {
            vencimientos.push(VencimientoContrato {
                contrato_id: c.id,
                inmueble_direccion: c.inmuebles.direccion.clone(),
                inquilino_nombre: c.inquilinos.nombre.clone(),
                propietario_nombre: c.inmuebles.propietarios.nombre.clone(),
                fecha_fin: fecha_fin.format("%Y-%m-%d").to_string(),
                dias_restantes,
            });
        }

        // --- actualizaciones pendientes ---
        let proxima = proxima_fecha_actualizacion_de(&c.actualizaciones, fecha_inicio, c.frecuencia_actualizacion_meses);
        let dias_restantes_act = (proxima - hoy).num_days();
        if dias_restantes_act <= dias_actualizacion {
            let monto_vig = monto_vigente_de(&c.actualizaciones, c.monto_inicial, hoy);
            actualizaciones_pendientes.push(ActualizacionPendiente {
                contrato_id: c.id,
                inmueble_direccion: c.inmuebles.direccion.clone(),
                inquilino_nombre: c.inquilinos.nombre.clone(),
                tipo_actualizacion: c.tipo_actualizacion.clone(),
                fecha_prevista: proxima.format("%Y-%m-%d").to_string(),
                dias_restantes: dias_restantes_act,
                monto_vigente: monto_vig,
            });
        }
    }

    vencimientos.sort_by_key(|v| v.dias_restantes);
    actualizaciones_pendientes.sort_by_key(|a| a.dias_restantes);

    // --- próximos cumpleaños (propietarios, inquilinos y garantes) ---
    let mut personas: Vec<(String, &str, String)> = Vec::new();
    for (tabla, tipo) in [("propietarios", "Propietario"), ("inquilinos", "Inquilino"), ("garantes", "Garante")] {
        let rows: Vec<PersonaCumple> = cliente
            .select(tabla, "select=nombre,fecha_nacimiento&fecha_nacimiento=not.is.null")
            .await
            .map_err(map_err)?;
        for r in rows {
            if let Some(f) = r.fecha_nacimiento {
                if !f.is_empty() {
                    personas.push((r.nombre, tipo, f));
                }
            }
        }
    }

    let mut cumpleanos_proximos: Vec<CumpleanosProximo> = personas
        .into_iter()
        .map(|(nombre, tipo, fecha_nacimiento)| {
            let nacimiento = parse_date(&fecha_nacimiento);
            let proximo = proxima_fecha_cumpleanos(nacimiento, hoy);
            let dias_restantes = (proximo - hoy).num_days();
            CumpleanosProximo {
                nombre,
                tipo: tipo.to_string(),
                fecha_nacimiento,
                proximo_cumple: proximo.format("%Y-%m-%d").to_string(),
                edad_cumple: (proximo.year() - nacimiento.year()) as i64,
                dias_restantes,
            }
        })
        .filter(|c| c.dias_restantes <= dias_cumpleanos)
        .collect();
    cumpleanos_proximos.sort_by_key(|c| c.dias_restantes);

    Ok(ResumenDashboard { deudas, vencimientos, actualizaciones_pendientes, cumpleanos_proximos, total_contratos_activos, total_adeudado })
}

// ---------- Actualización por índice ICL (BCRA) ----------

/// Consulta la API del BCRA y compara el ICL en `fecha_objetivo` contra el ICL
/// vigente cuando se fijó el monto actual del contrato. `fecha_objetivo` es
/// "hoy" para una estimación aproximada (todavía no se sabe el valor final del
/// día de la actualización) o la fecha real de la próxima actualización para
/// obtener el valor definitivo.
async fn calcular_con_icl(state: &DbState, contrato_id: i64, fecha_objetivo_es_hoy: bool) -> Result<EstimacionIcl, String> {
    #[derive(Deserialize)]
    struct ContratoParaIcl {
        fecha_inicio: String,
        monto_inicial: f64,
        frecuencia_actualizacion_meses: i64,
        tipo_actualizacion: String,
    }

    let cliente = obtener_cliente(state).await?;
    let contrato: ContratoParaIcl = cliente
        .select_uno("contratos", &format!("id=eq.{}&select=fecha_inicio,monto_inicial,frecuencia_actualizacion_meses,tipo_actualizacion", contrato_id))
        .await
        .map_err(map_err)?
        .ok_or_else(|| "El contrato no existe".to_string())?;

    if contrato.tipo_actualizacion != "ICL" {
        return Err("Este contrato no usa el índice ICL como esquema de actualización".to_string());
    }

    let actualizaciones = actualizaciones_de(&cliente, contrato_id).await?;
    let fecha_inicio = parse_date(&contrato.fecha_inicio);
    let fecha_referencia = fecha_base_de(&actualizaciones, fecha_inicio);
    let fecha_objetivo = if fecha_objetivo_es_hoy {
        today()
    } else {
        proxima_fecha_actualizacion_de(&actualizaciones, fecha_inicio, contrato.frecuencia_actualizacion_meses)
    };
    let monto_actual = monto_vigente_de(&actualizaciones, contrato.monto_inicial, today());

    let client = icl::cliente_http()?;
    let id_variable = match icl::id_variable_icl(&client).await {
        Ok(id) => id,
        Err(_) => icl::ICL_ID_FALLBACK,
    };
    let dato_referencia = icl::valor_en_o_antes(&client, id_variable, fecha_referencia).await?;
    let dato_objetivo = icl::valor_en_o_antes(&client, id_variable, fecha_objetivo).await?;
    let ratio = dato_objetivo.valor / dato_referencia.valor;

    Ok(EstimacionIcl {
        es_valor_real: !fecha_objetivo_es_hoy,
        fecha_referencia: dato_referencia.fecha,
        valor_icl_referencia: dato_referencia.valor,
        fecha_consulta: dato_objetivo.fecha,
        valor_icl_consulta: dato_objetivo.valor,
        porcentaje_variacion: (ratio - 1.0) * 100.0,
        monto_actual,
        monto_estimado: monto_actual * ratio,
    })
}

/// Para usar un mes antes de la actualización: compara el ICL de hoy contra
/// el ICL de referencia, como aproximación de a cuánto va a quedar el alquiler.
#[tauri::command]
pub async fn estimar_actualizacion_icl(state: State<'_, DbState>, contrato_id: i64) -> Result<EstimacionIcl, String> {
    calcular_con_icl(&state, contrato_id, true).await
}

/// Para usar el día de la actualización (o después): trae el valor real del
/// ICL para esa fecha exacta.
#[tauri::command]
pub async fn confirmar_actualizacion_icl(state: State<'_, DbState>, contrato_id: i64) -> Result<EstimacionIcl, String> {
    calcular_con_icl(&state, contrato_id, false).await
}

// ---------- Usuarios / login ----------

/// true si todavia no se creo ningun usuario (primer arranque de la app).
#[tauri::command]
pub async fn hay_usuarios(state: State<'_, DbState>) -> Result<bool, String> {
    let cliente = obtener_cliente(&state).await?;
    let filas: Vec<Value> = cliente.select("usuarios", "select=id&limit=1").await.map_err(map_err)?;
    Ok(!filas.is_empty())
}

#[tauri::command]
pub async fn get_usuarios(state: State<'_, DbState>) -> Result<Vec<Usuario>, String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.select("usuarios", "select=id,username,nombre_completo,activo&order=nombre_completo").await.map_err(map_err)
}

/// Crea un usuario nuevo. Cualquiera puede crear el primero (arranque de la
/// app, sin login todavia); a partir de ahi la pantalla de "nuevo usuario"
/// del frontend solo se muestra estando ya logueado.
#[tauri::command]
pub async fn crear_usuario(state: State<'_, DbState>, nuevo: NuevoUsuario) -> Result<Usuario, String> {
    if nuevo.username.trim().is_empty() || nuevo.password.len() < 4 {
        return Err("El usuario no puede estar vacío y la contraseña debe tener al menos 4 caracteres".to_string());
    }
    let hash = bcrypt::hash(&nuevo.password, bcrypt::DEFAULT_COST).map_err(map_err)?;
    let cliente = obtener_cliente(&state).await?;
    let body = json!({
        "username": nuevo.username.trim(), "password_hash": hash, "nombre_completo": nuevo.nombre_completo.trim(), "activo": true,
    });
    cliente.insert("usuarios", &body).await.map_err(|e: ErrorSupabase| {
        if e.es_codigo(UNIQUE_VIOLATION) {
            "Ya existe un usuario con ese nombre de usuario".to_string()
        } else {
            e.to_string()
        }
    })
}

#[tauri::command]
pub async fn eliminar_usuario(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let cliente = obtener_cliente(&state).await?;
    cliente.delete("usuarios", &format!("id=eq.{}", id)).await.map_err(map_err)
}

#[derive(Debug, Deserialize)]
struct UsuarioConHash {
    id: i64,
    username: String,
    password_hash: String,
    nombre_completo: String,
    activo: bool,
}

#[tauri::command]
pub async fn iniciar_sesion(state: State<'_, DbState>, username: String, password: String) -> Result<Usuario, String> {
    let cliente = obtener_cliente(&state).await?;
    let fila: UsuarioConHash = cliente
        .select_uno("usuarios", &format!("username=eq.{}&select=id,username,password_hash,nombre_completo,activo", urlencoding(username.trim())))
        .await
        .map_err(map_err)?
        .ok_or_else(|| "Usuario o contraseña incorrectos".to_string())?;

    if !fila.activo {
        return Err("Este usuario está deshabilitado".to_string());
    }

    let valido = bcrypt::verify(&password, &fila.password_hash).map_err(map_err)?;
    if !valido {
        return Err("Usuario o contraseña incorrectos".to_string());
    }

    Ok(Usuario { id: fila.id, username: fila.username, nombre_completo: fila.nombre_completo, activo: true })
}

/// Mensaje legible cuando un borrado choca contra una clave foránea de otra
/// tabla que todavía referencia la fila (DELETE RESTRICT).
fn mensaje_borrado_restringido(entidad: &'static str) -> impl Fn(ErrorSupabase) -> String {
    move |e| {
        if e.es_codigo(FOREIGN_KEY_VIOLATION) {
            format!("No se puede eliminar: hay otros registros que todavía dependen de este {}.", entidad)
        } else {
            e.to_string()
        }
    }
}

fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Test de integración contra un PostgREST real (y Postgres debajo): valida
/// que el cliente REST y la lógica de negocio de cada comando (RETURNING,
/// ON CONFLICT vía ignore-duplicates, UNIQUE, joins anidados, el conteo
/// dinámico de cumpleaños, DELETE RESTRICT) funcionan tal cual contra la
/// API con la que habla la app, no solo que compilan. Se salta solo si
/// TEST_SUPABASE_URL / TEST_SUPABASE_ANON_KEY no están definidas.
#[cfg(test)]
mod tests_rest {
    use super::*;

    async fn conectar_test() -> Option<Cliente> {
        let url = std::env::var("TEST_SUPABASE_URL").ok()?;
        let anon_key = std::env::var("TEST_SUPABASE_ANON_KEY").ok()?;
        Some(Cliente::con_base_completa(url, anon_key))
    }

    async fn limpiar_todo(cliente: &Cliente) {
        for tabla in ["liquidaciones", "pagos", "contrato_garantes", "actualizaciones", "contratos", "inmuebles", "garantes", "inquilinos", "propietarios", "usuarios"] {
            let _ = cliente.delete(tabla, "id=gte.0").await;
        }
    }

    #[tokio::test]
    async fn flujo_completo_contra_postgrest() {
        let Some(cliente) = conectar_test().await else {
            eprintln!("TEST_SUPABASE_URL / TEST_SUPABASE_ANON_KEY no están definidas: se omite el test de integración con PostgREST");
            return;
        };
        limpiar_todo(&cliente).await;

        // --- usuarios: alta, UNIQUE violation, bcrypt ---
        let hash = bcrypt::hash("claveSegura1", bcrypt::DEFAULT_COST).unwrap();
        let usuario: Usuario = cliente
            .insert("usuarios", &json!({"username":"agustin","password_hash":hash,"nombre_completo":"Agustín","activo":true}))
            .await
            .unwrap();
        assert_eq!(usuario.username, "agustin");
        assert!(usuario.activo);

        let duplicado = cliente
            .insert::<Usuario>("usuarios", &json!({"username":"agustin","password_hash":hash,"nombre_completo":"Otro","activo":true}))
            .await;
        assert!(duplicado.unwrap_err().es_codigo(UNIQUE_VIOLATION));

        let fila_login: UsuarioConHash = cliente
            .select_uno("usuarios", "username=eq.agustin&select=id,username,password_hash,nombre_completo,activo")
            .await
            .unwrap()
            .unwrap();
        assert!(bcrypt::verify("claveSegura1", &fila_login.password_hash).unwrap());
        assert!(!bcrypt::verify("claveIncorrecta", &fila_login.password_hash).unwrap());

        // --- propietarios: insert con RETURNING, update ---
        let propietario: Propietario = cliente
            .insert(
                "propietarios",
                &json!({"nombre":"Monchola","dni_cuit":"20111222","fecha_nacimiento":"1980-05-20","datos_bancarios":"CBU 123"}),
            )
            .await
            .unwrap();
        let propietario_id = propietario.id.unwrap();
        cliente.update("propietarios", &format!("id=eq.{}", propietario_id), &json!({"telefono": "1199998888"})).await.unwrap();
        let actualizado: Propietario = cliente.select_uno("propietarios", &format!("id=eq.{}&select=*", propietario_id)).await.unwrap().unwrap();
        assert_eq!(actualizado.telefono, Some("1199998888".to_string()));

        // --- inmueble con FK a propietario ---
        let inmueble: Inmueble = cliente
            .insert("inmuebles", &json!({"propietario_id": propietario_id, "direccion": "Mitre 586", "tipo": "Departamento", "superficie": 45.0, "ambientes": 2}))
            .await
            .unwrap();
        let inmueble_id = inmueble.id.unwrap();

        // --- inquilino y garante ---
        let inquilino: Inquilino = cliente.insert("inquilinos", &json!({"nombre": "Juan Pérez"})).await.unwrap();
        let inquilino_id = inquilino.id.unwrap();
        let garante: Garante = cliente.insert("garantes", &json!({"nombre": "María Gómez"})).await.unwrap();
        let garante_id = garante.id.unwrap();

        // --- contrato con joins anidados (contrato_embed_a_detallado) ---
        let contrato: FilaId = cliente
            .insert(
                "contratos",
                &json!({
                    "inmueble_id": inmueble_id, "inquilino_id": inquilino_id, "fecha_inicio": "2026-01-01", "fecha_fin": "2028-01-01",
                    "dia_pago": 10, "monto_inicial": 400000.0, "comision_porcentaje": 5.0, "tasa_mora_diaria": 0.1,
                    "frecuencia_actualizacion_meses": 3, "tipo_actualizacion": "ICL", "porcentaje_actualizacion": 0.0, "estado": "activo",
                }),
            )
            .await
            .unwrap();
        let contrato_id = contrato.id;

        // ignore-duplicates (equivalente a ON CONFLICT DO NOTHING): insertar el mismo par dos veces no duplica
        for _ in 0..2 {
            cliente
                .insert_ignorando_conflicto("contrato_garantes", &json!({"contrato_id": contrato_id, "garante_id": garante_id}), "contrato_id,garante_id")
                .await
                .unwrap();
        }
        let filas_garantes: Vec<Value> = cliente.select("contrato_garantes", &format!("contrato_id=eq.{}&select=garante_id", contrato_id)).await.unwrap();
        assert_eq!(filas_garantes.len(), 1);

        let filas: Vec<ContratoEmbed> = cliente.select("contratos", &format!("id=eq.{}&{}", contrato_id, SELECT_CONTRATO_DETALLADO)).await.unwrap();
        let detallado = contrato_embed_a_detallado(filas.into_iter().next().unwrap(), today());
        assert_eq!(detallado.inmueble_direccion, "Mitre 586");
        assert_eq!(detallado.inquilino_nombre, "Juan Pérez");
        assert_eq!(detallado.propietario_nombre, "Monchola");
        assert_eq!(detallado.garantes, vec!["María Gómez".to_string()]);
        assert_eq!(detallado.monto_vigente, 400000.0);

        // --- actualizacion de alquiler ---
        cliente
            .insert::<Actualizacion>(
                "actualizaciones",
                &json!({"contrato_id": contrato_id, "fecha_vigencia": "2026-04-01", "monto_nuevo": 440000.0, "motivo": "Actualización ICL"}),
            )
            .await
            .unwrap();
        let actualizaciones = actualizaciones_de(&cliente, contrato_id).await.unwrap();
        assert_eq!(monto_vigente_de(&actualizaciones, 400000.0, parse_date("2026-05-01")), 440000.0);
        assert_eq!(monto_vigente_de(&actualizaciones, 400000.0, parse_date("2026-02-01")), 400000.0);

        // --- pago con numeración secuencial ---
        let numero_recibo = siguiente_numero(&cliente, "pagos", "numero_recibo").await.unwrap();
        assert_eq!(numero_recibo, 1);
        let pago: Pago = cliente
            .insert(
                "pagos",
                &json!({
                    "contrato_id": contrato_id, "periodo": "2026-05", "fecha_pago": "2026-05-10", "monto_alquiler": 440000.0,
                    "dias_mora": 0, "monto_mora": 0.0, "monto_total": 440000.0, "metodo_pago": "Transferencia", "numero_recibo": numero_recibo,
                }),
            )
            .await
            .unwrap();

        // --- liquidacion ---
        let numero_comprobante = siguiente_numero(&cliente, "liquidaciones", "numero_comprobante").await.unwrap();
        assert_eq!(numero_comprobante, 1);
        cliente
            .insert::<Liquidacion>(
                "liquidaciones",
                &json!({
                    "pago_id": pago.id, "fecha": "2026-05-10", "monto_alquiler": 440000.0, "comision_porcentaje": 5.0,
                    "monto_comision": 22000.0, "monto_neto": 418000.0, "numero_comprobante": numero_comprobante,
                }),
            )
            .await
            .unwrap();

        // --- consulta dinámica de cumpleaños (mismo recorrido que usa get_dashboard) ---
        for tabla in ["propietarios", "inquilinos", "garantes"] {
            let filas: Vec<PersonaCumple> = cliente.select(tabla, "select=nombre,fecha_nacimiento&fecha_nacimiento=not.is.null").await.unwrap();
            if tabla == "propietarios" {
                assert_eq!(filas.len(), 1);
                assert_eq!(filas[0].nombre, "Monchola");
            }
        }

        // --- el DELETE RESTRICT de propietarios con inmuebles debe fallar con el código esperado ---
        let borrado_restringido = cliente.delete("propietarios", &format!("id=eq.{}", propietario_id)).await;
        assert!(borrado_restringido.unwrap_err().es_codigo(FOREIGN_KEY_VIOLATION));
    }
}
