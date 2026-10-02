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

/// La mayoría de los errores de red/TLS envuelven un error más específico
/// adentro (ej. "error performing TLS handshake" envuelve el motivo real:
/// certificado vencido, hostname que no matchea, etc). Mostrar solo el
/// mensaje de más afuera oculta esa info justo cuando más hace falta para
/// diagnosticar — así que concatenamos toda la cadena.
fn detalle_completo(e: &(dyn std::error::Error + 'static)) -> String {
    let mut partes = vec![e.to_string()];
    let mut actual = e.source();
    while let Some(err) = actual {
        partes.push(err.to_string());
        actual = err.source();
    }
    partes.join(" ← ")
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

/// Las páginas de Supabase muestran el connection string como una línea de
/// archivo .env, por ejemplo `DATABASE_URL="postgresql://..."`. Si alguien
/// copia esa línea entera (con el nombre de la variable y las comillas) en
/// vez de solo la URL, el parser lo rechaza entero. Si el texto antes del
/// primer "=" es un nombre de variable válido (sin "://", ":" ni "/") y lo
/// que sigue empieza con comillas o con "postgres", asumimos que es ese
/// prefijo y lo sacamos. Después sacamos comillas que envuelvan todo el
/// string, y espacios/saltos de línea de copiar y pegar.
fn quitar_prefijo_de_env(connection_string: &str) -> &str {
    let s = connection_string.trim();
    let s = match s.split_once('=') {
        Some((nombre, resto))
            if !nombre.is_empty()
                && nombre.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && (resto.trim_start().starts_with(['"', '\'']) || resto.trim_start().starts_with("postgres")) =>
        {
            resto.trim()
        }
        _ => s,
    };
    fn quitar_comillas(s: &str, comilla: char) -> &str {
        if s.len() >= 2 && s.starts_with(comilla) && s.ends_with(comilla) {
            &s[1..s.len() - 1]
        } else {
            s
        }
    }
    quitar_comillas(quitar_comillas(s, '"'), '\'').trim()
}

/// El parser de tokio-postgres no reconoce el parámetro "pgbouncer=true"
/// (presente en el string del pooler de Supabase en modo transacción) y
/// rechaza el connection string entero por eso. Lo sacamos puntualmente,
/// conservando cualquier otro parámetro válido (sslmode, etc.) que sí
/// entienda, en vez de descartar toda la query string.
fn quitar_query_params(connection_string: &str) -> String {
    let Some((base, query)) = connection_string.split_once('?') else {
        return connection_string.to_string();
    };
    let params: Vec<&str> = query.split('&').filter(|p| !p.starts_with("pgbouncer=")).collect();
    if params.is_empty() {
        base.to_string()
    } else {
        format!("{}?{}", base, params.join("&"))
    }
}

/// Arma un pool de conexiones contra Postgres (Supabase) y lo prueba con un
/// SELECT 1 antes de darlo por bueno, para detectar credenciales o red
/// incorrectas de entrada en vez de que fallen recién en el primer uso real.
pub async fn conectar(connection_string: &str) -> Result<Pool, String> {
    let limpio = quitar_query_params(quitar_prefijo_de_env(connection_string));
    let pg_config: PgConfig = limpio
        .parse()
        .map_err(|e| format!("El connection string no tiene un formato válido: {}", e))?;

    // rustls en vez de native-tls: native-tls usa schannel en Windows, que en
    // algunas PCs falla el handshake con Postgres de forma genérica y sin
    // detalle ("error performing TLS handshake") por motivos específicos de
    // esa máquina (caché de certificados intermedios, políticas locales,
    // etc). rustls trae sus propias raíces de confianza (Mozilla, vía
    // webpki-roots) y no depende del almacén de certificados del sistema
    // operativo, evitando esa clase de fallas.
    let mut raices = rustls::RootCertStore::empty();
    raices.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(raices)
        .with_no_client_auth();
    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_config);

    let manager_config = ManagerConfig { recycling_method: RecyclingMethod::Fast };
    let manager = Manager::from_config(pg_config, tls, manager_config);
    let pool = Pool::builder(manager)
        .max_size(8)
        .build()
        .map_err(|e| format!("No se pudo crear el pool de conexiones: {}", e))?;

    let conn = pool.get().await.map_err(|e| {
        format!(
            "No se pudo conectar a la base de datos. Verificá el connection string y tu conexión a internet. Detalle: {}",
            detalle_completo(&e)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Los dos formatos de connection string que Supabase muestra en
    /// "Connect → ORM": el pooler de sesión (sin query params, el que
    /// recomendamos) y el pooler de transacción (con "?pgbouncer=true", que
    /// alguien puede pegar igual porque también dice "DATABASE_URL"). Ambos
    /// tienen que poder parsearse, y cualquier otro parámetro (sslmode, etc.)
    /// que venga junto con pgbouncer tiene que conservarse.
    #[test]
    fn parsea_ambos_formatos_del_pooler_de_supabase() {
        let sesion = "postgresql://postgres.abcxyz:p%40ss&w0rd@aws-0-us-east-1.pooler.supabase.com:5432/postgres";
        let transaccion = "postgresql://postgres.abcxyz:p%40ss&w0rd@aws-0-us-east-1.pooler.supabase.com:6543/postgres?pgbouncer=true";
        let con_otro_param = "postgresql://postgres:x@localhost:5432/postgres?pgbouncer=true&sslmode=disable";

        quitar_query_params(sesion).parse::<PgConfig>().expect("el pooler de sesión debe parsear");
        quitar_query_params(transaccion).parse::<PgConfig>().expect("el pooler de transacción (con query params) debe parsear igual");

        let limpio = quitar_query_params(con_otro_param);
        assert!(limpio.contains("sslmode=disable"), "no debe borrar otros parámetros válidos: {}", limpio);
        assert!(!limpio.contains("pgbouncer"), "debe borrar específicamente pgbouncer: {}", limpio);
    }

    /// Supabase (y cualquier panel de "variables de entorno") muestra el
    /// connection string como una línea tipo .env: `NOMBRE="valor"`. Si se
    /// copia la línea entera en vez de solo el valor, tenemos que poder
    /// rescatar igual el connection string real.
    #[test]
    fn tolera_pegar_la_linea_completa_de_env() {
        let url = "postgresql://postgres.abc:pass@host:5432/postgres";

        assert_eq!(quitar_prefijo_de_env(&format!(r#"DATABASE_URL="{url}""#)), url);
        assert_eq!(quitar_prefijo_de_env(&format!("DIRECT_URL={url}")), url);
        assert_eq!(quitar_prefijo_de_env(&format!("DIRECT_URL='{url}'")), url);
        assert_eq!(quitar_prefijo_de_env(&format!("  {url}  \n")), url);
        assert_eq!(quitar_prefijo_de_env(url), url, "un connection string normal no debe tocarse");

        // Un connection string normal (sin prefijo) puede tener "=" en los
        // query params (ej. sslmode=require) y no debe confundirse con un
        // prefijo de variable.
        let con_query = "postgresql://postgres:pass@host:5432/postgres?sslmode=require";
        assert_eq!(quitar_prefijo_de_env(con_query), con_query);

        // Caso real que vimos: la línea completa, con query params y todo.
        let con_prefijo_y_query = format!(r#"DATABASE_URL="{con_query}""#);
        assert_eq!(quitar_prefijo_de_env(&con_prefijo_y_query), con_query);
    }
}
