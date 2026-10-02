use crate::icl;
use crate::models::*;
use chrono::{Datelike, NaiveDate};
use deadpool_postgres::{Object, Pool};
use tauri::{Manager, State};
use tokio::sync::RwLock;
use tokio_postgres::error::SqlState;

pub struct DbState(pub RwLock<Option<Pool>>);

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

async fn obtener_conn(state: &DbState) -> Result<Object, String> {
    let guard = state.0.read().await;
    let pool = guard
        .as_ref()
        .ok_or_else(|| "La aplicación todavía no está conectada a la base de datos. Configurá la conexión primero.".to_string())?;
    pool.get().await.map_err(|e| format!("No se pudo obtener una conexión a la base de datos: {}", e))
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

/// Monto de alquiler vigente a una fecha dada, segun la ultima actualizacion registrada.
async fn monto_vigente(conn: &Object, contrato_id: i64, fecha: NaiveDate) -> Result<f64, String> {
    let monto_inicial: f64 = conn
        .query_one("SELECT monto_inicial FROM contratos WHERE id = $1", &[&contrato_id])
        .await
        .map_err(map_err)?
        .get(0);
    let fecha_str = fecha.format("%Y-%m-%d").to_string();
    let actualizado = conn
        .query_opt(
            "SELECT monto_nuevo FROM actualizaciones WHERE contrato_id = $1 AND fecha_vigencia <= $2 ORDER BY fecha_vigencia DESC LIMIT 1",
            &[&contrato_id, &fecha_str],
        )
        .await
        .map_err(map_err)?
        .map(|r| r.get::<_, f64>(0));
    Ok(actualizado.unwrap_or(monto_inicial))
}

/// Fecha de vigencia de la ultima actualizacion registrada, o la fecha de
/// inicio del contrato si todavia no tuvo ninguna. Es el punto de referencia
/// contra el que se mide la variacion del indice para la proxima actualizacion.
async fn fecha_base_actualizacion(conn: &Object, contrato_id: i64, fecha_inicio: NaiveDate) -> Result<NaiveDate, String> {
    let ultima = conn
        .query_opt(
            "SELECT fecha_vigencia FROM actualizaciones WHERE contrato_id = $1 ORDER BY fecha_vigencia DESC LIMIT 1",
            &[&contrato_id],
        )
        .await
        .map_err(map_err)?
        .map(|r| r.get::<_, String>(0));
    Ok(match ultima {
        Some(f) => parse_date(&f),
        None => fecha_inicio,
    })
}

async fn proxima_fecha_actualizacion(conn: &Object, contrato_id: i64, fecha_inicio: NaiveDate, frecuencia_meses: i64) -> Result<NaiveDate, String> {
    let base = fecha_base_actualizacion(conn, contrato_id, fecha_inicio).await?;
    Ok(add_months(base, frecuencia_meses.max(1)))
}

// ---------- Conexión a la base de datos ----------

/// Connection string de fábrica, incluido en el instalador en tiempo de
/// compilación (variable de entorno INMOBILIARIA_DB_URL en el build de
/// GitHub Actions). Permite que la app venga lista para usar sin que la
/// persona que la instala tenga que pegar ningún dato técnico. Si no está
/// presente (build local de desarrollo), la app pide el connection string
/// a mano la primera vez, como antes.
const DB_URL_DE_FABRICA: Option<&str> = option_env!("INMOBILIARIA_DB_URL");

/// true si ya hay una conexión activa, si existe una guardada de una sesión
/// anterior y se pudo restablecer, o si se pudo conectar con el connection
/// string de fábrica incluido en el instalador. false solo si hace falta
/// pedirlo a mano (primera vez en un build sin connection string de fábrica).
#[tauri::command]
pub async fn hay_configuracion_conexion(app: tauri::AppHandle, state: State<'_, DbState>) -> Result<bool, String> {
    if state.0.read().await.is_some() {
        return Ok(true);
    }
    let data_dir = app.path().app_data_dir().map_err(map_err)?;
    if let Some(cs) = crate::db::leer_connection_string_guardado(&data_dir) {
        let pool = crate::db::conectar(&cs).await?;
        *state.0.write().await = Some(pool);
        return Ok(true);
    }
    if let Some(cs) = DB_URL_DE_FABRICA {
        let pool = crate::db::conectar(cs).await?;
        std::fs::create_dir_all(&data_dir).map_err(map_err)?;
        crate::db::guardar_connection_string(&data_dir, cs)?;
        *state.0.write().await = Some(pool);
        return Ok(true);
    }
    Ok(false)
}

#[tauri::command]
pub async fn configurar_conexion(app: tauri::AppHandle, state: State<'_, DbState>, connection_string: String) -> Result<(), String> {
    let pool = crate::db::conectar(connection_string.trim()).await?;
    let data_dir = app.path().app_data_dir().map_err(map_err)?;
    std::fs::create_dir_all(&data_dir).map_err(map_err)?;
    crate::db::guardar_connection_string(&data_dir, connection_string.trim())?;
    *state.0.write().await = Some(pool);
    Ok(())
}

// ---------- Propietarios ----------

#[tauri::command]
pub async fn get_propietarios(state: State<'_, DbState>) -> Result<Vec<Propietario>, String> {
    let conn = obtener_conn(&state).await?;
    let rows = conn
        .query(
            "SELECT id, nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, datos_bancarios, notas FROM propietarios ORDER BY nombre",
            &[],
        )
        .await
        .map_err(map_err)?;
    Ok(rows
        .iter()
        .map(|r| Propietario {
            id: Some(r.get(0)),
            nombre: r.get(1),
            dni_cuit: r.get(2),
            fecha_nacimiento: r.get(3),
            telefono: r.get(4),
            email: r.get(5),
            direccion: r.get(6),
            datos_bancarios: r.get(7),
            notas: r.get(8),
        })
        .collect())
}

#[tauri::command]
pub async fn guardar_propietario(state: State<'_, DbState>, propietario: Propietario) -> Result<i64, String> {
    let conn = obtener_conn(&state).await?;
    match propietario.id {
        Some(id) => {
            conn.execute(
                "UPDATE propietarios SET nombre=$1, dni_cuit=$2, fecha_nacimiento=$3, telefono=$4, email=$5, direccion=$6, datos_bancarios=$7, notas=$8 WHERE id=$9",
                &[&propietario.nombre, &propietario.dni_cuit, &propietario.fecha_nacimiento, &propietario.telefono, &propietario.email, &propietario.direccion, &propietario.datos_bancarios, &propietario.notas, &id],
            ).await.map_err(map_err)?;
            Ok(id)
        }
        None => {
            let fila = conn.query_one(
                "INSERT INTO propietarios (nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, datos_bancarios, notas) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id",
                &[&propietario.nombre, &propietario.dni_cuit, &propietario.fecha_nacimiento, &propietario.telefono, &propietario.email, &propietario.direccion, &propietario.datos_bancarios, &propietario.notas],
            ).await.map_err(map_err)?;
            Ok(fila.get(0))
        }
    }
}

#[tauri::command]
pub async fn eliminar_propietario(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let conn = obtener_conn(&state).await?;
    conn.execute("DELETE FROM propietarios WHERE id=$1", &[&id]).await.map_err(map_err)?;
    Ok(())
}

// ---------- Inquilinos ----------

#[tauri::command]
pub async fn get_inquilinos(state: State<'_, DbState>) -> Result<Vec<Inquilino>, String> {
    let conn = obtener_conn(&state).await?;
    let rows = conn
        .query("SELECT id, nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, notas FROM inquilinos ORDER BY nombre", &[])
        .await
        .map_err(map_err)?;
    Ok(rows
        .iter()
        .map(|r| Inquilino {
            id: Some(r.get(0)),
            nombre: r.get(1),
            dni_cuit: r.get(2),
            fecha_nacimiento: r.get(3),
            telefono: r.get(4),
            email: r.get(5),
            direccion: r.get(6),
            notas: r.get(7),
        })
        .collect())
}

#[tauri::command]
pub async fn guardar_inquilino(state: State<'_, DbState>, inquilino: Inquilino) -> Result<i64, String> {
    let conn = obtener_conn(&state).await?;
    match inquilino.id {
        Some(id) => {
            conn.execute(
                "UPDATE inquilinos SET nombre=$1, dni_cuit=$2, fecha_nacimiento=$3, telefono=$4, email=$5, direccion=$6, notas=$7 WHERE id=$8",
                &[&inquilino.nombre, &inquilino.dni_cuit, &inquilino.fecha_nacimiento, &inquilino.telefono, &inquilino.email, &inquilino.direccion, &inquilino.notas, &id],
            ).await.map_err(map_err)?;
            Ok(id)
        }
        None => {
            let fila = conn.query_one(
                "INSERT INTO inquilinos (nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, notas) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id",
                &[&inquilino.nombre, &inquilino.dni_cuit, &inquilino.fecha_nacimiento, &inquilino.telefono, &inquilino.email, &inquilino.direccion, &inquilino.notas],
            ).await.map_err(map_err)?;
            Ok(fila.get(0))
        }
    }
}

#[tauri::command]
pub async fn eliminar_inquilino(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let conn = obtener_conn(&state).await?;
    conn.execute("DELETE FROM inquilinos WHERE id=$1", &[&id]).await.map_err(map_err)?;
    Ok(())
}

// ---------- Garantes ----------

#[tauri::command]
pub async fn get_garantes(state: State<'_, DbState>) -> Result<Vec<Garante>, String> {
    let conn = obtener_conn(&state).await?;
    let rows = conn
        .query("SELECT id, nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, notas FROM garantes ORDER BY nombre", &[])
        .await
        .map_err(map_err)?;
    Ok(rows
        .iter()
        .map(|r| Garante {
            id: Some(r.get(0)),
            nombre: r.get(1),
            dni_cuit: r.get(2),
            fecha_nacimiento: r.get(3),
            telefono: r.get(4),
            email: r.get(5),
            direccion: r.get(6),
            notas: r.get(7),
        })
        .collect())
}

#[tauri::command]
pub async fn guardar_garante(state: State<'_, DbState>, garante: Garante) -> Result<i64, String> {
    let conn = obtener_conn(&state).await?;
    match garante.id {
        Some(id) => {
            conn.execute(
                "UPDATE garantes SET nombre=$1, dni_cuit=$2, fecha_nacimiento=$3, telefono=$4, email=$5, direccion=$6, notas=$7 WHERE id=$8",
                &[&garante.nombre, &garante.dni_cuit, &garante.fecha_nacimiento, &garante.telefono, &garante.email, &garante.direccion, &garante.notas, &id],
            ).await.map_err(map_err)?;
            Ok(id)
        }
        None => {
            let fila = conn.query_one(
                "INSERT INTO garantes (nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, notas) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id",
                &[&garante.nombre, &garante.dni_cuit, &garante.fecha_nacimiento, &garante.telefono, &garante.email, &garante.direccion, &garante.notas],
            ).await.map_err(map_err)?;
            Ok(fila.get(0))
        }
    }
}

#[tauri::command]
pub async fn eliminar_garante(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let conn = obtener_conn(&state).await?;
    conn.execute("DELETE FROM garantes WHERE id=$1", &[&id]).await.map_err(map_err)?;
    Ok(())
}

// ---------- Inmuebles ----------

#[tauri::command]
pub async fn get_inmuebles(state: State<'_, DbState>) -> Result<Vec<InmuebleDetallado>, String> {
    let conn = obtener_conn(&state).await?;
    let rows = conn
        .query(
            "SELECT i.id, i.propietario_id, p.nombre, i.direccion, i.tipo, i.superficie, i.ambientes, i.notas
             FROM inmuebles i JOIN propietarios p ON p.id = i.propietario_id
             ORDER BY i.direccion",
            &[],
        )
        .await
        .map_err(map_err)?;
    Ok(rows
        .iter()
        .map(|r| InmuebleDetallado {
            id: r.get(0),
            propietario_id: r.get(1),
            propietario_nombre: r.get(2),
            direccion: r.get(3),
            tipo: r.get(4),
            superficie: r.get(5),
            ambientes: r.get(6),
            notas: r.get(7),
        })
        .collect())
}

#[tauri::command]
pub async fn guardar_inmueble(state: State<'_, DbState>, inmueble: Inmueble) -> Result<i64, String> {
    let conn = obtener_conn(&state).await?;
    match inmueble.id {
        Some(id) => {
            conn.execute(
                "UPDATE inmuebles SET propietario_id=$1, direccion=$2, tipo=$3, superficie=$4, ambientes=$5, notas=$6 WHERE id=$7",
                &[&inmueble.propietario_id, &inmueble.direccion, &inmueble.tipo, &inmueble.superficie, &inmueble.ambientes, &inmueble.notas, &id],
            ).await.map_err(map_err)?;
            Ok(id)
        }
        None => {
            let fila = conn.query_one(
                "INSERT INTO inmuebles (propietario_id, direccion, tipo, superficie, ambientes, notas) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
                &[&inmueble.propietario_id, &inmueble.direccion, &inmueble.tipo, &inmueble.superficie, &inmueble.ambientes, &inmueble.notas],
            ).await.map_err(map_err)?;
            Ok(fila.get(0))
        }
    }
}

#[tauri::command]
pub async fn eliminar_inmueble(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let conn = obtener_conn(&state).await?;
    conn.execute("DELETE FROM inmuebles WHERE id=$1", &[&id]).await.map_err(map_err)?;
    Ok(())
}

// ---------- Contratos ----------

async fn cargar_garantes_de(conn: &Object, contrato_id: i64) -> Result<Vec<String>, String> {
    let rows = conn
        .query(
            "SELECT g.nombre FROM contrato_garantes cg JOIN garantes g ON g.id = cg.garante_id WHERE cg.contrato_id = $1 ORDER BY g.nombre",
            &[&contrato_id],
        )
        .await
        .map_err(map_err)?;
    Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
}

async fn cargar_garante_ids_de(conn: &Object, contrato_id: i64) -> Result<Vec<i64>, String> {
    let rows = conn
        .query("SELECT garante_id FROM contrato_garantes WHERE contrato_id = $1", &[&contrato_id])
        .await
        .map_err(map_err)?;
    Ok(rows.iter().map(|r| r.get::<_, i64>(0)).collect())
}

async fn construir_contrato_detallado(conn: &Object, id: i64) -> Result<ContratoDetallado, String> {
    let r = conn
        .query_one(
            "SELECT c.id, c.inmueble_id, im.direccion, c.inquilino_id, iq.nombre, im.propietario_id, p.nombre,
                    c.fecha_inicio, c.fecha_fin, c.dia_pago, c.monto_inicial, c.comision_porcentaje, c.tasa_mora_diaria,
                    c.frecuencia_actualizacion_meses, c.tipo_actualizacion, c.porcentaje_actualizacion, c.estado, c.notas
             FROM contratos c
             JOIN inmuebles im ON im.id = c.inmueble_id
             JOIN propietarios p ON p.id = im.propietario_id
             JOIN inquilinos iq ON iq.id = c.inquilino_id
             WHERE c.id = $1",
            &[&id],
        )
        .await
        .map_err(map_err)?;

    let cid: i64 = r.get(0);
    let inmueble_id: i64 = r.get(1);
    let inmueble_direccion: String = r.get(2);
    let inquilino_id: i64 = r.get(3);
    let inquilino_nombre: String = r.get(4);
    let propietario_id: i64 = r.get(5);
    let propietario_nombre: String = r.get(6);
    let fecha_inicio: String = r.get(7);
    let fecha_fin: String = r.get(8);
    let dia_pago: i64 = r.get(9);
    let monto_inicial: f64 = r.get(10);
    let comision_porcentaje: f64 = r.get(11);
    let tasa_mora_diaria: f64 = r.get(12);
    let frecuencia_actualizacion_meses: i64 = r.get(13);
    let tipo_actualizacion: String = r.get(14);
    let porcentaje_actualizacion: f64 = r.get(15);
    let estado: String = r.get(16);
    let notas: Option<String> = r.get(17);

    let garantes = cargar_garantes_de(conn, cid).await?;
    let garante_ids = cargar_garante_ids_de(conn, cid).await?;
    let monto_vig = monto_vigente(conn, cid, today()).await?;
    let proxima = proxima_fecha_actualizacion(conn, cid, parse_date(&fecha_inicio), frecuencia_actualizacion_meses).await?;

    Ok(ContratoDetallado {
        id: cid,
        inmueble_id,
        inmueble_direccion,
        inquilino_id,
        inquilino_nombre,
        propietario_id,
        propietario_nombre,
        garantes,
        garante_ids,
        fecha_inicio,
        fecha_fin,
        dia_pago,
        monto_inicial,
        monto_vigente: monto_vig,
        comision_porcentaje,
        tasa_mora_diaria,
        frecuencia_actualizacion_meses,
        tipo_actualizacion,
        porcentaje_actualizacion,
        proxima_actualizacion: Some(proxima.format("%Y-%m-%d").to_string()),
        estado,
        notas,
    })
}

#[tauri::command]
pub async fn get_contratos(state: State<'_, DbState>) -> Result<Vec<ContratoDetallado>, String> {
    let conn = obtener_conn(&state).await?;
    let rows = conn.query("SELECT id FROM contratos ORDER BY fecha_fin", &[]).await.map_err(map_err)?;
    let ids: Vec<i64> = rows.iter().map(|r| r.get(0)).collect();
    let mut resultado = Vec::with_capacity(ids.len());
    for id in ids {
        resultado.push(construir_contrato_detallado(&conn, id).await?);
    }
    Ok(resultado)
}

#[tauri::command]
pub async fn guardar_contrato(state: State<'_, DbState>, contrato: Contrato) -> Result<i64, String> {
    let conn = obtener_conn(&state).await?;
    let id = match contrato.id {
        Some(id) => {
            conn.execute(
                "UPDATE contratos SET inmueble_id=$1, inquilino_id=$2, fecha_inicio=$3, fecha_fin=$4, dia_pago=$5,
                 monto_inicial=$6, comision_porcentaje=$7, tasa_mora_diaria=$8, frecuencia_actualizacion_meses=$9,
                 tipo_actualizacion=$10, porcentaje_actualizacion=$11, estado=$12, notas=$13 WHERE id=$14",
                &[&contrato.inmueble_id, &contrato.inquilino_id, &contrato.fecha_inicio, &contrato.fecha_fin,
                    &contrato.dia_pago, &contrato.monto_inicial, &contrato.comision_porcentaje, &contrato.tasa_mora_diaria,
                    &contrato.frecuencia_actualizacion_meses, &contrato.tipo_actualizacion, &contrato.porcentaje_actualizacion,
                    &contrato.estado, &contrato.notas, &id],
            ).await.map_err(map_err)?;
            conn.execute("DELETE FROM contrato_garantes WHERE contrato_id=$1", &[&id]).await.map_err(map_err)?;
            id
        }
        None => {
            let fila = conn.query_one(
                "INSERT INTO contratos (inmueble_id, inquilino_id, fecha_inicio, fecha_fin, dia_pago, monto_inicial,
                 comision_porcentaje, tasa_mora_diaria, frecuencia_actualizacion_meses, tipo_actualizacion,
                 porcentaje_actualizacion, estado, notas) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) RETURNING id",
                &[&contrato.inmueble_id, &contrato.inquilino_id, &contrato.fecha_inicio, &contrato.fecha_fin,
                    &contrato.dia_pago, &contrato.monto_inicial, &contrato.comision_porcentaje, &contrato.tasa_mora_diaria,
                    &contrato.frecuencia_actualizacion_meses, &contrato.tipo_actualizacion, &contrato.porcentaje_actualizacion,
                    &contrato.estado, &contrato.notas],
            ).await.map_err(map_err)?;
            fila.get(0)
        }
    };
    for garante_id in contrato.garante_ids {
        conn.execute(
            "INSERT INTO contrato_garantes (contrato_id, garante_id) VALUES ($1,$2) ON CONFLICT DO NOTHING",
            &[&id, &garante_id],
        ).await.map_err(map_err)?;
    }
    Ok(id)
}

