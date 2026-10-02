-- Inmobiliaria App — esquema de base de datos para Supabase.
--
-- Se corre UNA SOLA VEZ, pegando todo este archivo en Supabase → SQL Editor
-- → New query → Run. Es seguro volver a correrlo si hace falta (no borra
-- datos ni duplica nada: las tablas se crean solo si no existen, y los
-- permisos se re-otorgan sin problema).
--
-- Por qué hace falta este paso manual: la app habla con Supabase por su API
-- REST (PostgREST), que no permite crear tablas por HTTP por motivos de
-- seguridad — solo lee y escribe filas de tablas que ya existen. Antes la
-- app se conectaba directo a Postgres y podía crear las tablas sola, pero
-- esa conexión directa chocaba con el certificado propio de Supabase y
-- fallaba en varias PCs Windows; por eso se pasó a la API REST, que es la
-- misma forma en que se conectan la mayoría de las apps hechas con Supabase.

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
    id                              BIGSERIAL PRIMARY KEY,
    inmueble_id                     BIGINT NOT NULL REFERENCES inmuebles(id) ON DELETE RESTRICT,
    inquilino_id                    BIGINT NOT NULL REFERENCES inquilinos(id) ON DELETE RESTRICT,
    fecha_inicio                    TEXT NOT NULL,
    fecha_fin                       TEXT NOT NULL,
    dia_pago                        BIGINT NOT NULL DEFAULT 10,
    monto_inicial                   DOUBLE PRECISION NOT NULL,
    comision_porcentaje             DOUBLE PRECISION NOT NULL DEFAULT 0,
    tasa_mora_diaria                DOUBLE PRECISION NOT NULL DEFAULT 0,
    frecuencia_actualizacion_meses  BIGINT NOT NULL DEFAULT 12,
    tipo_actualizacion              TEXT NOT NULL DEFAULT 'porcentaje_fijo',
    porcentaje_actualizacion        DOUBLE PRECISION NOT NULL DEFAULT 0,
    estado                          TEXT NOT NULL DEFAULT 'activo',
    notas                           TEXT
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
    id              BIGSERIAL PRIMARY KEY,
    contrato_id     BIGINT NOT NULL REFERENCES contratos(id) ON DELETE RESTRICT,
    periodo         TEXT NOT NULL,
    fecha_pago      TEXT NOT NULL,
    monto_alquiler  DOUBLE PRECISION NOT NULL,
    dias_mora       BIGINT NOT NULL DEFAULT 0,
    monto_mora      DOUBLE PRECISION NOT NULL DEFAULT 0,
    monto_total     DOUBLE PRECISION NOT NULL,
    metodo_pago     TEXT,
    numero_recibo   BIGINT NOT NULL,
    notas           TEXT
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

-- La app usa la clave "anon" de Supabase para leer y escribir, sin Row Level
-- Security (como el resto de los proyectos propios) — por eso hay que darle
-- permiso explícito a los roles "anon" y "authenticated" sobre estas tablas;
-- Supabase no lo hace solo para tablas nuevas.
GRANT USAGE ON SCHEMA public TO anon, authenticated;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO anon, authenticated;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO anon, authenticated;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO anon, authenticated;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT ON SEQUENCES TO anon, authenticated;
