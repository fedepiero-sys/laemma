mod commands;
mod db;
mod icl;
mod models;

use commands::DbState;
use std::sync::Mutex;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("No se pudo resolver el directorio de datos de la aplicacion");
            std::fs::create_dir_all(&data_dir).expect("No se pudo crear el directorio de datos");
            let db_path = data_dir.join("inmobiliaria.db");
            let conn = db::connect(&db_path);
            app.manage(DbState(Mutex::new(conn)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