#[tauri::command]
pub async fn eliminar_contrato(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let conn = obtener_conn(&state).await?;
    conn.execute("DELETE FROM contratos WHERE id=$1", &[&id]).await.map_err(map_err)?;
    Ok(())
}

// ---------- Actualizaciones de alquiler ----------

#[tauri::command]
pub async fn get_actualizaciones(state: State<'_, DbState>, contrato_id: i64) -> Result<Vec<Actualizacion>, String> {
    let conn = obtener_conn(&state).await?;
    let rows = conn
        .query(
            "SELECT id, contrato_id, fecha_vigencia, monto_nuevo, motivo FROM actualizaciones WHERE contrato_id=$1 ORDER BY fecha_vigencia DESC",
            &[&contrato_id],
        )
        .await
        .map_err(map_err)?;
    Ok(rows
        .iter()
        .map(|r| Actualizacion {
            id: Some(r.get(0)),
            contrato_id: r.get(1),
            fecha_vigencia: r.get(2),
            monto_nuevo: r.get(3),
            motivo: r.get(4),
        })
        .collect())
}

#[tauri::command]
pub async fn agregar_actualizacion(state: State<'_, DbState>, actualizacion: Actualizacion) -> Result<i64, String> {
    let conn = obtener_conn(&state).await?;
    let fila = conn.query_one(
        "INSERT INTO actualizaciones (contrato_id, fecha_vigencia, monto_nuevo, motivo) VALUES ($1,$2,$3,$4) RETURNING id",
        &[&actualizacion.contrato_id, &actualizacion.fecha_vigencia, &actualizacion.monto_nuevo, &actualizacion.motivo],
    ).await.map_err(map_err)?;
    Ok(fila.get(0))
}

