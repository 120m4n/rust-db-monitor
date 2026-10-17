# db-monitor

Aplicación de consola (Rust, pensada para Windows 10) que cada N segundos consulta una base de datos, detecta registros nuevos, avisa si una tabla deja de recibir datos y escribe todo en logs diarios. Soporta **PostgreSQL**, **SQLite** y **Oracle**. Se reconecta sola ante caídas y se apaga con Ctrl-C.

## Uso

```
db-monitor.exe [ruta\a\config.toml]
```

- Sin argumento busca `config.toml` **junto al `.exe`** (un servicio de Windows no arranca en la carpeta del programa).
- `logs/` y `state.toml` se crean relativos a la carpeta del `config.toml`, salvo que pongas rutas absolutas.
- Plantilla: copia `config.example.toml` a `config.toml`.
- Si la config es inválida el proceso sale con código 1 y explica el motivo.

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

### `[monitor]`

| Clave | Obligatoria | Por defecto | Descripción |
|---|---|---|---|
| `query` | sí | — | Consulta de registros nuevos (ver reglas abajo) |
| `interval_secs` | no | `60` | Segundos entre ciclos (≥ 1) |
| `query_timeout_secs` | no | `30` | Timeout de cada consulta. Mantenlo menor que `interval_secs` |
| `backoff_max_secs` | no | `300` | Espera máxima entre reintentos de conexión (1 s, 2 s, 4 s… hasta este tope) |
| `state_file` | no | `state.toml` | Dónde se guarda la última marca de agua |

**Reglas de `query`:**

- Debe contener el marcador `{last_id}`; se sustituye por el último `id` procesado.
- Debe devolver una columna entera llamada `id` (también se acepta un decimal sin fracción, p. ej. `5.00`).
- Debe terminar con `ORDER BY id` y acotar las filas: `LIMIT 1000` en postgres/sqlite, `FETCH FIRST 1000 ROWS ONLY` en Oracle 12c+. Sin tope, el primer arranque carga toda la tabla.
- No pueden repetirse nombres de columna (`SELECT a.id, b.id` da error).

```toml
query = "SELECT id, message FROM events WHERE id > {last_id} ORDER BY id LIMIT 1000"
```

La marca de agua se guarda en `state_file`, así que tras un reinicio no se reprocesa lo ya visto. La primera vez empieza en 0.

### `[freshness]` (opcional)

Avisa si una tabla no recibe filas nuevas. No sustituye a `query`: es un chequeo aparte.

| Clave | Descripción |
|---|---|
| `table` | `tabla` o `esquema.tabla` (identificador simple, sin comillas) |
| `key_column` | Columna con llave única |
| `time_column` | Columna time/datetime con el momento de inserción |
| `window_secs` | Ventana en segundos (≥ 1) |

En cada ciclo cuenta las filas con `time_column` dentro de los últimos `window_secs`, usando el reloj de la base de datos:

- Hay filas → `INFO frescura: N registros ...`
- No hay → `WARN sin registros nuevos en 'tabla' durante los últimos Ns` (en cada ciclo mientras siga vacía).

La existencia de la tabla y las columnas se valida **una sola vez**, en la primera conexión; si no existen, el proceso termina con error.

Formato de `time_column`:

- **SQLite:** texto UTC `YYYY-MM-DD HH:MM:SS` (p. ej. `DEFAULT CURRENT_TIMESTAMP`).
- **PostgreSQL / Oracle:** `timestamp` o `date` en la hora del servidor.

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

[monitor]
query = "SELECT id, message FROM events WHERE id > {last_id} ORDER BY id LIMIT 1000"
interval_secs = 60

[freshness]
table = "events"
key_column = "id"
time_column = "created_at"
window_secs = 300
```

Oracle:

```toml
[database]
kind = "oracle"
host = "10.0.0.6"
user = "monitor"
dbname = "ORCLPDB1"     # service name

[monitor]
query = "SELECT id, message FROM events WHERE id > {last_id} ORDER BY id FETCH FIRST 1000 ROWS ONLY"
```

SQLite:

```toml
[database]
kind = "sqlite"
dbname = "datos.db"

[monitor]
query = "SELECT id, message FROM events WHERE id > {last_id} ORDER BY id LIMIT 1000"
```

La contraseña va por entorno: `set DB_PASSWORD=...` y luego `db-monitor.exe`.

## Requisitos por motor

- **PostgreSQL:** nada extra. Conexión sin TLS.
- **SQLite:** nada extra (va incluido). El archivo debe existir; no se crea.
- **Oracle:** requiere **Oracle Instant Client** (64 bits) en el `PATH` o junto al `.exe`. En macOS las librerías `.dylib` completas deben estar junto al ejecutable.

## Comportamiento

- **Reconexión:** si falla la conexión o una consulta, espera con backoff exponencial (hasta `backoff_max_secs`) y reintenta. Nunca termina por errores de red o de BD.
- **Errores fatales (salen con código 1):** config inválida, `state.toml` corrupto, tabla de `[freshness]` inexistente.
- **Fila con `id` inválido** (NULL, texto, ausente): se registra un error por ciclo y la marca de agua no avanza hasta corregir el dato.
- **Apagado:** Ctrl-C cierra la conexión, vacía los logs y sale con código 0 (máximo ~2 s).

## Limitaciones conocidas

- **Ids no monótonos:** con secuencias y transacciones concurrentes, una fila con `id` menor puede confirmarse después de que la marca de agua pasó y no se verá nunca.
- Solo detecta registros **nuevos**, no UPDATE ni DELETE.
- Sin TLS en PostgreSQL.
- Un error de sintaxis en `config.toml` puede mostrar por pantalla la línea afectada, incluida la del password si está ahí.
- El cierre de la ventana de consola no dispara el apagado ordenado (solo Ctrl-C / Ctrl-Break).

## Desarrollo

```
cargo test                                   # unitarios + SQLite
cargo test --test db_targets -- --ignored --test-threads=1   # Postgres y Oracle reales
```

Los tests de Postgres y Oracle leen `TEST_PG_*` y `TEST_ORA_*` (`HOST`, `PORT`, `USER`, `PASSWORD`, `NAME`). El CI (`.github/workflows/ci.yml`) compila y testea en `windows-latest` y publica `db-monitor.exe`.
