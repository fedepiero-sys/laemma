use crate::models::*;
use chrono::{Datelike, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Mutex;
use tauri::State;

pub struct DbState(pub Mutex<Connection>);

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
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
fn monto_vigente(conn: &Connection, contrato_id: i64, fecha: NaiveDate) -> rusqlite::Result<f64> {
    let monto_inicial: f64 = conn.query_row(
        "SELECT monto_inicial FROM contratos WHERE id = ?1",
        params![contrato_id],
        |r| r.get(0),
    )?;
    let fecha_str = fecha.format("%Y-%m-%d").to_string();
    let actualizado: Option<f64> = conn
        .query_row(
            "SELECT monto_nuevo FROM actualizaciones WHERE contrato_id = ?1 AND fecha_vigencia <= ?2 ORDER BY fecha_vigencia DESC LIMIT 1",
            params![contrato_id, fecha_str],
            |r| r.get(0),
        )
        .optional()?;
    Ok(actualizado.unwrap_or(monto_inicial))
}

fn proxima_fecha_actualizacion(conn: &Connection, contrato_id: i64, fecha_inicio: NaiveDate, frecuencia_meses: i64) -> rusqlite::Result<NaiveDate> {
    let ultima: Option<String> = conn
        .query_row(
            "SELECT fecha_vigencia FROM actualizaciones WHERE contrato_id = ?1 ORDER BY fecha_vigencia DESC LIMIT 1",
            params![contrato_id],
            |r| r.get(0),
        )
        .optional()?;
    let base = match ultima {
        Some(f) => parse_date(&f),
        None => fecha_inicio,
    };
    Ok(add_months(base, frecuencia_meses.max(1)))
}

// ---------- Propietarios ----------

#[tauri::command]
pub fn get_propietarios(state: State<DbState>) -> Result<Vec<Propietario>, String> {
    let conn = state.0.lock().map_err(map_err)?;
    let mut stmt = conn
        .prepare("SELECT id, nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, datos_bancarios, notas FROM propietarios ORDER BY nombre")
        .map_err(map_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Propietario {
                id: r.get(0)?,
                nombre: r.get(1)?,
                dni_cuit: r.get(2)?,
                fecha_nacimiento: r.get(3)?,
                telefono: r.get(4)?,
                email: r.get(5)?,
                direccion: r.get(6)?,
                datos_bancarios: r.get(7)?,
                notas: r.get(8)?,
            })
        })
        .map_err(map_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(map_err)
}

#[tauri::command]
pub fn guardar_propietario(state: State<DbState>, propietario: Propietario) -> Result<i64, String> {
    let conn = state.0.lock().map_err(map_err)?;
    match propietario.id {
        Some(id) => {
            conn.execute(
                "UPDATE propietarios SET nombre=?1, dni_cuit=?2, fecha_nacimiento=?3, telefono=?4, email=?5, direccion=?6, datos_bancarios=?7, notas=?8 WHERE id=?9",
                params![propietario.nombre, propietario.dni_cuit, propietario.fecha_nacimiento, propietario.telefono, propietario.email, propietario.direccion, propietario.datos_bancarios, propietario.notas, id],
            ).map_err(map_err)?;
            Ok(id)
        }
        None => {
            conn.execute(
                "INSERT INTO propietarios (nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, datos_bancarios, notas) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![propietario.nombre, propietario.dni_cuit, propietario.fecha_nacimiento, propietario.telefono, propietario.email, propietario.direccion, propietario.datos_bancarios, propietario.notas],
            ).map_err(map_err)?;
            Ok(conn.last_insert_rowid())
        }
    }
}

#[tauri::command]
pub fn eliminar_propietario(state: State<DbState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(map_err)?;
    conn.execute("DELETE FROM propietarios WHERE id=?1", params![id]).map_err(map_err)?;
    Ok(())
}

// ---------- Inquilinos ----------

#[tauri::command]
pub fn get_inquilinos(state: State<DbState>) -> Result<Vec<Inquilino>, String> {
    let conn = state.0.lock().map_err(map_err)?;
    let mut stmt = conn
        .prepare("SELECT id, nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, notas FROM inquilinos ORDER BY nombre")
        .map_err(map_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Inquilino {
                id: r.get(0)?,
                nombre: r.get(1)?,
                dni_cuit: r.get(2)?,
                fecha_nacimiento: r.get(3)?,
                telefono: r.get(4)?,
                email: r.get(5)?,
                direccion: r.get(6)?,
                notas: r.get(7)?,
            })
        })
        .map_err(map_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(map_err)
}