#[tauri::command]
pub async fn eliminar_actualizacion(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let conn = obtener_conn(&state).await?;
    conn.execute("DELETE FROM actualizaciones WHERE id=$1", &[&id]).await.map_err(map_err)?;
    Ok(())
}

// ---------- Ingresos (pagos de inquilinos / recibos) ----------

#[tauri::command]
pub async fn get_pagos(state: State<'_, DbState>) -> Result<Vec<Pago>, String> {
    let conn = obtener_conn(&state).await?;
    let rows = conn
        .query(
            "SELECT id, contrato_id, periodo, fecha_pago, monto_alquiler, dias_mora, monto_mora, monto_total, metodo_pago, numero_recibo, notas FROM pagos ORDER BY fecha_pago DESC",
            &[],
        )
        .await
        .map_err(map_err)?;
    Ok(rows
        .iter()
        .map(|r| Pago {
            id: r.get(0),
            contrato_id: r.get(1),
            periodo: r.get(2),
            fecha_pago: r.get(3),
            monto_alquiler: r.get(4),
            dias_mora: r.get(5),
            monto_mora: r.get(6),
            monto_total: r.get(7),
            metodo_pago: r.get(8),
            numero_recibo: r.get(9),
            notas: r.get(10),
        })
        .collect())
}

