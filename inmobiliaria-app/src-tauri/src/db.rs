use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio_postgres::Config as PgConfig;

#[derive(Debug, Serialize, Deserialize)]
struct ConfigGuardada {
    connection_string: String,
}

fn archivo_config(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("conexion.json")
}

/// Lee el connection string guardado en una instalación anterior, si existe.
pub fn leer_connection_string_guardado(data_dir: &std::path::Path) -> Option<String> {
    let contenido = std::fs::read_to_string(archivo_config(data_dir)).ok()?;
    let config: ConfigGuardada = serde_json::from_str(&contenido).ok()?;
    Some(config.connection_string)
}

pub fn guardar_connection_string(data_dir: &std::path::Path, connection_string: &str) -> Result<(), String> {
    let config = ConfigGuardada { connection_string: connection_string.to_string() };
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(archivo_config(data_dir), json).map_err(|e| e.to_string())
}

/// Arma un pool de conexiones contra Postgres (Supabase) y lo prueba con un
/// SELECT 1 antes de darlo por bueno, para detectar credenciales o red
/// incorrectas de entrada en vez de que fallen recién en el primer uso real.
pub async fn conectar(connection_string: &str) -> Result<Pool, String> {
    let pg_config: PgConfig = connection_string
        .parse()
        .map_err(|e| format!("El connection string no tiene un formato válido: {}", e))?;

    let tls_connector = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| format!("No se pudo inicializar TLS: {}", e))?;
    let tls = postgres_native_tls::MakeTlsConnector::new(tls_connector);

    let manager_config = ManagerConfig { recycling_method: RecyclingMethod::Fast };
    let manager = Manager::from_config(pg_config, tls, manager_config);
    let pool = Pool::builder(manager)
        .max_size(8)
        .build()
        .map_err(|e| format!("No se pudo crear el pool de conexiones: {}", e))?;

    let conn = pool.get().await.map_err(|e| {
        format!(
            "No se pudo conectar a la base de datos. Verificá el connection string y tu conexión a internet. Detalle: {}",
            e
        )
    })?;
    conn.simple_query("SELECT 1").await.map_err(|e| e.to_string())?;
    drop(conn);

    inicializar_esquema(&pool).await?;
    Ok(pool)
}

async fn inicializar_esquema(pool: &Pool) -> Result<(), String> {
    let conn = pool.get().await.map_err(|e| e.to_string())?;
    conn.batch_execute(SCHEMA).await.map_err(|e| e.to_string())?;
    migrar(&conn).await?;
    Ok(())
}

