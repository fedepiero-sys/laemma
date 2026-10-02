# Inmobiliaria App

Programa de escritorio para gestión de alquileres: propietarios, inquilinos,
garantes, inmuebles, contratos, ingresos (recibos de alquiler) y egresos
(comprobantes de comisión), con un tablero de control de deudas y
vencimientos.

Hecho con [Tauri](https://tauri.app) (Rust + WebView del sistema) y una base
de datos Postgres compartida (pensada para [Supabase](https://supabase.com),
aunque funciona contra cualquier Postgres). Todas las PCs que se conecten al
mismo proyecto ven y cargan los mismos datos.

## Qué incluye

- **Propietarios, Inquilinos, Garantes**: fichas con datos de contacto y,
  para propietarios, datos bancarios para transferencias.
- **Inmuebles**: dirección, tipo, superficie, ambientes, vinculados a un
  propietario.
- **Contratos**: inmueble, inquilino, garante(s), fecha de inicio y
  vencimiento, día de pago, comisión de la inmobiliaria, tasa de interés
  por mora (% diario), y esquema de actualización de alquiler (porcentaje
  fijo, ICL, IPC, Casa Propia u otro) con su frecuencia. Cada contrato
  lleva un historial de actualizaciones de monto.
- **Ingresos**: registrar el pago de un inquilino genera automáticamente
  un recibo numerado, calculando el alquiler vigente a esa fecha y el
  interés por mora según los días de atraso.
- **Egresos**: a partir de un recibo cobrado, se genera un comprobante de
  liquidación al propietario, descontando la comisión de la inmobiliaria,
  con los datos bancarios para la transferencia.
- **Tablero de control**: inquilinos con deuda (y cuánto deben, con mora
  calculada a hoy), contratos próximos a vencer, y próximas actualizaciones
  de alquiler pendientes.
- Los recibos y comprobantes se pueden imprimir o guardar como PDF desde la
  propia aplicación.
- **Login**: la primera vez que se abre la app pide crear un usuario
  (nombre, usuario y contraseña); de ahí en adelante pide usuario y
  contraseña para entrar. Se pueden cargar más usuarios desde la sección
  "Usuarios" ya logueado — las cuentas son compartidas entre todas las PCs
  conectadas a la misma base.

## Varias personas, varias PCs

La primera vez que se abre la app en una PC, pide el connection string de
Postgres (ver "Puesta en marcha con Supabase" abajo) y lo guarda en
`%APPDATA%\com.inmobiliaria.app\conexion.json`. Cualquier PC que se conecte
con el mismo connection string ve y carga los mismos contratos, pagos,
usuarios, etc. — no hace falta configurar nada más por PC.

## Puesta en marcha con Supabase

1. Crear una cuenta y un proyecto gratis en [supabase.com](https://supabase.com).
2. En el proyecto, ir a **Connect → ORM** y copiar el valor de `DIRECT_URL`
   (connection string del **pooler en modo sesión**, puerto `5432`, host
   terminado en `pooler.supabase.com`). **No usar**:
   - la pestaña "Direct connection": esa conexión es IPv6-only en el plan
     gratis, y muchas redes (incluidas varias de Argentina) no tienen salida
     IPv6;
   - el `DATABASE_URL` (pooler en modo **transacción**, puerto `6543`, con
     `?pgbouncer=true`): no soporta bien las consultas preparadas que usa
     esta app, puede dar errores intermitentes raros bajo uso concurrente.
3. Pegar ese string en la app la primera vez que se abra en cada PC. El
   esquema de tablas se crea solo en el primer connect — no hace falta
   correr ningún SQL a mano.
4. El primer usuario que se crea queda disponible para cualquier otra PC que
   se conecte después con el mismo string; desde la sección "Usuarios" se
   pueden cargar las cuentas de todo el equipo.

El connection string incluye la contraseña de la base en texto plano — se
guarda localmente en cada PC (no se sube a ningún lado), lo mismo que
cualquier otro programa de escritorio que se conecta a una base remota. Si
en algún momento se quiere invalidar el acceso, se puede rotar la
contraseña desde el panel de Supabase (Settings → Database).

## Requisitos para compilar

- [Node.js](https://nodejs.org/) 18 o superior
- [Rust](https://www.rust-lang.org/tools/install) (estable)
- En Windows: Tauri usa el WebView2 del sistema (viene incluido en Windows
  10/11 actualizados) y necesita las "Build Tools for Visual Studio" con el
  workload de C++ (se instalan automáticamente si faltan al compilar, o
  desde https://visualstudio.microsoft.com/visual-cpp-build-tools/).

## Cómo generar el instalador para Windows

Desde una PC con Windows (o siguiendo la guía de compilación cruzada de
Tauri si se compila desde otro sistema):

```bash
npm install
npm run tauri build
```

Al finalizar, los instaladores quedan en:

- `src-tauri/target/release/bundle/nsis/Inmobiliaria App_0.1.0_x64-setup.exe`
- `src-tauri/target/release/bundle/msi/Inmobiliaria App_0.1.0_x64_en-US.msi`

Cualquiera de los dos instala el programa normalmente en la PC (acceso
directo, desinstalador, etc.). Los datos viven en Postgres (Supabase), no en
la PC, así que reinstalar o actualizar la aplicación no los afecta; lo único
que queda guardado localmente es el connection string de conexión (ver
"Varias personas, varias PCs").

## Desarrollo

```bash
npm install
npm run tauri dev
```

Esto abre la aplicación en una ventana con recarga automática del frontend
(`src/`). El código de la base de datos y la lógica de negocio están en
`src-tauri/src/` (Rust): `db.rs` (esquema Postgres y conexión), `models.rs`
(estructuras de datos) y `commands.rs` (comandos que usa la interfaz,
incluyendo el cálculo de mora, montos vigentes y el tablero de control).

`commands.rs` incluye un test de integración (`cargo test --lib
tests_postgres`) que corre contra un Postgres real y valida el esquema y las
consultas más sensibles (altas con `RETURNING`, `ON CONFLICT`, joins del
contrato, numeración secuencial, violación de usuario único). Se salta solo
si no está definida la variable `TEST_DATABASE_URL`.

## Notas sobre el cálculo de mora y actualizaciones

- La **mora** se calcula como `alquiler vigente × (tasa diaria % / 100) ×
  días de atraso`, tomando como vencimiento el "día de pago" configurado en
  el contrato para el período que se está abonando.
- El **monto vigente** de un contrato en una fecha dada es el monto inicial,
  salvo que exista una actualización registrada con fecha de vigencia
  anterior o igual a esa fecha (se usa la más reciente).
- La **próxima actualización** se calcula sumando la frecuencia configurada
  (en meses) a la fecha de la última actualización registrada, o a la fecha
  de inicio del contrato si todavía no hubo ninguna.

## Actualización automática por índice ICL (BCRA)

Para contratos con tipo de actualización "ICL", el tablero de control agrega
una columna con un botón que consulta en vivo la API pública del BCRA
(`api.bcra.gob.ar/estadisticas/v4.0/monetarias`, variable ICL):

- **Mientras falta más de un mes** para la actualización: el botón "Estimar
  con ICL" compara el valor del índice de hoy contra el valor que tenía
  cuando se fijó el monto actual, y muestra un porcentaje y monto
  **aproximados** (el valor real todavía no existe, porque corresponde a una
  fecha futura).
- **El día de la actualización (o después)**: el botón "Traer valor real ICL"
  consulta el valor del índice publicado exactamente para esa fecha y
  calcula el monto definitivo. Desde ahí se puede aplicar directamente con
  un clic, lo que registra la actualización del contrato sin tipear nada a
  mano.

Esto requiere que la PC tenga conexión a internet en el momento de
consultar; si no hay conexión o el BCRA no responde, se muestra un aviso y
un botón para reintentar. El id de la variable ICL se busca por nombre en el
catálogo del BCRA en cada consulta (por si cambiara con el tiempo); si esa
búsqueda falla, se usa como respaldo el id conocido actualmente (40).