#[tauri::command]
pub async fn registrar_pago(state: State<'_, DbState>, nuevo: NuevoPago) -> Result<i64, String> {
    let conn = obtener_conn(&state).await?;
    let fila_contrato = conn
        .query_one("SELECT dia_pago, tasa_mora_diaria FROM contratos WHERE id=$1", &[&nuevo.contrato_id])
        .await
        .map_err(map_err)?;
    let dia_pago: i64 = fila_contrato.get(0);
    let tasa_mora_diaria: f64 = fila_contrato.get(1);

    let fecha_pago = parse_date(&nuevo.fecha_pago);
    let vencimiento = due_date_for_periodo(&nuevo.periodo, dia_pago);
    let monto_alquiler = monto_vigente(&conn, nuevo.contrato_id, vencimiento).await?;
    let dias_mora = (fecha_pago - vencimiento).num_days().max(0);
    let monto_mora = monto_alquiler * (tasa_mora_diaria / 100.0) * dias_mora as f64;
    let monto_total = monto_alquiler + monto_mora;

    let numero_recibo: i64 = conn
        .query_one("SELECT COALESCE(MAX(numero_recibo),0)+1 FROM pagos", &[])
        .await
        .map_err(map_err)?
        .get(0);

    let fila = conn.query_one(
        "INSERT INTO pagos (contrato_id, periodo, fecha_pago, monto_alquiler, dias_mora, monto_mora, monto_total, metodo_pago, numero_recibo, notas)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING id",
        &[&nuevo.contrato_id, &nuevo.periodo, &nuevo.fecha_pago, &monto_alquiler, &dias_mora, &monto_mora, &monto_total, &nuevo.metodo_pago, &numero_recibo, &nuevo.notas],
    ).await.map_err(map_err)?;
    Ok(fila.get(0))
}

#[tauri::command]
pub async fn eliminar_pago(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let conn = obtener_conn(&state).await?;
    conn.execute("DELETE FROM liquidaciones WHERE pago_id=$1", &[&id]).await.map_err(map_err)?;
    conn.execute("DELETE FROM pagos WHERE id=$1", &[&id]).await.map_err(map_err)?;
    Ok(())
}