#[tauri::command]
pub fn guardar_inquilino(state: State<DbState>, inquilino: Inquilino) -> Result<i64, String> {
    let conn = state.0.lock().map_err(map_err)?;
    match inquilino.id {
        Some(id) => {
            conn.execute(
                "UPDATE inquilinos SET nombre=?1, dni_cuit=?2, fecha_nacimiento=?3, telefono=?4, email=?5, direccion=?6, notas=?7 WHERE id=?8",
                params![inquilino.nombre, inquilino.dni_cuit, inquilino.fecha_nacimiento, inquilino.telefono, inquilino.email, inquilino.direccion, inquilino.notas, id],
            ).map_err(map_err)?;
            Ok(id)
        }
        None => {
            conn.execute(
                "INSERT INTO inquilinos (nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, notas) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![inquilino.nombre, inquilino.dni_cuit, inquilino.fecha_nacimiento, inquilino.telefono, inquilino.email, inquilino.direccion, inquilino.notas],
            ).map_err(map_err)?;
            Ok(conn.last_insert_rowid())
        }
    }
}

#[tauri::command]
pub fn eliminar_inquilino(state: State<DbState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(map_err)?;
    conn.execute("DELETE FROM inquilinos WHERE id=?1", params![id]).map_err(map_err)?;
    Ok(())
}

// ---------- Garantes ----------

#[tauri::command]
pub fn get_garantes(state: State<DbState>) -> Result<Vec<Garante>, String> {
    let conn = state.0.lock().map_err(map_err)?;
    let mut stmt = conn
        .prepare("SELECT id, nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, notas FROM garantes ORDER BY nombre")
        .map_err(map_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Garante {
                id: r.get(0)?,
                nombre: r.get(1)?,
                dni_cuit: r.get(2)?,
                fecha_nacimiento: r.get(3)?,
                telefono: r.get(4)?,
                email: r.get(5)?,
                direccion: r.get(6)?,
                notas: r.get(7)?,
            })
        })
        .map_err(map_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(map_err)
}

#[tauri::command]
pub fn guardar_garante(state: State<DbState>, garante: Garante) -> Result<i64, String> {
    let conn = state.0.lock().map_err(map_err)?;
    match garante.id {
        Some(id) => {
            conn.execute(
                "UPDATE garantes SET nombre=?1, dni_cuit=?2, fecha_nacimiento=?3, telefono=?4, email=?5, direccion=?6, notas=?7 WHERE id=?8",
                params![garante.nombre, garante.dni_cuit, garante.fecha_nacimiento, garante.telefono, garante.email, garante.direccion, garante.notas, id],
            ).map_err(map_err)?;
            Ok(id)
        }
        None => {
            conn.execute(
                "INSERT INTO garantes (nombre, dni_cuit, fecha_nacimiento, telefono, email, direccion, notas) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![garante.nombre, garante.dni_cuit, garante.fecha_nacimiento, garante.telefono, garante.email, garante.direccion, garante.notas],
            ).map_err(map_err)?;
            Ok(conn.last_insert_rowid())
        }
    }
}

#[tauri::command]
pub fn eliminar_garante(state: State<DbState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(map_err)?;
    conn.execute("DELETE FROM garantes WHERE id=?1", params![id]).map_err(map_err)?;
    Ok(())
}

// ---------- Inmuebles ----------

#[tauri::command]
pub fn get_inmuebles(state: State<DbState>) -> Result<Vec<InmuebleDetallado>, String> {
    let conn = state.0.lock().map_err(map_err)?;
    let mut stmt = conn
        .prepare(
            "SELECT i.id, i.propietario_id, p.nombre, i.direccion, i.tipo, i.superficie, i.ambientes, i.notas
             FROM inmuebles i JOIN propietarios p ON p.id = i.propietario_id
             ORDER BY i.direccion",
        )
        .map_err(map_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(InmuebleDetallado {
                id: r.get(0)?,
                propietario_id: r.get(1)?,
                propietario_nombre: r.get(2)?,
                direccion: r.get(3)?,
                tipo: r.get(4)?,
                superficie: r.get(5)?,
                ambientes: r.get(6)?,
                notas: r.get(7)?,
            })
        })
        .map_err(map_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(map_err)
}

