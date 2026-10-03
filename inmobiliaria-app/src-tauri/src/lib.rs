mod commands;
mod config;
mod icl;
mod models;
mod supabase;

use commands::DbState;
use tauri::Manager;
use tokio::sync::RwLock;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            app.manage(DbState(RwLock::new(None)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::hay_configuracion_conexion,
            commands::configurar_conexion,
            commands::obtener_version,
            commands::revisar_actualizaciones,
            commands::maximizar_ventana,
            commands::minimizar_ventana,
            commands::cerrar_ventana,
            commands::leer_credenciales_guardadas,
            commands::guardar_credenciales,
            commands::borrar_credenciales,
            commands::get_propietarios,
            commands::guardar_propietario,
            commands::eliminar_propietario,
            commands::get_inquilinos,
            commands::guardar_inquilino,
            commands::eliminar_inquilino,
            commands::get_garantes,
            commands::guardar_garante,
            commands::eliminar_garante,
            commands::get_inmuebles,
            commands::guardar_inmueble,
            commands::eliminar_inmueble,
            commands::get_contratos,
            commands::guardar_contrato,
            commands::eliminar_contrato,
            commands::get_actualizaciones,
            commands::agregar_actualizacion,
            commands::eliminar_actualizacion,
            commands::get_pagos,
            commands::registrar_pago,
            commands::eliminar_pago,
            commands::get_recibo,
            commands::get_liquidaciones,
            commands::generar_liquidacion,
            commands::eliminar_liquidacion,
            commands::get_comprobante,
            commands::get_dashboard,
            commands::estimar_actualizacion_icl,
            commands::confirmar_actualizacion_icl,
            commands::hay_usuarios,
            commands::get_usuarios,
            commands::crear_usuario,
            commands::eliminar_usuario,
            commands::iniciar_sesion,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
