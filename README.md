# db-monitor

Perro guardián (Rust, pensado para Windows 10/11) que cada N segundos comprueba que una tabla siga recibiendo filas y avisa cuando deja de recibirlas. Escribe en logs diarios. Soporta **PostgreSQL**, **SQLite** y **Oracle**. Se reconecta sola ante caídas y se apaga con Ctrl-C.

## Uso

```
db-monitor.exe [ruta\a\config.toml]
```

- Se lanza a mano (doble clic). No abre consola: solo aparece un icono en la bandeja del sistema (en macOS, en la barra de menú).
- Sin argumento busca `config.toml` **junto al `.exe`**.
- `logs/` se crea relativo a la carpeta del `config.toml`, salvo que pongas rutas absolutas.
- Plantilla: copia `config.example.toml` a `config.toml`.
- Si la config es inválida, muestra una ventana de error con el motivo y sale con código 1.
- Una sola instancia: si ya hay una corriendo (archivo `db-monitor.lock` junto al `config.toml`), avisa "db-monitor ya está en ejecución" y sale.

## Avisos

| Icono | Significa | Al entrar en ese estado |
|---|---|---|
| Verde | La tabla recibe datos | Toast si venía de rojo o de una caída de conexión |
| Rojo | Conectado, pero sin filas en `window_secs` | Toast + ventana de advertencia |
| Amarillo | Sin conexión a la base durante más de 60 s (también "Conectando…" al arrancar) | Toast |

- Al arrancar sin errores sale un toast "db-monitor en ejecución" que dice dónde está el icono.
- Los toasts salen **solo al cambiar de estado**, no en cada ciclo.
- Mientras siga en rojo, la ventana de advertencia vuelve a aparecer en cada ciclo si se cerró. Nunca hay más de una. Se cierra sola cuando vuelven los datos.
- Un corte de conexión de menos de 60 s no cambia el icono ni avisa; queda solo en el log.
- Menú del icono: estado actual (también como tooltip), **Abrir logs** (abre la carpeta) y **Salir**.

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

En cada ciclo cuenta las filas con `time_column` dentro de los últimos `window_secs`, usando el reloj de la base de datos:

- Hay filas → `INFO frescura: N registros ...`
- No hay → `WARN sin registros nuevos en 'tabla' durante los últimos Ns` (en cada ciclo mientras siga vacía).

La existencia de la tabla y las columnas se valida **una sola vez**, en la primera conexión; si no existen, el proceso termina con error.

Un `config.toml` de versiones anteriores con `[monitor]` da error: la sección ya no existe; mueve `interval_secs`, `query_timeout_secs` y `backoff_max_secs` a `[freshness]` y borra `query` y `state_file`.

Formato de `time_column`:

- **SQLite:** texto UTC `YYYY-MM-DD HH:MM:SS` (p. ej. `DEFAULT CURRENT_TIMESTAMP`).
- **PostgreSQL / Oracle:** `timestamp` o `date` en la hora del servidor.

### `[ntfy]` (opcional)

Reenvía los avisos a [ntfy](https://ntfy.sh) para recibirlos en el móvil o en otro equipo (app de ntfy suscrita al mismo `topic`). Se envía lo mismo que los toasts, solo en los cambios de estado: prioridad alta al pasar a rojo o amarillo, normal al recuperarse.

| Clave | Por defecto | Descripción |
|---|---|---|
| `topic` | — | Tema (letras, dígitos, `-`, `_`). En ntfy.sh los temas son públicos: usa un nombre difícil de adivinar |
| `server` | `https://ntfy.sh` | Servidor ntfy (o uno propio) |
| `token` | vacío | Token de acceso para servidores con autenticación. Mejor la variable de entorno `NTFY_TOKEN`, que tiene prioridad |

Usa el `curl` del sistema (incluido en Windows 10/11 y macOS). Si falla el envío (sin red, servidor caído), queda un `WARN ntfy: …` en el log y el programa sigue.

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

## Comportamiento

- **Reconexión:** si falla la conexión o una consulta, espera con backoff exponencial (hasta `backoff_max_secs`) y reintenta. Nunca termina por errores de red o de BD.
- **Errores fatales (salen con código 1):** config inválida, tabla o columnas de `[freshness]` inexistentes.
- **Apagado:** menú **Salir** (o Ctrl-C si se lanzó desde una terminal): cierra la conexión, vacía los logs y sale con código 0 (máximo ~2 s).

## Limitaciones conocidas

- Solo detecta registros **nuevos** (por `time_column`), no UPDATE ni DELETE.
- Sin TLS en PostgreSQL.
- Un error de sintaxis en `config.toml` puede mostrar por pantalla la línea afectada, incluida la del password si está ahí.
- En Windows, el remitente del toast puede aparecer como un programa genérico (no hay instalador que registre un AppUserModelID propio).

## Desarrollo

```
cargo test                                   # unitarios + SQLite
cargo test --test db_targets -- --ignored --test-threads=1   # Postgres y Oracle reales
```

Los tests de Postgres y Oracle leen `TEST_PG_*` y `TEST_ORA_*` (`HOST`, `PORT`, `USER`, `PASSWORD`, `NAME`). El `.exe` de Windows se genera con `docker compose run --rm build-windows` (ver `docs/guia-build-y-prueba.md`).

Iconos: los fuentes son `assets/logo.svg` y `assets/logo-small.svg` (16/24 px). `assets/build-icons.sh` regenera `assets/db-monitor.ico` (incrustado en el `.exe` por `build.rs`) y los iconos de bandeja `assets/tray/*.png|.rgba`; requiere `rsvg-convert` e ImageMagick.

## TODO

**Pendiente de verificar**
- [ ] Ejecutar en Windows 11: icono, toasts, ventana de advertencia (cierre automático) y diálogos de error; medir RAM (< 100 MB) y CPU (< 5 %) en operación continua.
- [ ] Oracle: comprobar que `set_call_timeout` corta consultas lentas y cómo se convierten DATE, LOB y RAW a texto.

**Funcionalidad**
- [ ] Arrancar con el sistema (acceso directo en Inicio o tarea programada).
- [ ] TLS para PostgreSQL.
- [ ] Detectar UPDATE y DELETE (hoy solo registros nuevos).

**Robustez**
- [ ] Rotación con límite de archivos de log (`max_log_files`); hoy no se borran solos.
- [ ] Validar `query_timeout_secs < interval_secs`.
- [ ] No mostrar la línea del password en los errores de sintaxis de `config.toml`.
- [ ] Abrir SQLite en solo lectura y tratar un archivo inexistente como error de configuración.
- [ ] Keepalives de TCP más cortos en PostgreSQL (hoy tardan 2 h en detectar una conexión muerta).
- [ ] Hacer que `non_blocking` no pierda líneas de log en ráfagas.
- [ ] Usar tablas distintas en los tests de Postgres para poder ejecutarlos en paralelo.