#[tauri::command]
pub async fn get_recibo(state: State<'_, DbState>, pago_id: i64) -> Result<ReciboCompleto, String> {
    let conn = obtener_conn(&state).await?;
    let r = conn
        .query_one(
            "SELECT id, contrato_id, periodo, fecha_pago, monto_alquiler, dias_mora, monto_mora, monto_total, metodo_pago, numero_recibo, notas FROM pagos WHERE id=$1",
            &[&pago_id],
        )
        .await
        .map_err(map_err)?;
    let pago = Pago {
        id: r.get(0),
        contrato_id: r.get(1),
        periodo: r.get(2),
        fecha_pago: r.get(3),
        monto_alquiler: r.get(4),
        dias_mora: r.get(5),
        monto_mora: r.get(6),
        monto_total: r.get(7),
        metodo_pago: r.get(8),
        numero_recibo: r.get(9),
        notas: r.get(10),
    };

    let r2 = conn
        .query_one(
            "SELECT iq.nombre, im.direccion, p.nombre
             FROM contratos c
             JOIN inquilinos iq ON iq.id = c.inquilino_id
             JOIN inmuebles im ON im.id = c.inmueble_id
             JOIN propietarios p ON p.id = im.propietario_id
             WHERE c.id = $1",
            &[&pago.contrato_id],
        )
        .await
        .map_err(map_err)?;

    Ok(ReciboCompleto {
        pago,
        inquilino_nombre: r2.get(0),
        inmueble_direccion: r2.get(1),
        propietario_nombre: r2.get(2),
    })
}

// ---------- Egresos (liquidaciones a propietarios / comisiones) ----------

#[tauri::command]
pub async fn get_liquidaciones(state: State<'_, DbState>) -> Result<Vec<Liquidacion>, String> {
    let conn = obtener_conn(&state).await?;
    let rows = conn
        .query(
            "SELECT id, pago_id, fecha, monto_alquiler, comision_porcentaje, monto_comision, monto_neto, numero_comprobante, notas FROM liquidaciones ORDER BY fecha DESC",
            &[],
        )
        .await
        .map_err(map_err)?;
    Ok(rows
        .iter()
        .map(|r| Liquidacion {
            id: r.get(0),
            pago_id: r.get(1),
            fecha: r.get(2),
            monto_alquiler: r.get(3),
            comision_porcentaje: r.get(4),
            monto_comision: r.get(5),
            monto_neto: r.get(6),
            numero_comprobante: r.get(7),
            notas: r.get(8),
        })
        .collect())
}

#[tauri::command]
pub async fn generar_liquidacion(state: State<'_, DbState>, pago_id: i64, comision_porcentaje: Option<f64>, fecha: String, notas: Option<String>) -> Result<i64, String> {
    let conn = obtener_conn(&state).await?;
    let fila_pago = conn
        .query_one("SELECT contrato_id, monto_alquiler, monto_total FROM pagos WHERE id=$1", &[&pago_id])
        .await
        .map_err(map_err)?;
    let contrato_id: i64 = fila_pago.get(0);
    let monto_alquiler: f64 = fila_pago.get(1);
    let monto_total: f64 = fila_pago.get(2);

    let ya_existe = conn
        .query_opt("SELECT id FROM liquidaciones WHERE pago_id=$1", &[&pago_id])
        .await
        .map_err(map_err)?;
    if ya_existe.is_some() {
        return Err("Este pago ya tiene un comprobante de liquidacion generado".to_string());
    }

    let comision_pct = match comision_porcentaje {
        Some(v) => v,
        None => conn
            .query_one("SELECT comision_porcentaje FROM contratos WHERE id=$1", &[&contrato_id])
            .await
            .map_err(map_err)?
            .get(0),
    };

    let monto_comision = monto_alquiler * (comision_pct / 100.0);
    let monto_neto = monto_total - monto_comision;

    let numero_comprobante: i64 = conn
        .query_one("SELECT COALESCE(MAX(numero_comprobante),0)+1 FROM liquidaciones", &[])
        .await
        .map_err(map_err)?
        .get(0);

    let fila = conn.query_one(
        "INSERT INTO liquidaciones (pago_id, fecha, monto_alquiler, comision_porcentaje, monto_comision, monto_neto, numero_comprobante, notas)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id",
        &[&pago_id, &fecha, &monto_alquiler, &comision_pct, &monto_comision, &monto_neto, &numero_comprobante, &notas],
    ).await.map_err(map_err)?;
    Ok(fila.get(0))
}

#[tauri::command]
pub async fn eliminar_liquidacion(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let conn = obtener_conn(&state).await?;
    conn.execute("DELETE FROM liquidaciones WHERE id=$1", &[&id]).await.map_err(map_err)?;
    Ok(())
}

#[tauri::command]
pub async fn get_comprobante(state: State<'_, DbState>, liquidacion_id: i64) -> Result<ComprobanteCompleto, String> {
    let conn = obtener_conn(&state).await?;
    let r = conn
        .query_one(
            "SELECT id, pago_id, fecha, monto_alquiler, comision_porcentaje, monto_comision, monto_neto, numero_comprobante, notas FROM liquidaciones WHERE id=$1",
            &[&liquidacion_id],
        )
        .await
        .map_err(map_err)?;
    let liquidacion = Liquidacion {
        id: r.get(0),
        pago_id: r.get(1),
        fecha: r.get(2),
        monto_alquiler: r.get(3),
        comision_porcentaje: r.get(4),
        monto_comision: r.get(5),
        monto_neto: r.get(6),
        numero_comprobante: r.get(7),
        notas: r.get(8),
    };

    let periodo: String = conn
        .query_one("SELECT periodo FROM pagos WHERE id=$1", &[&liquidacion.pago_id])
        .await
        .map_err(map_err)?
        .get(0);

    let r2 = conn
        .query_one(
            "SELECT p.nombre, p.datos_bancarios, im.direccion, iq.nombre
             FROM pagos pa
             JOIN contratos c ON c.id = pa.contrato_id
             JOIN inmuebles im ON im.id = c.inmueble_id
             JOIN propietarios p ON p.id = im.propietario_id
             JOIN inquilinos iq ON iq.id = c.inquilino_id
             WHERE pa.id = $1",
            &[&liquidacion.pago_id],
        )
        .await
        .map_err(map_err)?;

    Ok(ComprobanteCompleto {
        liquidacion,
        periodo,
        propietario_nombre: r2.get(0),
        propietario_datos_bancarios: r2.get(1),
        inmueble_direccion: r2.get(2),
        inquilino_nombre: r2.get(3),
    })
}

// ---------- Tablero de control ----------