/// Agrega columnas nuevas a bases ya existentes, sin tocar los datos cargados.
async fn migrar(conn: &deadpool_postgres::Object) -> Result<(), String> {
    for tabla in ["propietarios", "inquilinos", "garantes"] {
        if !columna_existe(conn, tabla, "fecha_nacimiento").await? {
            conn.batch_execute(&format!("ALTER TABLE {} ADD COLUMN fecha_nacimiento TEXT", tabla))
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

async fn columna_existe(conn: &deadpool_postgres::Object, tabla: &str, columna: &str) -> Result<bool, String> {
    let fila = conn
        .query_opt(
            "SELECT 1 FROM information_schema.columns WHERE table_name = $1 AND column_name = $2",
            &[&tabla, &columna],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(fila.is_some())
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS usuarios (
    id              BIGSERIAL PRIMARY KEY,
    username        TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,
    nombre_completo TEXT NOT NULL,
    activo          BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE TABLE IF NOT EXISTS propietarios (
    id                BIGSERIAL PRIMARY KEY,
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
    id                BIGSERIAL PRIMARY KEY,
    nombre            TEXT NOT NULL,
    dni_cuit          TEXT,
    fecha_nacimiento  TEXT,
    telefono          TEXT,
    email             TEXT,
    direccion         TEXT,
    notas             TEXT
);

CREATE TABLE IF NOT EXISTS garantes (
    id                BIGSERIAL PRIMARY KEY,
    nombre            TEXT NOT NULL,
    dni_cuit          TEXT,
    fecha_nacimiento  TEXT,
    telefono          TEXT,
    email             TEXT,
    direccion         TEXT,
    notas             TEXT
);

CREATE TABLE IF NOT EXISTS inmuebles (
    id             BIGSERIAL PRIMARY KEY,
    propietario_id BIGINT NOT NULL REFERENCES propietarios(id) ON DELETE RESTRICT,
    direccion      TEXT NOT NULL,
    tipo           TEXT,
    superficie     DOUBLE PRECISION,
    ambientes      BIGINT,
    notas          TEXT
);

CREATE TABLE IF NOT EXISTS contratos (
    id                          BIGSERIAL PRIMARY KEY,
    inmueble_id                 BIGINT NOT NULL REFERENCES inmuebles(id) ON DELETE RESTRICT,
    inquilino_id                BIGINT NOT NULL REFERENCES inquilinos(id) ON DELETE RESTRICT,
    fecha_inicio                TEXT NOT NULL,
    fecha_fin                   TEXT NOT NULL,
    dia_pago                    BIGINT NOT NULL DEFAULT 10,
    monto_inicial                DOUBLE PRECISION NOT NULL,
    comision_porcentaje         DOUBLE PRECISION NOT NULL DEFAULT 0,
    tasa_mora_diaria            DOUBLE PRECISION NOT NULL DEFAULT 0,
    frecuencia_actualizacion_meses BIGINT NOT NULL DEFAULT 12,
    tipo_actualizacion          TEXT NOT NULL DEFAULT 'porcentaje_fijo',
    porcentaje_actualizacion    DOUBLE PRECISION NOT NULL DEFAULT 0,
    estado                      TEXT NOT NULL DEFAULT 'activo',
    notas                       TEXT
);

CREATE TABLE IF NOT EXISTS contrato_garantes (
    contrato_id BIGINT NOT NULL REFERENCES contratos(id) ON DELETE CASCADE,
    garante_id  BIGINT NOT NULL REFERENCES garantes(id) ON DELETE RESTRICT,
    PRIMARY KEY (contrato_id, garante_id)
);

CREATE TABLE IF NOT EXISTS actualizaciones (
    id             BIGSERIAL PRIMARY KEY,
    contrato_id    BIGINT NOT NULL REFERENCES contratos(id) ON DELETE CASCADE,
    fecha_vigencia TEXT NOT NULL,
    monto_nuevo    DOUBLE PRECISION NOT NULL,
    motivo         TEXT
);

CREATE TABLE IF NOT EXISTS pagos (
    id                     BIGSERIAL PRIMARY KEY,
    contrato_id            BIGINT NOT NULL REFERENCES contratos(id) ON DELETE RESTRICT,
    periodo                TEXT NOT NULL,
    fecha_pago             TEXT NOT NULL,
    monto_alquiler         DOUBLE PRECISION NOT NULL,
    dias_mora              BIGINT NOT NULL DEFAULT 0,
    monto_mora             DOUBLE PRECISION NOT NULL DEFAULT 0,
    monto_total            DOUBLE PRECISION NOT NULL,
    metodo_pago            TEXT,
    numero_recibo          BIGINT NOT NULL,
    notas                  TEXT
);

CREATE TABLE IF NOT EXISTS liquidaciones (
    id                  BIGSERIAL PRIMARY KEY,
    pago_id             BIGINT NOT NULL REFERENCES pagos(id) ON DELETE RESTRICT,
    fecha               TEXT NOT NULL,
    monto_alquiler      DOUBLE PRECISION NOT NULL,
    comision_porcentaje DOUBLE PRECISION NOT NULL,
    monto_comision      DOUBLE PRECISION NOT NULL,
    monto_neto          DOUBLE PRECISION NOT NULL,
    numero_comprobante  BIGINT NOT NULL,
    notas               TEXT
);
"#;