#[tauri::command]
pub fn guardar_inmueble(state: State<DbState>, inmueble: Inmueble) -> Result<i64, String> {
    let conn = state.0.lock().map_err(map_err)?;
    match inmueble.id {
        Some(id) => {
            conn.execute(
                "UPDATE inmuebles SET propietario_id=?1, direccion=?2, tipo=?3, superficie=?4, ambientes=?5, notas=?6 WHERE id=?7",
                params![inmueble.propietario_id, inmueble.direccion, inmueble.tipo, inmueble.superficie, inmueble.ambientes, inmueble.notas, id],
            ).map_err(map_err)?;
            Ok(id)
        }
        None => {
            conn.execute(
                "INSERT INTO inmuebles (propietario_id, direccion, tipo, superficie, ambientes, notas) VALUES (?1,?2,?3,?4,?5,?6)",
                params![inmueble.propietario_id, inmueble.direccion, inmueble.tipo, inmueble.superficie, inmueble.ambientes, inmueble.notas],
            ).map_err(map_err)?;
            Ok(conn.last_insert_rowid())
        }
    }
}

#[tauri::command]
pub fn eliminar_inmueble(state: State<DbState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(map_err)?;
    conn.execute("DELETE FROM inmuebles WHERE id=?1", params![id]).map_err(map_err)?;
    Ok(())
}

// ---------- Contratos ----------

fn cargar_garantes_de(conn: &Connection, contrato_id: i64) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT g.nombre FROM contrato_garantes cg JOIN garantes g ON g.id = cg.garante_id WHERE cg.contrato_id = ?1 ORDER BY g.nombre",
    )?;
    let rows = stmt.query_map(params![contrato_id], |r| r.get::<_, String>(0))?;
    rows.collect()
}

fn cargar_garante_ids_de(conn: &Connection, contrato_id: i64) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT garante_id FROM contrato_garantes WHERE contrato_id = ?1")?;
    let rows = stmt.query_map(params![contrato_id], |r| r.get::<_, i64>(0))?;
    rows.collect()
}

fn construir_contrato_detallado(conn: &Connection, id: i64) -> rusqlite::Result<ContratoDetallado> {
    let fila = conn.query_row(
        "SELECT c.id, c.inmueble_id, im.direccion, c.inquilino_id, iq.nombre, im.propietario_id, p.nombre,
                c.fecha_inicio, c.fecha_fin, c.dia_pago, c.monto_inicial, c.comision_porcentaje, c.tasa_mora_diaria,
                c.frecuencia_actualizacion_meses, c.tipo_actualizacion, c.porcentaje_actualizacion, c.estado, c.notas
         FROM contratos c
         JOIN inmuebles im ON im.id = c.inmueble_id
         JOIN propietarios p ON p.id = im.propietario_id
         JOIN inquilinos iq ON iq.id = c.inquilino_id
         WHERE c.id = ?1",
        params![id],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, i64>(9)?,
                r.get::<_, f64>(10)?,
                r.get::<_, f64>(11)?,
                r.get::<_, f64>(12)?,
                r.get::<_, i64>(13)?,
                r.get::<_, String>(14)?,
                r.get::<_, f64>(15)?,
                r.get::<_, String>(16)?,
                r.get::<_, Option<String>>(17)?,
            ))
        },
    )?;
    let (cid, inmueble_id, inmueble_direccion, inquilino_id, inquilino_nombre, propietario_id, propietario_nombre,
        fecha_inicio, fecha_fin, dia_pago, monto_inicial, comision_porcentaje, tasa_mora_diaria,
        frecuencia_actualizacion_meses, tipo_actualizacion, porcentaje_actualizacion, estado, notas) = fila;

    let garantes = cargar_garantes_de(conn, cid)?;
    let garante_ids = cargar_garante_ids_de(conn, cid)?;
    let monto_vig = monto_vigente(conn, cid, today())?;
    let proxima = proxima_fecha_actualizacion(conn, cid, parse_date(&fecha_inicio), frecuencia_actualizacion_meses)?;

    let _ = monto_inicial; // se conserva en el registro, se expone igual

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
pub fn get_contratos(state: State<DbState>) -> Result<Vec<ContratoDetallado>, String> {
    let conn = state.0.lock().map_err(map_err)?;
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM contratos ORDER BY fecha_fin").map_err(map_err)?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0)).map_err(map_err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_err)?
    };
    ids.into_iter()
        .map(|id| construir_contrato_detallado(&conn, id).map_err(map_err))
        .collect()
}

