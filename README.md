# db-monitor

Perro guardián (Rust, pensado para Windows 10/11) que cada N segundos comprueba que una tabla siga recibiendo filas y avisa cuando deja de recibirlas. Escribe en logs diarios. Soporta **PostgreSQL**, **SQLite** y **Oracle**. Se reconecta sola ante caídas y se apaga con Ctrl-C.

## Uso

```
db-monitor.exe [ruta\a\config.toml] [--tarea]
```

- Se lanza a mano (doble clic) o, en producción, como tarea programada (ver [Arranque automático](#arranque-automático-windows)). No abre consola: solo aparece un icono en la bandeja del sistema (en macOS, en la barra de menú).
- Sin argumento busca `config.toml` **junto al `.exe`**.
- `logs/` se crea relativo a la carpeta del `config.toml`, salvo que pongas rutas absolutas.
- Plantilla: copia `config.example.toml` a `config.toml`.
- Si la config es inválida, muestra una ventana de error con el motivo y sale con código 1.
- Una sola instancia: si ya hay una corriendo (archivo `db-monitor.lock` junto al `config.toml`), avisa "db-monitor ya está en ejecución" y sale. Con `--tarea` (lo usa la tarea programada) sale sin avisar.

## Avisos

| Icono | Significa | Al entrar en ese estado |
|---|---|---|
| Verde | La tabla recibe datos | Toast si venía de rojo o de una caída de conexión |
| Rojo | Conectado, pero sin filas en `window_secs` | Toast + ventana de advertencia, y el toast se repite en cada ciclo mientras siga así |
| Amarillo | Sin conexión a la base (también "Conectando…" al arrancar) | Toast, y se repite en cada ciclo mientras siga caída |

- Al arrancar sin errores sale un toast "db-monitor en ejecución" que dice dónde está el icono.
- Los toasts de recuperación salen **solo al cambiar de estado**; los de rojo y amarillo se repiten en cada ciclo mientras dure el problema. Todo toast se reenvía también a ntfy si está configurado.
- Mientras siga en rojo, la ventana de advertencia vuelve a aparecer en cada ciclo si se cerró, encima de las demás pero **sin quitar el foco** a lo que se esté usando (solo la primera vez de cada episodio rojo se trae al frente). Nunca hay más de una. Si sigue abierta, en Windows se actualiza la hora del último chequeo. Se cierra sola cuando vuelven los datos.
- **Caída de la base:** avisa (toast y ntfy, prioridad alta) en cuanto falla la conexión o una consulta, y repite el aviso como mucho una vez por `interval_secs` mientras siga caída; los reintentos más seguidos del backoff no lo repiten. Al volver, toast "conexión restablecida".
- La caída se detecta en el siguiente chequeo: puede tardar hasta `interval_secs`, más `query_timeout_secs` si la consulta se queda colgada. Para enterarse antes, baja `interval_secs`.
- Menú del icono: estado actual (también como tooltip), **Abrir logs** (abre la carpeta) y **Salir**.

## Arranque automático (Windows)

Tarea programada que lo lanza al iniciar sesión, **en la sesión del usuario** (un servicio de Windows corre sin escritorio: no se verían el icono ni las ventanas).

1. Copia a una carpeta escribible por el usuario (p. ej. `C:\db-monitor`, no `Program Files`): `db-monitor.exe`, `config.toml`, `install-task.ps1` y `db-monitor-task.xml` (los dos últimos están en `scripts/` y en `dist/windows/`).
2. En esa carpeta: `powershell -ExecutionPolicy Bypass -File install-task.ps1`. Registra la tarea `db-monitor` para el usuario actual y la lanza. Para otro usuario: `-Usuario EQUIPO\operador`, como administrador.
3. Variables como `DB_PASSWORD` deben ser **de usuario** (`setx DB_PASSWORD ...`); toman efecto en el siguiente inicio de sesión.

- Arranca 30 s después de iniciar sesión y se relanza cada 5 min si no está corriendo (también tras **Salir**). Sin límite de duración.
- **Salir** cierra la app, pero la tarea la vuelve a lanzar en 5 minutos: es lo esperado, la vigilancia no se apaga por error. Para pararlo del todo: deshabilita la tarea en el Programador de tareas o `install-task.ps1 -Desinstalar`.
- `install-task.ps1` comprueba que la carpeta sea escribible y que `config.toml` no sea de solo lectura (logs, lock y ofuscación de credenciales escriben ahí).
- Si además se abre con doble clic no pasa nada: la tarea lo lanza con `--tarea` y, si ya corre, sale sin avisar.
- Sin el script: Programador de tareas → **Importar tarea** → `db-monitor-task.xml`, cambiando antes `EQUIPO\usuario` y `C:\db-monitor`.

## Configuración (`config.toml`)

### `[database]`

| Clave | Obligatoria | Por defecto | Descripción |
|---|---|---|---|
| `kind` | sí | — | `postgres`, `sqlite` u `oracle` |
| `host` | salvo sqlite | — | Servidor |
| `port` | no | pg `5432`, oracle `1521` | Puerto |
| `user` | salvo sqlite | — | Usuario |
| `password` | no | vacío | Mejor usar la variable de entorno `DB_PASSWORD`, que tiene prioridad |
| `dbname` | sí | — | postgres: base de datos · oracle: *service name* · sqlite: ruta del archivo (relativa al config) |
| `connect_timeout_secs` | no | `10` | Timeout de conexión |

### `[freshness]` (obligatoria)

Avisa si una tabla no recibe filas nuevas.

| Clave | Por defecto | Descripción |
|---|---|---|
| `table` | — | `tabla` o `esquema.tabla` (identificador simple, sin comillas) |
| `key_column` | — | Columna con llave única |
| `time_column` | — | Columna time/datetime con el momento de inserción |
| `window_secs` | — | Ventana en segundos (≥ 1) |
| `interval_secs` | `60` | Segundos entre chequeos (≥ 1) |
| `query_timeout_secs` | `30` | Timeout de cada consulta. Mantenlo menor que `interval_secs` |
| `backoff_max_secs` | `300` | Espera máxima entre reintentos de conexión (1 s, 2 s, 4 s… hasta este tope) |
| `clock` | sqlite `utc`, resto `local` | Zona en la que `time_column` guarda la hora: `utc` o `local` (hora del servidor de BD). Si el adquisidor guarda UTC y el servidor está en hora local, pon `utc`; si no, la ventana queda corrida varias horas |

En cada ciclo cuenta las filas con `time_column` dentro de los últimos `window_secs`, usando el reloj de la base de datos:

- Hay filas → `INFO frescura: N registros ...`
- No hay → `WARN sin registros nuevos en 'tabla' durante los últimos Ns` (en cada ciclo mientras siga vacía).

La existencia de la tabla y las columnas se valida **una sola vez**, en la primera conexión; si no existen, el proceso termina con error. Si la tabla o una columna desaparece **después** (p. ej. un `DROP`), también termina con error ("la tabla … dejó de estar disponible"), avisa por ntfy si está configurado y no se informa como caída de conexión.

- **Índice:** crea un índice sobre `time_column` (`CREATE INDEX ix_tabla_ts ON tabla(time_column)`). Sin él, cada chequeo recorre la tabla completa en producción.
- **Fechas futuras:** las filas con `time_column` más de 5 minutos en el futuro (reloj del adquisidor adelantado) no cuentan como recientes.

Un `config.toml` de versiones anteriores con `[monitor]` da error: la sección ya no existe; mueve `interval_secs`, `query_timeout_secs` y `backoff_max_secs` a `[freshness]` y borra `query` y `state_file`.

Formato de `time_column`:

- **SQLite:** texto UTC `YYYY-MM-DD HH:MM:SS` (p. ej. `DEFAULT CURRENT_TIMESTAMP`).
- **PostgreSQL / Oracle:** `timestamp` o `date` en la hora del servidor.

### `[ntfy]` (opcional)

Reenvía los avisos a [ntfy](https://ntfy.sh) para recibirlos en el móvil o en otro equipo (app de ntfy suscrita al mismo `topic`). Se envía lo mismo que los toasts (también sus repeticiones en cada ciclo): prioridad alta en rojo o amarillo, normal al recuperarse o al arrancar.

| Clave | Por defecto | Descripción |
|---|---|---|
| `topic` | — | Tema (letras, dígitos, `-`, `_`). En ntfy.sh los temas son públicos: usa un nombre difícil de adivinar |
| `server` | `https://ntfy.sh` | Servidor ntfy (o uno propio) |
| `token` | vacío | Token de acceso para servidores con autenticación. Mejor la variable de entorno `NTFY_TOKEN`, que tiene prioridad |

Usa el `curl` del sistema (incluido en Windows 10/11 y macOS). El token se le pasa por la entrada estándar, no en la línea de comandos, así que no lo ve otro proceso del equipo. Si falla el envío (sin red, servidor caído), queda un `WARN ntfy: …` en el log y el programa sigue; ese aviso **no se reintenta** (el siguiente ciclo envía el que corresponda).

### `[textos]` (opcional)

Cambia los textos que ve el operador: toasts, ventana de advertencia, menú del icono y ventanas de aviso. Con `tabla` el operador lee una descripción en vez del nombre técnico de la tabla (los logs siguen usando el nombre real). Cada clave que no se ponga conserva su texto por defecto, así que basta con escribir las que cambian. Los logs y los mensajes de error de configuración no cambian.

Los textos admiten variables entre llaves:

- **En todos:** `{app}` (el nombre de abajo), `{tabla}` (la descripción `tabla` de abajo o, si no se pone, el nombre de `freshness.table`) y `{periodo}` (`window_secs` en palabras: `3 minutos`, `2 horas`, `45 segundos`).
- **En algunos:** `{hora}`, `{n}`, `{desde}` y `{donde}`, solo donde lo indica la tabla.

Una clave mal escrita o una variable que ese texto no admite impide arrancar y muestra una ventana de error con el motivo.

| Clave | Dónde aparece | Por defecto | Variables extra |
|---|---|---|---|
| `app` | Títulos por defecto, ventanas de error e información | `db-monitor` | ninguna (ni las globales) |
| `tabla` | Donde se use `{tabla}`: descripción para el operador, p. ej. `las señales de los equipos IEC-104` | el nombre de `freshness.table` | ninguna (ni las globales) |
| `sin_datos.titulo` | Toast "sin datos" y título de la ventana de advertencia | `{app}: sin datos` | |
| `sin_datos.cuerpo` | Toast "sin datos" | `Sin registros nuevos en {tabla} en los últimos {periodo}` | |
| `sin_datos.ventana` | Texto de la ventana de advertencia | `Sin registros nuevos en {tabla} en los últimos {periodo}.\nÚltimo chequeo: {hora}` | `{hora}` del último chequeo |
| `recuperado.titulo` / `.cuerpo` | Toast al volver los datos | `{app}: OK` / `{tabla} volvió a recibir registros` | |
| `sin_conexion.titulo` / `.cuerpo` | Toast al caer la conexión, repetido en cada ciclo | `{app}: sin conexión` / `No se puede conectar a la base de datos desde las {desde}; reintentando` | `{desde}` (hora del corte) |
| `reconectado.titulo` | Toast al recuperar la conexión | `{app}: conexión restablecida` | |
| `reconectado.cuerpo_ok` | Ídem, con datos | `Conexión restablecida · {tabla} OK` | |
| `reconectado.cuerpo_sin_datos` | Ídem, sin datos | `Conexión restablecida · {tabla} sin registros en {periodo}` | |
| `inicio.titulo` / `.cuerpo` | Toast al arrancar | `{app} en ejecución` / `Vigilando {tabla}. El icono está en {donde}.` | `{donde}` en `.cuerpo` |
| `estado.conectando` | Menú y tooltip, al arrancar | `Conectando…` | |
| `estado.ok` | Menú y tooltip, en verde | `OK · {n} registros en {periodo} · chequeo {hora}` | `{n}`, `{hora}` |
| `estado.sin_datos` | Menú y tooltip, en rojo | `Sin registros en {periodo} · chequeo {hora}` | `{hora}` |
| `estado.sin_conexion` | Menú y tooltip, en amarillo | `Sin conexión · reintentando · desde {desde}` | `{desde}` (hora del corte) |
| `ya_en_ejecucion` | Aviso al abrir una segunda instancia | `{app} ya está en ejecución` | |
| `detenido.titulo` | Aviso ntfy al detenerse por un error fatal (el cuerpo es el error) | `{app}: detenido` | |
| `credenciales.titulo` / `.cuerpo` | Toast al ofuscar credenciales en claro (ver abajo) | `{app}: credenciales protegidas` / `Credenciales visibles detectadas, iniciando el proceso de cifrado` | |

```toml
[textos]
app = "Interfaz SCADA - EnerGis"

[textos.sin_datos]
titulo = "Alerta {app}"
ventana = "No se han recibido nuevas señales de los equipos en los últimos {periodo}. (Última revisión: {hora})."

[textos.recuperado]
titulo = "Alerta {app}"
cuerpo = "Comunicación recuperada. Ingresando nuevos datos correctamente."
```

Un salto de línea se escribe `\n`. ntfy recibe los mismos títulos y textos que los toasts. En Windows conviene que el título de la ventana (`sin_datos.titulo`) no coincida con el de otra ventana abierta: se la busca por el título para actualizarla y cerrarla.

### Credenciales ofuscadas (`GSR:`)

`user` y `password` de `[database]` y `token` de `[ntfy]` no quedan legibles en `config.toml`:

- Al arrancar, si alguno está en claro, db-monitor lo ofusca, reescribe `config.toml` y avisa con un toast *"Credenciales visibles detectadas, iniciando el proceso de cifrado"* (y en el log `credenciales ofuscadas en <ruta>`). El valor queda como `password = "GSR:Kx8aBQ..."`.
- Solo cambian esos valores: se conservan comentarios, orden, codificación (BOM) y saltos de línea del Bloc de notas, y en Windows los permisos del archivo.
- **Para cambiar una credencial**, escríbela en claro en lugar del valor `GSR:`; se ofusca en el siguiente arranque.
- Los campos vacíos no se tocan. `DB_PASSWORD` y `NTFY_TOKEN` siguen teniendo prioridad y nunca se escriben en el archivo.
- Si `config.toml` no se puede escribir (solo lectura, sin permisos) o un valor `GSR:` está dañado, sale una ventana de error y no arranca.
- **Es ofuscación, no cifrado:** evita leerlas a simple vista, pero quien tenga el `.exe` puede revertirla. La protección real es un usuario de BD de solo lectura (ver [Requisitos por motor](#requisitos-por-motor)); para cifrado ver `[credenciales] metodo = "dpapi"`.

### `[logging]`

| Clave | Por defecto | Descripción |
|---|---|---|
| `dir` | `logs` | Carpeta; un archivo por día: `db-monitor.log.AAAA-MM-DD` |
| `level` | `info` | `error`, `warn`, `info`, `debug`, `trace` |

Los logs viejos no se borran solos.

## Ejemplos

PostgreSQL:

```toml
[database]
kind = "postgres"
host = "10.0.0.5"
user = "monitor"
dbname = "produccion"


[freshness]
table = "events"
key_column = "id"
time_column = "created_at"
window_secs = 300
interval_secs = 60

[ntfy]                 # opcional: avisos al móvil
topic = "sala-control-db-x7k2"
```

Oracle:

```toml
[database]
kind = "oracle"
host = "10.0.0.6"
user = "monitor"
dbname = "ORCLPDB1"     # service name

[freshness]
table = "events"
key_column = "id"
time_column = "created_at"
window_secs = 300
```

SQLite:

```toml
[database]
kind = "sqlite"
dbname = "datos.db"

[freshness]
table = "events"
key_column = "id"
time_column = "created_at"
window_secs = 300
```

La contraseña va por entorno: `set DB_PASSWORD=...` y luego `db-monitor.exe`.

## Requisitos por motor

- **PostgreSQL:** nada extra. Conexión sin TLS.
- **SQLite:** nada extra (va incluido). El archivo debe existir; no se crea.
- **Oracle:** requiere **Oracle Instant Client** (64 bits) en el `PATH` o junto al `.exe`. En macOS las librerías `.dylib` completas deben estar junto al ejecutable.

### Usuario de solo lectura (recomendado)

db-monitor solo lee la tabla vigilada. Úsalo con un usuario que **solo** tenga `SELECT` sobre esa tabla: si alguien recupera la credencial (la ofuscación `GSR:` no es cifrado), no puede modificar nada. Lo crea el DBA:

```sql
-- Oracle
CREATE USER monitor IDENTIFIED BY "<contraseña>";
GRANT CREATE SESSION TO monitor;
GRANT SELECT ON esquema.tabla TO monitor;   -- en config: table = "esquema.tabla"

-- PostgreSQL
CREATE ROLE monitor LOGIN PASSWORD '<contraseña>';
GRANT CONNECT ON DATABASE mydb TO monitor;
GRANT USAGE ON SCHEMA esquema TO monitor;
GRANT SELECT ON esquema.tabla TO monitor;
```

## Comportamiento

- **Reconexión:** si falla la conexión o una consulta, espera con backoff exponencial (hasta `backoff_max_secs`) y reintenta. Nunca termina por errores de red o de BD.
- **Timeouts:** una consulta que pasa de `query_timeout_secs` se cancela también en el servidor (PostgreSQL y Oracle). En PostgreSQL el keepalive TCP (60 s) detecta en minutos una red caída sin aviso. Un connect de Oracle colgado no se puede abortar: mientras siga colgado no se lanza otro (cuenta como intento fallido), así los reintentos no acumulan hilos.
- **Errores fatales (salen con código 1):** config inválida, tabla o columnas de `[freshness]` inexistentes.
- **Config estricta:** una clave desconocida en cualquier sección (p. ej. `pasword` mal escrito), `query_timeout_secs` mayor o igual que `interval_secs`, `connect_timeout_secs = 0` o un `level` que no sea `error`/`warn`/`info`/`debug`/`trace` impiden arrancar y la ventana de error nombra la clave. Al actualizar desde una versión anterior, revisa estos puntos en tu `config.toml`.
- **Apagado:** menú **Salir** (o Ctrl-C si se lanzó desde una terminal): cierra la conexión, vacía los logs y sale con código 0 (máximo ~2 s).

## Limitaciones conocidas

- Solo detecta registros **nuevos** (por `time_column`), no UPDATE ni DELETE.
- Sin TLS en PostgreSQL.
- En Windows el remitente del toast es `[textos].app`. Para eso db-monitor crea, la primera vez, un acceso directo con ese nombre en el menú Inicio del usuario (`%APPDATA%\Microsoft\Windows\Start Menu\Programs`), sin permisos de administrador. **Esa primera ejecución** los toasts siguen saliendo como "Windows PowerShell": Windows tarda en reconocer el acceso directo nuevo. Desde el siguiente arranque salen con el nombre propio. El log dice cuál se usa (`toasts: remitente …`). Si no se puede crear el acceso directo, siguen como PowerShell.
- Si se cambia `[textos].app`, el acceso directo con el nombre anterior queda en el menú Inicio; `install-task.ps1 -Desinstalar` borra todos los que apuntan al `.exe`.

## Desarrollo

```
cargo test                                   # unitarios + SQLite
cargo test --test db_targets -- --ignored --test-threads=1   # Postgres y Oracle reales
```

**Firma del `.exe`:** con el certificado de firma de código de TI (`.pfx`) dentro del repo o montado en `/src`, `SIGN_PFX=certs/ti.pfx SIGN_PASS=... docker compose run --rm build-windows` firma `dist/windows/db-monitor.exe` (Authenticode con sello de tiempo) y lo verifica. Sin `SIGN_PFX` imprime `AVISO: exe sin firmar`; SmartScreen y algunos antivirus bloquean un `.exe` sin firmar. No subas el `.pfx` al repositorio.

Los tests de Postgres y Oracle leen `TEST_PG_*` y `TEST_ORA_*` (`HOST`, `PORT`, `USER`, `PASSWORD`, `NAME`). El `.exe` de Windows se genera con `docker compose run --rm build-windows` (ver `docs/guia-build-y-prueba.md`).

Iconos: los fuentes son `assets/logo.svg` y `assets/logo-small.svg` (16/24 px). `assets/build-icons.sh` regenera `assets/db-monitor.ico` (incrustado en el `.exe` por `build.rs`) y los iconos de bandeja `assets/tray/*.png|.rgba`; requiere `rsvg-convert` e ImageMagick.

## TODO

**Pendiente de verificar**
- [ ] Ejecutar en Windows 11: tarea programada (`install-task.ps1`), remitente propio del toast (segundo arranque), actualización de la hora en la ventana abierta, icono, toasts, ventana de advertencia (cierre automático) y diálogos de error; medir RAM (< 100 MB) y CPU (< 5 %) en operación continua.
- [ ] Oracle: comprobar que `set_call_timeout` corta consultas lentas y cómo se convierten DATE, LOB y RAW a texto.

**Funcionalidad**
- [ ] TLS para PostgreSQL.
- [ ] Detectar UPDATE y DELETE (hoy solo registros nuevos).

**Robustez**
- [ ] Rotación con límite de archivos de log (`max_log_files`); hoy no se borran solos.
- [ ] Abrir SQLite en solo lectura y tratar un archivo inexistente como error de configuración.
- [ ] Hacer que `non_blocking` no pierda líneas de log en ráfagas.
- [ ] Usar tablas distintas en los tests de Postgres para poder ejecutarlos en paralelo.
