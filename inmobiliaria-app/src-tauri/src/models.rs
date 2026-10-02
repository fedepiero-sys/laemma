use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Propietario {
    pub id: Option<i64>,
    pub nombre: String,
    pub dni_cuit: Option<String>,
    pub telefono: Option<String>,
    pub email: Option<String>,
    pub direccion: Option<String>,
    pub datos_bancarios: Option<String>,
    pub notas: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Inquilino {
    pub id: Option<i64>,
    pub nombre: String,
    pub dni_cuit: Option<String>,
    pub telefono: Option<String>,
    pub email: Option<String>,
    pub direccion: Option<String>,
    pub notas: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Garante {
    pub id: Option<i64>,
    pub nombre: String,
    pub dni_cuit: Option<String>,
    pub telefono: Option<String>,
    pub email: Option<String>,
    pub direccion: Option<String>,
    pub notas: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Inmueble {
    pub id: Option<i64>,
    pub propietario_id: i64,
    pub direccion: String,
    pub tipo: Option<String>,
    pub superficie: Option<f64>,
    pub ambientes: Option<i64>,
    pub notas: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InmuebleDetallado {
    pub id: i64,
    pub propietario_id: i64,
    pub propietario_nombre: String,
    pub direccion: String,
    pub tipo: Option<String>,
    pub superficie: Option<f64>,
    pub ambientes: Option<i64>,
    pub notas: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Contrato {
    pub id: Option<i64>,
    pub inmueble_id: i64,
    pub inquilino_id: i64,
    pub garante_ids: Vec<i64>,
    pub fecha_inicio: String,
    pub fecha_fin: String,
    pub dia_pago: i64,
    pub monto_inicial: f64,
    pub comision_porcentaje: f64,
    pub tasa_mora_diaria: f64,
    pub frecuencia_actualizacion_meses: i64,
    pub tipo_actualizacion: String,
    pub porcentaje_actualizacion: f64,
    pub estado: String,
    pub notas: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContratoDetallado {
    pub id: i64,
    pub inmueble_id: i64,
    pub inmueble_direccion: String,
    pub inquilino_id: i64,
    pub inquilino_nombre: String,
    pub propietario_id: i64,
    pub propietario_nombre: String,
    pub garantes: Vec<String>,
    pub garante_ids: Vec<i64>,
    pub fecha_inicio: String,
    pub fecha_fin: String,
    pub dia_pago: i64,
    pub monto_inicial: f64,
    pub monto_vigente: f64,
    pub comision_porcentaje: f64,
    pub tasa_mora_diaria: f64,
    pub frecuencia_actualizacion_meses: i64,
    pub tipo_actualizacion: String,
    pub porcentaje_actualizacion: f64,
    pub proxima_actualizacion: Option<String>,
    pub estado: String,
    pub notas: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Actualizacion {
    pub id: Option<i64>,
    pub contrato_id: i64,
    pub fecha_vigencia: String,
    pub monto_nuevo: f64,
    pub motivo: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NuevoPago {
    pub contrato_id: i64,
    pub periodo: String,
    pub fecha_pago: String,
    pub metodo_pago: Option<String>,
    pub notas: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Pago {
    pub id: i64,
    pub contrato_id: i64,
    pub periodo: String,
    pub fecha_pago: String,
    pub monto_alquiler: f64,
    pub dias_mora: i64,
    pub monto_mora: f64,
    pub monto_total: f64,
    pub metodo_pago: Option<String>,
    pub numero_recibo: i64,
    pub notas: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReciboCompleto {
    pub pago: Pago,
    pub inquilino_nombre: String,
    pub inmueble_direccion: String,
    pub propietario_nombre: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Liquidacion {
    pub id: i64,
    pub pago_id: i64,
    pub fecha: String,
    pub monto_alquiler: f64,
    pub comision_porcentaje: f64,
    pub monto_comision: f64,
    pub monto_neto: f64,
    pub numero_comprobante: i64,
    pub notas: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ComprobanteCompleto {
    pub liquidacion: Liquidacion,
    pub periodo: String,
    pub propietario_nombre: String,
    pub propietario_datos_bancarios: Option<String>,
    pub inmueble_direccion: String,
    pub inquilino_nombre: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeudaContrato {
    pub contrato_id: i64,
    pub inmueble_direccion: String,
    pub inquilino_nombre: String,
    pub periodos_impagos: Vec<String>,
    pub monto_adeudado: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VencimientoContrato {
    pub contrato_id: i64,
    pub inmueble_direccion: String,
    pub inquilino_nombre: String,
    pub propietario_nombre: String,
    pub fecha_fin: String,
    pub dias_restantes: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActualizacionPendiente {
    pub contrato_id: i64,
    pub inmueble_direccion: String,
    pub inquilino_nombre: String,
    pub fecha_prevista: String,
    pub dias_restantes: i64,
    pub monto_vigente: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResumenDashboard {
    pub deudas: Vec<DeudaContrato>,
    pub vencimientos: Vec<VencimientoContrato>,
    pub actualizaciones_pendientes: Vec<ActualizacionPendiente>,
    pub total_contratos_activos: i64,
    pub total_adeudado: f64,
}