#[tauri::command]
pub fn guardar_contrato(state: State<DbState>, contrato: Contrato) -> Result<i64, String> {
    let conn = state.0.lock().map_err(map_err)?;
    let id = match contrato.id {
        Some(id) => {
            conn.execute(
                "UPDATE contratos SET inmueble_id=?1, inquilino_id=?2, fecha_inicio=?3, fecha_fin=?4, dia_pago=?5,
                 monto_inicial=?6, comision_porcentaje=?7, tasa_mora_diaria=?8, frecuencia_actualizacion_meses=?9,
                 tipo_actualizacion=?10, porcentaje_actualizacion=?11, estado=?12, notas=?13 WHERE id=?14",
                params![contrato.inmueble_id, contrato.inquilino_id, contrato.fecha_inicio, contrato.fecha_fin,
                    contrato.dia_pago, contrato.monto_inicial, contrato.comision_porcentaje, contrato.tasa_mora_diaria,
                    contrato.frecuencia_actualizacion_meses, contrato.tipo_actualizacion, contrato.porcentaje_actualizacion,
                    contrato.estado, contrato.notas, id],
            ).map_err(map_err)?;
            conn.execute("DELETE FROM contrato_garantes WHERE contrato_id=?1", params![id]).map_err(map_err)?;
            id
        }
        None => {
            conn.execute(
                "INSERT INTO contratos (inmueble_id, inquilino_id, fecha_inicio, fecha_fin, dia_pago, monto_inicial,
                 comision_porcentaje, tasa_mora_diaria, frecuencia_actualizacion_meses, tipo_actualizacion,
                 porcentaje_actualizacion, estado, notas) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![contrato.inmueble_id, contrato.inquilino_id, contrato.fecha_inicio, contrato.fecha_fin,
                    contrato.dia_pago, contrato.monto_inicial, contrato.comision_porcentaje, contrato.tasa_mora_diaria,
                    contrato.frecuencia_actualizacion_meses, contrato.tipo_actualizacion, contrato.porcentaje_actualizacion,
                    contrato.estado, contrato.notas],
            ).map_err(map_err)?;
            conn.last_insert_rowid()
        }
    };
    for garante_id in contrato.garante_ids {
        conn.execute(
            "INSERT OR IGNORE INTO contrato_garantes (contrato_id, garante_id) VALUES (?1,?2)",
            params![id, garante_id],
        ).map_err(map_err)?;
    }
    Ok(id)
}

#[tauri::command]
pub fn eliminar_contrato(state: State<DbState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(map_err)?;
    conn.execute("DELETE FROM contratos WHERE id=?1", params![id]).map_err(map_err)?;
    Ok(())
}

// ---------- Actualizaciones de alquiler ----------

#[tauri::command]
pub fn get_actualizaciones(state: State<DbState>, contrato_id: i64) -> Result<Vec<Actualizacion>, String> {
    let conn = state.0.lock().map_err(map_err)?;
    let mut stmt = conn
        .prepare("SELECT id, contrato_id, fecha_vigencia, monto_nuevo, motivo FROM actualizaciones WHERE contrato_id=?1 ORDER BY fecha_vigencia DESC")
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![contrato_id], |r| {
            Ok(Actualizacion {
                id: r.get(0)?,
                contrato_id: r.get(1)?,
                fecha_vigencia: r.get(2)?,
                monto_nuevo: r.get(3)?,
                motivo: r.get(4)?,
            })
        })
        .map_err(map_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(map_err)
}

