use chrono::NaiveDate;
use serde_json::Value;

/// Id de la variable ICL en el catalogo de "Principales variables" del BCRA,
/// usado como respaldo si no se puede resolver dinamicamente por descripcion
/// (la API puede reordenar o renombrar variables con el tiempo).
pub const ICL_ID_FALLBACK: i64 = 40;

const BASE_URL: &str = "https://api.bcra.gob.ar/estadisticas/v4.0/monetarias";

#[derive(Debug, Clone)]
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

fn truncar(s: &str) -> String {
    if s.chars().count() > 600 {
        format!("{}...", s.chars().take(600).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Busca un campo por varias variantes de nombre (la API del BCRA no es
/// siempre consistente con mayusculas/minusculas entre endpoints/versiones).
fn campo<'a>(obj: &'a Value, nombres: &[&str]) -> Option<&'a Value> {
    nombres.iter().find_map(|n| obj.get(n))
}

fn campo_texto(obj: &Value, nombres: &[&str]) -> Option<String> {
    campo(obj, nombres).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn campo_numero(obj: &Value, nombres: &[&str]) -> Option<f64> {
    campo(obj, nombres).and_then(|v| v.as_f64())
}

/// Encuentra el arreglo de resultados sin importar como este envuelta la
/// respuesta. El catalogo de variables devuelve {"results": [{idVariable,
/// descripcion, ...}, ...]} directamente, pero la serie historica de una
/// variable devuelve {"results": [{..., "detalle": [{fecha, valor}, ...]}]}
/// (un objeto por variable consultada, con el historico adentro de
/// "detalle") — en ese caso hay que aplanar "detalle" para llegar a los
/// datos reales.
fn extraer_resultados(data: &Value) -> Option<Vec<Value>> {
    if let Some(arr) = data.as_array() {
        return Some(arr.clone());
    }
    let results = campo(data, &["results", "Results", "resultados"])?;

    if let Some(arr) = results.as_array() {
        let tiene_detalle = arr
            .iter()
            .any(|item| campo(item, &["detalle", "Detalle"]).and_then(|v| v.as_array()).is_some());
        if tiene_detalle {
            let mut planos = Vec::new();
            for item in arr {
                if let Some(detalle) = campo(item, &["detalle", "Detalle"]).and_then(|v| v.as_array()) {
                    planos.extend(detalle.iter().cloned());
                } else {
                    planos.push(item.clone());
                }
            }
            return Some(planos);
        }
        return Some(arr.clone());
    }

    campo(results, &["detalle", "Detalle"]).and_then(|v| v.as_array()).cloned()
}

pub fn cliente_http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("InmobiliariaApp/1.0")
        .build()
        .map_err(|e| format!("No se pudo inicializar el cliente HTTP: {}", e))
}

async fn fetch_json(client: &reqwest::Client, url: &str, query: &[(&str, String)]) -> Result<Value, String> {
    let resp = client.get(url).query(query).send().await.map_err(error_conexion)?;
    let status = resp.status();
    let texto = resp.text().await.map_err(error_conexion)?;
    if !status.is_success() {
        return Err(format!(
            "El BCRA respondió un error ({}) al consultar el ICL. Detalle: {}",
            status,
            truncar(&texto)
        ));
    }
    serde_json::from_str(&texto).map_err(|e| {
        format!(
            "No se pudo interpretar la respuesta del BCRA. Pasale este detalle a soporte para ajustarlo: {} — respuesta: {}",
            e,
            truncar(&texto)
        )
    })
}

/// Busca el id de variable del ICL por su descripcion en el catalogo del BCRA.
/// Si la busqueda falla (sin red, formato distinto, etc.) el llamador debe
/// usar ICL_ID_FALLBACK.
pub async fn id_variable_icl(client: &reqwest::Client) -> Result<i64, String> {
    let data = fetch_json(client, BASE_URL, &[]).await?;
    let resultados = extraer_resultados(&data).ok_or_else(|| {
        format!(
            "No se reconoció el formato del listado de variables del BCRA. Respuesta: {}",
            truncar(&data.to_string())
        )
    })?;

    for item in &resultados {
        let descripcion = campo_texto(item, &["descripcion", "Descripcion", "DESCRIPCION"]).unwrap_or_default();
        let d = descripcion.to_uppercase();
        if d.contains("CONTRATOS DE LOCACION") || d.contains("CONTRATOS DE LOCACIÓN") || d.contains("(ICL)") {
            if let Some(id) = campo(item, &["idVariable", "IdVariable", "id_variable"]).and_then(|v| v.as_i64()) {
                return Ok(id);
            }
        }
    }
    Err("No se encontró la variable ICL en el listado del BCRA".to_string())
}

/// Valor de la serie en la fecha pedida, o el mas reciente disponible antes de
/// esa fecha (el ICL no publica fines de semana/feriados largos).
pub async fn valor_en_o_antes(client: &reqwest::Client, id_variable: i64, fecha: NaiveDate) -> Result<DatoSerie, String> {
    let desde = fecha - chrono::Duration::days(20);
    let url = format!("{}/{}", BASE_URL, id_variable);
    let query = [
        ("desde", desde.format("%Y-%m-%d").to_string()),
        ("hasta", fecha.format("%Y-%m-%d").to_string()),
    ];
    let data = fetch_json(client, &url, &query).await?;
    let resultados = extraer_resultados(&data).ok_or_else(|| {
        format!(
            "No se reconoció el formato de la serie de ICL del BCRA. Respuesta: {}",
            truncar(&data.to_string())
        )
    })?;

    let mut mejor: Option<DatoSerie> = None;
    for item in &resultados {
        let fecha_item = campo_texto(item, &["fecha", "Fecha"]);
        let valor_item = campo_numero(item, &["valor", "Valor"]);
        if let (Some(f), Some(v)) = (fecha_item, valor_item) {
            let es_mejor = mejor.as_ref().map(|m| f > m.fecha).unwrap_or(true);
            if es_mejor {
                mejor = Some(DatoSerie { fecha: f, valor: v });
            }
        }
    }
    mejor.ok_or_else(|| {
        format!(
            "No se encontraron valores de ICL cerca del {}. Respuesta: {}",
            fecha.format("%d/%m/%Y"),
            truncar(&data.to_string())
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JSON real devuelto por api.bcra.gob.ar/estadisticas/v4.0/monetarias/40
    /// (capturado de un error en producción): los valores vienen agrupados
    /// bajo results[0].detalle, no directamente en results.
    const RESPUESTA_SERIE_REAL: &str = r#"{
        "metadata": {"resultset": {"count": 14, "limit": 1000, "offset": 0}},
        "results": [{
            "detalle": [
                {"fecha": "2026-10-01", "valor": 36.54},
                {"fecha": "2026-09-30", "valor": 36.52},
                {"fecha": "2026-09-29", "valor": 36.5},
                {"fecha": "2026-09-28", "valor": 36.48},
                {"fecha": "2026-09-27", "valor": 36.45},
                {"fecha": "2026-09-26", "valor": 36.43},
                {"fecha": "2026-09-25", "valor": 36.41},
                {"fecha": "2026-09-24", "valor": 36.39},
                {"fecha": "2026-09-23", "valor": 36.37},
                {"fecha": "2026-09-22", "valor": 36.34},
                {"fecha": "2026-09-21", "valor": 36.32},
                {"fecha": "2026-09-20", "valor": 36.3},
                {"fecha": "2026-09-19", "valor": 36.28},
                {"fecha": "2026-09-18", "valor": 36.25}
            ]
        }]
    }"#;

    #[test]
    fn extrae_el_valor_mas_reciente_de_una_serie_anidada_en_detalle() {
        let data: Value = serde_json::from_str(RESPUESTA_SERIE_REAL).unwrap();
        let resultados = extraer_resultados(&data).expect("debe reconocer el formato con 'detalle'");
        assert_eq!(resultados.len(), 14);

        let mut mejor: Option<DatoSerie> = None;
        for item in &resultados {
            let f = campo_texto(item, &["fecha", "Fecha"]).unwrap();
            let v = campo_numero(item, &["valor", "Valor"]).unwrap();
            if mejor.as_ref().map(|m| f > m.fecha).unwrap_or(true) {
                mejor = Some(DatoSerie { fecha: f, valor: v });
            }
        }
        let mejor = mejor.unwrap();
        assert_eq!(mejor.fecha, "2026-10-01");
        assert_eq!(mejor.valor, 36.54);
    }

    #[test]
    fn reconoce_un_listado_plano_de_variables() {
        let data: Value = serde_json::from_str(
            r#"{"results": [
                {"idVariable": 1, "descripcion": "Otra variable"},
                {"idVariable": 40, "descripcion": "Índice para Contratos de Locación (ICL) - Ley 27.551"}
            ]}"#,
        )
        .unwrap();
        let resultados = extraer_resultados(&data).expect("debe reconocer el listado plano");
        assert_eq!(resultados.len(), 2);
        assert_eq!(campo_texto(&resultados[1], &["descripcion"]).unwrap(), "Índice para Contratos de Locación (ICL) - Ley 27.551");
    }
}
