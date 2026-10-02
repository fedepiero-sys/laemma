use chrono::NaiveDate;
use serde::Deserialize;

/// Id de la variable ICL en el catalogo de "Principales variables" del BCRA,
/// usado como respaldo si no se puede resolver dinamicamente por descripcion
/// (la API puede reordenar o renombrar variables con el tiempo).
pub const ICL_ID_FALLBACK: i64 = 40;

const BASE_URL: &str = "https://api.bcra.gob.ar/estadisticas/v4.0/monetarias";

#[derive(Debug, Deserialize)]
struct ListadoResponse {
    results: Vec<VariableInfo>,
}

#[derive(Debug, Deserialize)]
struct VariableInfo {
    #[serde(rename = "idVariable")]
    id_variable: i64,
    descripcion: String,
}

#[derive(Debug, Deserialize)]
struct SerieResponse {
    results: Vec<DatoSerie>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatoSerie {
    pub fecha: String,
    pub valor: f64,
}

fn error_conexion<E: std::fmt::Display>(e: E) -> String {
    format!(
        "No se pudo conectar con la API del BCRA para obtener el ICL. Verificá tu conexión a internet. Detalle: {}",
        e
    )
}

pub fn cliente_http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("InmobiliariaApp/1.0")
        .build()
        .map_err(|e| format!("No se pudo inicializar el cliente HTTP: {}", e))
}

/// Busca el id de variable del ICL por su descripcion en el catalogo del BCRA.
/// Si la busqueda falla (sin red, formato distinto, etc.) el llamador debe
/// usar ICL_ID_FALLBACK.
pub async fn id_variable_icl(client: &reqwest::Client) -> Result<i64, String> {
    let resp = client.get(BASE_URL).send().await.map_err(error_conexion)?;
    let resp = resp.error_for_status().map_err(error_conexion)?;
    let data: ListadoResponse = resp.json().await.map_err(error_conexion)?;
    data.results
        .into_iter()
        .find(|v| {
            let d = v.descripcion.to_uppercase();
            d.contains("CONTRATOS DE LOCACION") || d.contains("CONTRATOS DE LOCACIÓN") || d.contains("(ICL)")
        })
        .map(|v| v.id_variable)
        .ok_or_else(|| "No se encontró la variable ICL en el listado del BCRA".to_string())
}

/// Valor de la serie en la fecha pedida, o el mas reciente disponible antes de
/// esa fecha (el ICL no publica fines de semana/feriados largos).
pub async fn valor_en_o_antes(client: &reqwest::Client, id_variable: i64, fecha: NaiveDate) -> Result<DatoSerie, String> {
    let desde = fecha - chrono::Duration::days(20);
    let url = format!("{}/{}", BASE_URL, id_variable);
    let resp = client
        .get(&url)
        .query(&[
            ("desde", desde.format("%Y-%m-%d").to_string()),
            ("hasta", fecha.format("%Y-%m-%d").to_string()),
        ])
        .send()
        .await
        .map_err(error_conexion)?;
    let resp = resp.error_for_status().map_err(error_conexion)?;
    let data: SerieResponse = resp.json().await.map_err(error_conexion)?;
    data.results
        .into_iter()
        .max_by(|a, b| a.fecha.cmp(&b.fecha))
        .ok_or_else(|| format!("No se encontraron valores de ICL cerca del {}", fecha.format("%d/%m/%Y")))
}
