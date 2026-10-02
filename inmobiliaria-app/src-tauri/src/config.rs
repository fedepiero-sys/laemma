use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupabaseConfig {
    pub url: String,
    pub anon_key: String,
}

fn archivo_config(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("conexion.json")
}

/// Lee la configuración guardada en una instalación anterior, si existe.
pub fn leer_guardada(data_dir: &std::path::Path) -> Option<SupabaseConfig> {
    let contenido = std::fs::read_to_string(archivo_config(data_dir)).ok()?;
    serde_json::from_str(&contenido).ok()
}

pub fn guardar(data_dir: &std::path::Path, config: &SupabaseConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(archivo_config(data_dir), json).map_err(|e| e.to_string())
}

/// Las páginas de Supabase muestran a veces la URL o la key como una línea de
/// archivo .env (ej. `NEXT_PUBLIC_SUPABASE_URL="https://..."`). Si alguien
/// copia esa línea entera, le sacamos el nombre de la variable y las comillas
/// para no obligar a que esté perfecto.
pub fn limpiar(valor: &str) -> String {
    let s = valor.trim();
    let s = match s.split_once('=') {
        Some((nombre, resto))
            if !nombre.is_empty()
                && nombre.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && (resto.trim_start().starts_with(['"', '\'']) || resto.trim_start().starts_with("http") || resto.trim_start().starts_with("ey")) =>
        {
            resto.trim()
        }
        _ => s,
    };
    let s = s.trim_matches(|c| c == '"' || c == '\'');
    s.trim().trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limpia_url_pegada_como_linea_de_env() {
        assert_eq!(limpiar(r#"NEXT_PUBLIC_SUPABASE_URL="https://abc.supabase.co""#), "https://abc.supabase.co");
        assert_eq!(limpiar("SUPABASE_URL=https://abc.supabase.co"), "https://abc.supabase.co");
        assert_eq!(limpiar("  https://abc.supabase.co/  "), "https://abc.supabase.co");
    }

    #[test]
    fn limpia_anon_key_pegada_como_linea_de_env() {
        let key = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJyb2xlIjoiYW5vbiJ9.firma";
        assert_eq!(limpiar(&format!(r#"NEXT_PUBLIC_SUPABASE_ANON_KEY="{}""#, key)), key);
        assert_eq!(limpiar(key), key);
    }
}