#[tauri::command]
pub fn agregar_actualizacion(state: State<DbState>, actualizacion: Actualizacion) -> Result<i64, String> {
    let conn = state.0.lock().map_err(map_err)?;
    conn.execute(
        "INSERT INTO actualizaciones (contrato_id, fecha_vigencia, monto_nuevo, motivo) VALUES (?1,?2,?3,?4)",
        params![actualizacion.contrato_id, actualizacion.fecha_vigencia, actualizacion.monto_nuevo, actualizacion.motivo],
    ).map_err(map_err)?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
pub fn eliminar_actualizacion(state: State<DbState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(map_err)?;
    conn.execute("DELETE FROM actualizaciones WHERE id=?1", params![id]).map_err(map_err)?;
    Ok(())
}

// ---------- Ingresos (pagos de inquilinos / recibos) ----------

#[tauri::command]
pub fn get_pagos(state: State<DbState>) -> Result<Vec<Pago>, String> {
    let conn = state.0.lock().map_err(map_err)?;
    let mut stmt = conn
        .prepare("SELECT id, contrato_id, periodo, fecha_pago, monto_alquiler, dias_mora, monto_mora, monto_total, metodo_pago, numero_recibo, notas FROM pagos ORDER BY fecha_pago DESC")
        .map_err(map_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Pago {
                id: r.get(0)?,
                contrato_id: r.get(1)?,
                periodo: r.get(2)?,
                fecha_pago: r.get(3)?,
                monto_alquiler: r.get(4)?,
                dias_mora: r.get(5)?,
                monto_mora: r.get(6)?,
                monto_total: r.get(7)?,
                metodo_pago: r.get(8)?,
                numero_recibo: r.get(9)?,
                notas: r.get(10)?,
            })
        })
        .map_err(map_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(map_err)
}