#[tauri::command]
pub async fn get_dashboard(state: State<'_, DbState>, dias_vencimiento: i64, dias_actualizacion: i64, dias_cumpleanos: i64) -> Result<ResumenDashboard, String> {
    let conn = obtener_conn(&state).await?;
    let hoy = today();

    struct ContratoBasico {
        id: i64,
        inmueble_direccion: String,
        inquilino_nombre: String,
        propietario_nombre: String,
        fecha_inicio: NaiveDate,
        fecha_fin: NaiveDate,
        dia_pago: i64,
        tasa_mora_diaria: f64,
        frecuencia_actualizacion_meses: i64,
        tipo_actualizacion: String,
    }

    let filas = conn
        .query(
            "SELECT c.id, im.direccion, iq.nombre, p.nombre, c.fecha_inicio, c.fecha_fin, c.dia_pago, c.tasa_mora_diaria, c.frecuencia_actualizacion_meses, c.tipo_actualizacion
             FROM contratos c
             JOIN inmuebles im ON im.id = c.inmueble_id
             JOIN propietarios p ON p.id = im.propietario_id
             JOIN inquilinos iq ON iq.id = c.inquilino_id
             WHERE c.estado = 'activo'",
            &[],
        )
        .await
        .map_err(map_err)?;

    let contratos: Vec<ContratoBasico> = filas
        .iter()
        .map(|r| ContratoBasico {
            id: r.get(0),
            inmueble_direccion: r.get(1),
            inquilino_nombre: r.get(2),
            propietario_nombre: r.get(3),
            fecha_inicio: parse_date(&r.get::<_, String>(4)),
            fecha_fin: parse_date(&r.get::<_, String>(5)),
            dia_pago: r.get(6),
            tasa_mora_diaria: r.get(7),
            frecuencia_actualizacion_meses: r.get(8),
            tipo_actualizacion: r.get(9),
        })
        .collect();

    let mut deudas = Vec::new();
    let mut vencimientos = Vec::new();
    let mut actualizaciones_pendientes = Vec::new();
    let mut total_adeudado = 0.0;

    for c in &contratos {
        // --- deudas ---
        let mut periodos_impagos = Vec::new();
        let mut monto_adeudado = 0.0;
        let mut cursor = NaiveDate::from_ymd_opt(c.fecha_inicio.year(), c.fecha_inicio.month(), 1).unwrap();
        let limite = NaiveDate::from_ymd_opt(hoy.year(), hoy.month(), 1).unwrap();
        while cursor <= limite {
            let periodo = periodo_de(cursor);
            let vencimiento = due_date_for_periodo(&periodo, c.dia_pago);
            if vencimiento <= hoy {
                let pagado = conn
                    .query_opt("SELECT id FROM pagos WHERE contrato_id=$1 AND periodo=$2 LIMIT 1", &[&c.id, &periodo])
                    .await
                    .map_err(map_err)?;
                if pagado.is_none() {
                    let monto = monto_vigente(&conn, c.id, vencimiento).await?;
                    let dias_mora = (hoy - vencimiento).num_days().max(0);
                    let mora = monto * (c.tasa_mora_diaria / 100.0) * dias_mora as f64;
                    periodos_impagos.push(periodo.clone());
                    monto_adeudado += monto + mora;
                }
            }
            cursor = add_months(cursor, 1);
        }
        if !periodos_impagos.is_empty() {
            total_adeudado += monto_adeudado;
            deudas.push(DeudaContrato {
                contrato_id: c.id,
                inmueble_direccion: c.inmueble_direccion.clone(),
                inquilino_nombre: c.inquilino_nombre.clone(),
                periodos_impagos,
                monto_adeudado,
            });
        }

        // --- vencimientos de contrato ---
        let dias_restantes = (c.fecha_fin - hoy).num_days();
        if dias_restantes <= dias_vencimiento {
            vencimientos.push(VencimientoContrato {
                contrato_id: c.id,
                inmueble_direccion: c.inmueble_direccion.clone(),
                inquilino_nombre: c.inquilino_nombre.clone(),
                propietario_nombre: c.propietario_nombre.clone(),
                fecha_fin: c.fecha_fin.format("%Y-%m-%d").to_string(),
                dias_restantes,
            });
        }

        // --- actualizaciones pendientes ---
        let proxima = proxima_fecha_actualizacion(&conn, c.id, c.fecha_inicio, c.frecuencia_actualizacion_meses).await?;
        let dias_restantes_act = (proxima - hoy).num_days();
        if dias_restantes_act <= dias_actualizacion {
            let monto_vig = monto_vigente(&conn, c.id, hoy).await?;
            actualizaciones_pendientes.push(ActualizacionPendiente {
                contrato_id: c.id,
                inmueble_direccion: c.inmueble_direccion.clone(),
                inquilino_nombre: c.inquilino_nombre.clone(),
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
        let rows = conn
            .query(
                &format!("SELECT nombre, fecha_nacimiento FROM {} WHERE fecha_nacimiento IS NOT NULL AND fecha_nacimiento != ''", tabla),
                &[],
            )
            .await
            .map_err(map_err)?;
        for r in rows {
            personas.push((r.get(0), tipo, r.get(1)));
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

    Ok(ResumenDashboard {
        deudas,
        vencimientos,
        actualizaciones_pendientes,
        cumpleanos_proximos,
        total_contratos_activos: contratos.len() as i64,
        total_adeudado,
    })
}

// ---------- Actualización por índice ICL (BCRA) ----------

/// Consulta la API del BCRA y compara el ICL en `fecha_objetivo` contra el ICL
/// vigente cuando se fijó el monto actual del contrato. `fecha_objetivo` es
/// "hoy" para una estimación aproximada (todavía no se sabe el valor final del
/// día de la actualización) o la fecha real de la próxima actualización para
/// obtener el valor definitivo.
async fn calcular_con_icl(state: &DbState, contrato_id: i64, fecha_objetivo_es_hoy: bool) -> Result<EstimacionIcl, String> {
    let conn = obtener_conn(state).await?;
    let fila = conn
        .query_one(
            "SELECT fecha_inicio, frecuencia_actualizacion_meses, tipo_actualizacion FROM contratos WHERE id=$1",
            &[&contrato_id],
        )
        .await
        .map_err(map_err)?;
    let fecha_inicio_s: String = fila.get(0);
    let frecuencia: i64 = fila.get(1);
    let tipo_actualizacion: String = fila.get(2);

    let fecha_inicio = parse_date(&fecha_inicio_s);
    let fecha_referencia = fecha_base_actualizacion(&conn, contrato_id, fecha_inicio).await?;
    let fecha_objetivo = if fecha_objetivo_es_hoy {
        today()
    } else {
        proxima_fecha_actualizacion(&conn, contrato_id, fecha_inicio, frecuencia).await?
    };
    let monto_actual = monto_vigente(&conn, contrato_id, today()).await?;
    drop(conn);

    if tipo_actualizacion != "ICL" {
        return Err("Este contrato no usa el índice ICL como esquema de actualización".to_string());
    }

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

fn fila_a_usuario(r: &tokio_postgres::Row) -> Usuario {
    Usuario {
        id: r.get(0),
        username: r.get(1),
        nombre_completo: r.get(2),
        activo: r.get(3),
    }
}

/// true si todavia no se creo ningun usuario (primer arranque de la app).
#[tauri::command]
pub async fn hay_usuarios(state: State<'_, DbState>) -> Result<bool, String> {
    let conn = obtener_conn(&state).await?;
    let cantidad: i64 = conn.query_one("SELECT COUNT(*) FROM usuarios", &[]).await.map_err(map_err)?.get(0);
    Ok(cantidad > 0)
}

#[tauri::command]
pub async fn get_usuarios(state: State<'_, DbState>) -> Result<Vec<Usuario>, String> {
    let conn = obtener_conn(&state).await?;
    let rows = conn
        .query("SELECT id, username, nombre_completo, activo FROM usuarios ORDER BY nombre_completo", &[])
        .await
        .map_err(map_err)?;
    Ok(rows.iter().map(fila_a_usuario).collect())
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
    let conn = obtener_conn(&state).await?;
    let fila = conn
        .query_one(
            "INSERT INTO usuarios (username, password_hash, nombre_completo, activo) VALUES ($1, $2, $3, TRUE) RETURNING id, username, nombre_completo, activo",
            &[&nuevo.username.trim(), &hash, &nuevo.nombre_completo.trim()],
        )
        .await
        .map_err(|e| {
            if e.code() == Some(&SqlState::UNIQUE_VIOLATION) {
                "Ya existe un usuario con ese nombre de usuario".to_string()
            } else {
                map_err(e)
            }
        })?;
    Ok(fila_a_usuario(&fila))
}

#[tauri::command]
pub async fn eliminar_usuario(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let conn = obtener_conn(&state).await?;
    conn.execute("DELETE FROM usuarios WHERE id=$1", &[&id]).await.map_err(map_err)?;
    Ok(())
}

#[tauri::command]
pub async fn iniciar_sesion(state: State<'_, DbState>, username: String, password: String) -> Result<Usuario, String> {
    let conn = obtener_conn(&state).await?;
    let fila = conn
        .query_opt(
            "SELECT id, username, password_hash, nombre_completo, activo FROM usuarios WHERE username = $1",
            &[&username.trim()],
        )
        .await
        .map_err(map_err)?
        .ok_or_else(|| "Usuario o contraseña incorrectos".to_string())?;

    let id: i64 = fila.get(0);
    let username: String = fila.get(1);
    let password_hash: String = fila.get(2);
    let nombre_completo: String = fila.get(3);
    let activo: bool = fila.get(4);

    if !activo {
        return Err("Este usuario está deshabilitado".to_string());
    }

    let valido = bcrypt::verify(&password, &password_hash).map_err(map_err)?;
    if !valido {
        return Err("Usuario o contraseña incorrectos".to_string());
    }

    Ok(Usuario { id, username, nombre_completo, activo: true })
}

/// Test de integración contra un Postgres real: valida que el esquema y el
/// SQL de cada comando (RETURNING, ON CONFLICT, UNIQUE, joins, el conteo
/// dinámico de cumpleaños) corren tal cual contra Postgres, no solo que
/// compilan. Se salta solo si TEST_DATABASE_URL no está definida.
#[cfg(test)]
mod tests_postgres {
    use super::*;

    async fn conectar_test() -> Option<Pool> {
        let url = std::env::var("TEST_DATABASE_URL").ok()?;
        Some(crate::db::conectar(&url).await.expect("no se pudo conectar a la base de datos de test"))
    }

    #[tokio::test]
    async fn flujo_completo_contra_postgres() {
        let Some(pool) = conectar_test().await else {
            eprintln!("TEST_DATABASE_URL no está definida: se omite el test de integración con Postgres");
            return;
        };
        let state = DbState(RwLock::new(Some(pool)));
        let conn = obtener_conn(&state).await.unwrap();

        conn.batch_execute(
            "TRUNCATE usuarios, propietarios, inquilinos, garantes, inmuebles, contratos, contrato_garantes, actualizaciones, pagos, liquidaciones RESTART IDENTITY CASCADE",
        )
        .await
        .unwrap();

        // --- usuarios: alta, UNIQUE violation, bcrypt ---
        let hash = bcrypt::hash("claveSegura1", bcrypt::DEFAULT_COST).unwrap();
        let fila = conn
            .query_one(
                "INSERT INTO usuarios (username, password_hash, nombre_completo, activo) VALUES ($1,$2,$3,TRUE) RETURNING id, username, nombre_completo, activo",
                &[&"agustin", &hash, &"Agustín"],
            )
            .await
            .unwrap();
        let usuario = fila_a_usuario(&fila);
        assert_eq!(usuario.username, "agustin");
        assert!(usuario.activo);

        let duplicado = conn
            .query_one(
                "INSERT INTO usuarios (username, password_hash, nombre_completo, activo) VALUES ($1,$2,$3,TRUE) RETURNING id",
                &[&"agustin", &hash, &"Otro"],
            )
            .await;
        let err = duplicado.unwrap_err();
        assert_eq!(err.code(), Some(&SqlState::UNIQUE_VIOLATION));

        let fila_login = conn
            .query_opt("SELECT password_hash FROM usuarios WHERE username = $1", &[&"agustin"])
            .await
            .unwrap()
            .unwrap();
        let hash_guardado: String = fila_login.get(0);
        assert!(bcrypt::verify("claveSegura1", &hash_guardado).unwrap());
        assert!(!bcrypt::verify("claveIncorrecta", &hash_guardado).unwrap());

        // --- propietarios: insert con RETURNING, update ---
        let propietario_id: i64 = conn
            .query_one(
                "INSERT INTO propietarios (nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, datos_bancarios, notas) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id",
                &[&"Monchola", &"20111222", &"1980-05-20", &"1122334455", &None::<String>, &None::<String>, &"CBU 123", &None::<String>],
            )
            .await
            .unwrap()
            .get(0);
        conn.execute("UPDATE propietarios SET telefono=$1 WHERE id=$2", &[&"1199998888", &propietario_id]).await.unwrap();
        let telefono: String = conn
            .query_one("SELECT telefono FROM propietarios WHERE id=$1", &[&propietario_id])
            .await
            .unwrap()
            .get(0);
        assert_eq!(telefono, "1199998888");

        // --- inmueble con FK a propietario ---
        let inmueble_id: i64 = conn
            .query_one(
                "INSERT INTO inmuebles (propietario_id, direccion, tipo, superficie, ambientes, notas) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
                &[&propietario_id, &"Mitre 586", &"Departamento", &45.0_f64, &2_i64, &None::<String>],
            )
            .await
            .unwrap()
            .get(0);

        // --- inquilino y garante ---
        let inquilino_id: i64 = conn
            .query_one(
                "INSERT INTO inquilinos (nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, notas) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id",
                &[&"Juan Pérez", &None::<String>, &None::<String>, &None::<String>, &None::<String>, &None::<String>, &None::<String>],
            )
            .await
            .unwrap()
            .get(0);
        let garante_id: i64 = conn
            .query_one(
                "INSERT INTO garantes (nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, notas) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id",
                &[&"María Gómez", &None::<String>, &None::<String>, &None::<String>, &None::<String>, &None::<String>, &None::<String>],
            )
            .await
            .unwrap()
            .get(0);

        // --- contrato con joins (construir_contrato_detallado) ---
        let contrato_id: i64 = conn
            .query_one(
                "INSERT INTO contratos (inmueble_id, inquilino_id, fecha_inicio, fecha_fin, dia_pago, monto_inicial,
                 comision_porcentaje, tasa_mora_diaria, frecuencia_actualizacion_meses, tipo_actualizacion,
                 porcentaje_actualizacion, estado, notas) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) RETURNING id",
                &[&inmueble_id, &inquilino_id, &"2026-01-01", &"2028-01-01", &10_i64, &400000.0_f64,
                    &5.0_f64, &0.1_f64, &3_i64, &"ICL", &0.0_f64, &"activo", &None::<String>],
            )
            .await
            .unwrap()
            .get(0);

        // ON CONFLICT DO NOTHING: insertar el mismo par dos veces no duplica
        for _ in 0..2 {
            conn.execute(
                "INSERT INTO contrato_garantes (contrato_id, garante_id) VALUES ($1,$2) ON CONFLICT DO NOTHING",
                &[&contrato_id, &garante_id],
            )
            .await
            .unwrap();
        }
        let cantidad_garantes: i64 = conn
            .query_one("SELECT COUNT(*) FROM contrato_garantes WHERE contrato_id=$1", &[&contrato_id])
            .await
            .unwrap()
            .get(0);
        assert_eq!(cantidad_garantes, 1);

        let detallado = construir_contrato_detallado(&conn, contrato_id).await.unwrap();
        assert_eq!(detallado.inmueble_direccion, "Mitre 586");
        assert_eq!(detallado.inquilino_nombre, "Juan Pérez");
        assert_eq!(detallado.propietario_nombre, "Monchola");
        assert_eq!(detallado.garantes, vec!["María Gómez".to_string()]);
        assert_eq!(detallado.monto_vigente, 400000.0);

        // --- actualizacion de alquiler ---
        conn.execute(
            "INSERT INTO actualizaciones (contrato_id, fecha_vigencia, monto_nuevo, motivo) VALUES ($1,$2,$3,$4)",
            &[&contrato_id, &"2026-04-01", &440000.0_f64, &"Actualización ICL"],
        )
        .await
        .unwrap();
        let monto_post_actualizacion = monto_vigente(&conn, contrato_id, parse_date("2026-05-01")).await.unwrap();
        assert_eq!(monto_post_actualizacion, 440000.0);
        let monto_antes = monto_vigente(&conn, contrato_id, parse_date("2026-02-01")).await.unwrap();
        assert_eq!(monto_antes, 400000.0);

        // --- pago con numeración secuencial ---
        let numero_recibo: i64 = conn.query_one("SELECT COALESCE(MAX(numero_recibo),0)+1 FROM pagos", &[]).await.unwrap().get(0);
        assert_eq!(numero_recibo, 1);
        let pago_id: i64 = conn
            .query_one(
                "INSERT INTO pagos (contrato_id, periodo, fecha_pago, monto_alquiler, dias_mora, monto_mora, monto_total, metodo_pago, numero_recibo, notas)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING id",
                &[&contrato_id, &"2026-05", &"2026-05-10", &440000.0_f64, &0_i64, &0.0_f64, &440000.0_f64, &"Transferencia", &numero_recibo, &None::<String>],
            )
            .await
            .unwrap()
            .get(0);

        // --- liquidacion ---
        let numero_comprobante: i64 = conn.query_one("SELECT COALESCE(MAX(numero_comprobante),0)+1 FROM liquidaciones", &[]).await.unwrap().get(0);
        assert_eq!(numero_comprobante, 1);
        conn.execute(
            "INSERT INTO liquidaciones (pago_id, fecha, monto_alquiler, comision_porcentaje, monto_comision, monto_neto, numero_comprobante, notas)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            &[&pago_id, &"2026-05-10", &440000.0_f64, &5.0_f64, &22000.0_f64, &418000.0_f64, &numero_comprobante, &None::<String>],
        )
        .await
        .unwrap();

        // --- consulta dinámica de cumpleaños (nombre de tabla por format!) ---
        for tabla in ["propietarios", "inquilinos", "garantes"] {
            let filas = conn
                .query(
                    &format!("SELECT nombre, fecha_nacimiento FROM {} WHERE fecha_nacimiento IS NOT NULL AND fecha_nacimiento != ''", tabla),
                    &[],
                )
                .await
                .unwrap();
            if tabla == "propietarios" {
                assert_eq!(filas.len(), 1);
                let nombre: String = filas[0].get(0);
                assert_eq!(nombre, "Monchola");
            }
        }

        // --- limpieza: el DELETE RESTRICT de propietarios con inmuebles debe fallar ---
        let borrado_restringido = conn.execute("DELETE FROM propietarios WHERE id=$1", &[&propietario_id]).await;
        assert!(borrado_restringido.is_err());
    }
}
