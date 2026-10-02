use rusqlite::Connection;
use std::path::PathBuf;

pub fn connect(db_path: &PathBuf) -> Connection {
    let conn = Connection::open(db_path).expect("No se pudo abrir la base de datos");
    conn.execute_batch(SCHEMA).expect("No se pudo inicializar el esquema");
    migrate(&conn);
    conn
}

/// Agrega columnas nuevas a bases de datos creadas con un esquema anterior,
/// sin tocar los datos ya cargados.
fn migrate(conn: &Connection) {
    for tabla in ["propietarios", "inquilinos", "garantes"] {
        if !column_exists(conn, tabla, "fecha_nacimiento") {
            conn.execute(
                &format!("ALTER TABLE {} ADD COLUMN fecha_nacimiento TEXT", tabla),
                [],
            )
            .expect("No se pudo migrar el esquema (fecha_nacimiento)");
        }
    }
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({})", table))
        .expect("No se pudo inspeccionar el esquema");
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .expect("No se pudo leer el esquema")
        .filter_map(|c| c.ok())
        .collect();
    cols.iter().any(|c| c == column)
}

const SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS usuarios (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    username        TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,
    nombre_completo TEXT NOT NULL,
    activo          INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS propietarios (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    nombre            TEXT NOT NULL,
    dni_cuit          TEXT,
    fecha_nacimiento  TEXT,
    telefono          TEXT,
    email             TEXT,
    direccion         TEXT,
    datos_bancarios   TEXT,
    notas             TEXT
);

CREATE TABLE IF NOT EXISTS inquilinos (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    nombre            TEXT NOT NULL,
    dni_cuit          TEXT,
    fecha_nacimiento  TEXT,
    telefono          TEXT,
    email             TEXT,
    direccion         TEXT,
    notas             TEXT
);

CREATE TABLE IF NOT EXISTS garantes (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    nombre            TEXT NOT NULL,
    dni_cuit          TEXT,
    fecha_nacimiento  TEXT,
    telefono          TEXT,
    email             TEXT,
    direccion         TEXT,
    notas             TEXT
);

CREATE TABLE IF NOT EXISTS inmuebles (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    propietario_id INTEGER NOT NULL REFERENCES propietarios(id) ON DELETE RESTRICT,
    direccion      TEXT NOT NULL,
    tipo           TEXT,
    superficie     REAL,
    ambientes      INTEGER,
    notas          TEXT
);

CREATE TABLE IF NOT EXISTS contratos (
    id                          INTEGER PRIMARY KEY AUTOINCREMENT,
    inmueble_id                 INTEGER NOT NULL REFERENCES inmuebles(id) ON DELETE RESTRICT,
    inquilino_id                INTEGER NOT NULL REFERENCES inquilinos(id) ON DELETE RESTRICT,
    fecha_inicio                TEXT NOT NULL,
    fecha_fin                   TEXT NOT NULL,
    dia_pago                    INTEGER NOT NULL DEFAULT 10,
    monto_inicial                REAL NOT NULL,
    comision_porcentaje         REAL NOT NULL DEFAULT 0,
    tasa_mora_diaria            REAL NOT NULL DEFAULT 0,
    frecuencia_actualizacion_meses INTEGER NOT NULL DEFAULT 12,
    tipo_actualizacion          TEXT NOT NULL DEFAULT 'porcentaje_fijo',
    porcentaje_actualizacion    REAL NOT NULL DEFAULT 0,
    estado                      TEXT NOT NULL DEFAULT 'activo',
    notas                       TEXT
);

CREATE TABLE IF NOT EXISTS contrato_garantes (
    contrato_id INTEGER NOT NULL REFERENCES contratos(id) ON DELETE CASCADE,
    garante_id  INTEGER NOT NULL REFERENCES garantes(id) ON DELETE RESTRICT,
    PRIMARY KEY (contrato_id, garante_id)
);

CREATE TABLE IF NOT EXISTS actualizaciones (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    contrato_id    INTEGER NOT NULL REFERENCES contratos(id) ON DELETE CASCADE,
    fecha_vigencia TEXT NOT NULL,
    monto_nuevo    REAL NOT NULL,
    motivo         TEXT
);

CREATE TABLE IF NOT EXISTS pagos (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    contrato_id            INTEGER NOT NULL REFERENCES contratos(id) ON DELETE RESTRICT,
    periodo                TEXT NOT NULL,
    fecha_pago             TEXT NOT NULL,
    monto_alquiler         REAL NOT NULL,
    dias_mora              INTEGER NOT NULL DEFAULT 0,
    monto_mora             REAL NOT NULL DEFAULT 0,
    monto_total            REAL NOT NULL,
    metodo_pago            TEXT,
    numero_recibo          INTEGER NOT NULL,
    notas                  TEXT
);

CREATE TABLE IF NOT EXISTS liquidaciones (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    pago_id             INTEGER NOT NULL REFERENCES pagos(id) ON DELETE RESTRICT,
    fecha               TEXT NOT NULL,
    monto_alquiler      REAL NOT NULL,
    comision_porcentaje REAL NOT NULL,
    monto_comision      REAL NOT NULL,
    monto_neto          REAL NOT NULL,
    numero_comprobante  INTEGER NOT NULL,
    notas               TEXT
);
"#;