#[tauri::command]
pub fn registrar_pago(state: State<DbState>, nuevo: NuevoPago) -> Result<i64, String> {
    let conn = state.0.lock().map_err(map_err)?;
    let (dia_pago, tasa_mora_diaria): (i64, f64) = conn
        .query_row(
            "SELECT dia_pago, tasa_mora_diaria FROM contratos WHERE id=?1",
            params![nuevo.contrato_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(map_err)?;

    let fecha_pago = parse_date(&nuevo.fecha_pago);
    let vencimiento = due_date_for_periodo(&nuevo.periodo, dia_pago);
    let monto_alquiler = monto_vigente(&conn, nuevo.contrato_id, vencimiento).map_err(map_err)?;
    let dias_mora = (fecha_pago - vencimiento).num_days().max(0);
    let monto_mora = monto_alquiler * (tasa_mora_diaria / 100.0) * dias_mora as f64;
    let monto_total = monto_alquiler + monto_mora;

    let numero_recibo: i64 = conn
        .query_row("SELECT COALESCE(MAX(numero_recibo),0)+1 FROM pagos", [], |r| r.get(0))
        .map_err(map_err)?;

    conn.execute(
        "INSERT INTO pagos (contrato_id, periodo, fecha_pago, monto_alquiler, dias_mora, monto_mora, monto_total, metodo_pago, numero_recibo, notas)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![nuevo.contrato_id, nuevo.periodo, nuevo.fecha_pago, monto_alquiler, dias_mora, monto_mora, monto_total, nuevo.metodo_pago, numero_recibo, nuevo.notas],
    ).map_err(map_err)?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
pub fn eliminar_pago(state: State<DbState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(map_err)?;
    conn.execute("DELETE FROM liquidaciones WHERE pago_id=?1", params![id]).map_err(map_err)?;
    conn.execute("DELETE FROM pagos WHERE id=?1", params![id]).map_err(map_err)?;
    Ok(())
}

#[tauri::command]
pub fn get_recibo(state: State<DbState>, pago_id: i64) -> Result<ReciboCompleto, String> {
    let conn = state.0.lock().map_err(map_err)?;
    let pago = conn
        .query_row(
            "SELECT id, contrato_id, periodo, fecha_pago, monto_alquiler, dias_mora, monto_mora, monto_total, metodo_pago, numero_recibo, notas FROM pagos WHERE id=?1",
            params![pago_id],
            |r| {
                Ok(Pago {
                    id: r.get(0)?,
                    contrato_id: r.get(1)?,
                    periodo: r.get(2)?,
                    fecha_pago: r.get(3)?,
                    monto_alquiler: r.get(4)?,
                    dias_mora: r.get(5)?,
                    monto_mora: r.get(6)?,
                    monto_total: r.get(7)?,
                    metodo_pago: r.get(8)?,
                    numero_recibo: r.get(9)?,
                    notas: r.get(10)?,
                })
            },
        )
        .map_err(map_err)?;

    let (inquilino_nombre, inmueble_direccion, propietario_nombre) = conn
        .query_row(
            "SELECT iq.nombre, im.direccion, p.nombre
             FROM contratos c
             JOIN inquilinos iq ON iq.id = c.inquilino_id
             JOIN inmuebles im ON im.id = c.inmueble_id
             JOIN propietarios p ON p.id = im.propietario_id
             WHERE c.id = ?1",
            params![pago.contrato_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
        )
        .map_err(map_err)?;

    Ok(ReciboCompleto { pago, inquilino_nombre, inmueble_direccion, propietario_nombre })
}

// ---------- Egresos (liquidaciones a propietarios / comisiones) ----------

#[tauri::command]
pub fn get_liquidaciones(state: State<DbState>) -> Result<Vec<Liquidacion>, String> {
    let conn = state.0.lock().map_err(map_err)?;
    let mut stmt = conn
        .prepare("SELECT id, pago_id, fecha, monto_alquiler, comision_porcentaje, monto_comision, monto_neto, numero_comprobante, notas FROM liquidaciones ORDER BY fecha DESC")
        .map_err(map_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Liquidacion {
                id: r.get(0)?,
                pago_id: r.get(1)?,
                fecha: r.get(2)?,
                monto_alquiler: r.get(3)?,
                comision_porcentaje: r.get(4)?,
                monto_comision: r.get(5)?,
                monto_neto: r.get(6)?,
                numero_comprobante: r.get(7)?,
                notas: r.get(8)?,
            })
        })
        .map_err(map_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(map_err)
}

#[tauri::command]
pub fn generar_liquidacion(state: State<DbState>, pago_id: i64, comision_porcentaje: Option<f64>, fecha: String, notas: Option<String>) -> Result<i64, String> {
    let conn = state.0.lock().map_err(map_err)?;
    let (contrato_id, monto_alquiler, monto_total): (i64, f64, f64) = conn
        .query_row(
            "SELECT contrato_id, monto_alquiler, monto_total FROM pagos WHERE id=?1",
            params![pago_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(map_err)?;

    let ya_existe: Option<i64> = conn
        .query_row("SELECT id FROM liquidaciones WHERE pago_id=?1", params![pago_id], |r| r.get(0))
        .optional()
        .map_err(map_err)?;
    if ya_existe.is_some() {
        return Err("Este pago ya tiene un comprobante de liquidacion generado".to_string());
    }

    let comision_pct = match comision_porcentaje {
        Some(v) => v,
        None => conn
            .query_row("SELECT comision_porcentaje FROM contratos WHERE id=?1", params![contrato_id], |r| r.get(0))
            .map_err(map_err)?,
    };

    let monto_comision = monto_alquiler * (comision_pct / 100.0);
    let monto_neto = monto_total - monto_comision;

    let numero_comprobante: i64 = conn
        .query_row("SELECT COALESCE(MAX(numero_comprobante),0)+1 FROM liquidaciones", [], |r| r.get(0))
        .map_err(map_err)?;

    conn.execute(
        "INSERT INTO liquidaciones (pago_id, fecha, monto_alquiler, comision_porcentaje, monto_comision, monto_neto, numero_comprobante, notas)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![pago_id, fecha, monto_alquiler, comision_pct, monto_comision, monto_neto, numero_comprobante, notas],
    ).map_err(map_err)?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
pub fn eliminar_liquidacion(state: State<DbState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(map_err)?;
    conn.execute("DELETE FROM liquidaciones WHERE id=?1", params![id]).map_err(map_err)?;
    Ok(())
}

#[tauri::command]
pub fn get_comprobante(state: State<DbState>, liquidacion_id: i64) -> Result<ComprobanteCompleto, String> {
    let conn = state.0.lock().map_err(map_err)?;
    let liquidacion = conn
        .query_row(
            "SELECT id, pago_id, fecha, monto_alquiler, comision_porcentaje, monto_comision, monto_neto, numero_comprobante, notas FROM liquidaciones WHERE id=?1",
            params![liquidacion_id],
            |r| {
                Ok(Liquidacion {
                    id: r.get(0)?,
                    pago_id: r.get(1)?,
                    fecha: r.get(2)?,
                    monto_alquiler: r.get(3)?,
                    comision_porcentaje: r.get(4)?,
                    monto_comision: r.get(5)?,
                    monto_neto: r.get(6)?,
                    numero_comprobante: r.get(7)?,
                    notas: r.get(8)?,
                })
            },
        )
        .map_err(map_err)?;

    let periodo: String = conn
        .query_row("SELECT periodo FROM pagos WHERE id=?1", params![liquidacion.pago_id], |r| r.get(0))
        .map_err(map_err)?;

    let (propietario_nombre, propietario_datos_bancarios, inmueble_direccion, inquilino_nombre) = conn
        .query_row(
            "SELECT p.nombre, p.datos_bancarios, im.direccion, iq.nombre
             FROM pagos pa
             JOIN contratos c ON c.id = pa.contrato_id
             JOIN inmuebles im ON im.id = c.inmueble_id
             JOIN propietarios p ON p.id = im.propietario_id
             JOIN inquilinos iq ON iq.id = c.inquilino_id
             WHERE pa.id = ?1",
            params![liquidacion.pago_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)),
        )
        .map_err(map_err)?;

    Ok(ComprobanteCompleto {
        liquidacion,
        periodo,
        propietario_nombre,
        propietario_datos_bancarios,
        inmueble_direccion,
        inquilino_nombre,
    })
}

// ---------- Tablero de control ----------

#[tauri::command]
pub fn get_dashboard(state: State<DbState>, dias_vencimiento: i64, dias_actualizacion: i64, dias_cumpleanos: i64) -> Result<ResumenDashboard, String> {
    let conn = state.0.lock().map_err(map_err)?;
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
    }

    let contratos: Vec<ContratoBasico> = {
        let mut stmt = conn
            .prepare(
                "SELECT c.id, im.direccion, iq.nombre, p.nombre, c.fecha_inicio, c.fecha_fin, c.dia_pago, c.tasa_mora_diaria, c.frecuencia_actualizacion_meses
                 FROM contratos c
                 JOIN inmuebles im ON im.id = c.inmueble_id
                 JOIN propietarios p ON p.id = im.propietario_id
                 JOIN inquilinos iq ON iq.id = c.inquilino_id
                 WHERE c.estado = 'activo'",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ContratoBasico {
                    id: r.get(0)?,
                    inmueble_direccion: r.get(1)?,
                    inquilino_nombre: r.get(2)?,
                    propietario_nombre: r.get(3)?,
                    fecha_inicio: parse_date(&r.get::<_, String>(4)?),
                    fecha_fin: parse_date(&r.get::<_, String>(5)?),
                    dia_pago: r.get(6)?,
                    tasa_mora_diaria: r.get(7)?,
                    frecuencia_actualizacion_meses: r.get(8)?,
                })
            })
            .map_err(map_err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_err)?
    };

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
                let pagado: Option<i64> = conn
                    .query_row(
                        "SELECT id FROM pagos WHERE contrato_id=?1 AND periodo=?2 LIMIT 1",
                        params![c.id, periodo],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(map_err)?;
                if pagado.is_none() {
                    let monto = monto_vigente(&conn, c.id, vencimiento).map_err(map_err)?;
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
        let proxima = proxima_fecha_actualizacion(&conn, c.id, c.fecha_inicio, c.frecuencia_actualizacion_meses).map_err(map_err)?;
        let dias_restantes_act = (proxima - hoy).num_days();
        if dias_restantes_act <= dias_actualizacion {
            let monto_vig = monto_vigente(&conn, c.id, hoy).map_err(map_err)?;
            actualizaciones_pendientes.push(ActualizacionPendiente {
                contrato_id: c.id,
                inmueble_direccion: c.inmueble_direccion.clone(),
                inquilino_nombre: c.inquilino_nombre.clone(),
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
        let mut stmt = conn
            .prepare(&format!(
                "SELECT nombre, fecha_nacimiento FROM {} WHERE fecha_nacimiento IS NOT NULL AND fecha_nacimiento != ''",
                tabla
            ))
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(map_err)?;
        for fila in rows {
            let (nombre, fecha_nacimiento) = fila.map_err(map_err)?;
            personas.push((nombre, tipo, fecha_nacimiento));
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
