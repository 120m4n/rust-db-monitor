# db-monitor

Perro guardián (Rust, pensado para Windows 10/11) que cada N segundos comprueba que una tabla siga recibiendo filas y avisa cuando deja de recibirlas. Escribe en logs diarios. Soporta **PostgreSQL**, **SQLite** y **Oracle**. Se reconecta sola ante caídas y se apaga con Ctrl-C.

## Uso

```
db-monitor.exe [ruta\a\config.toml]
```

- Sin argumento busca `config.toml` **junto al `.exe`** (un servicio de Windows no arranca en la carpeta del programa).
- `logs/` se crea relativos a la carpeta del `config.toml`, salvo que pongas rutas absolutas.
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
- **Apagado:** Ctrl-C cierra la conexión, vacía los logs y sale con código 0 (máximo ~2 s).

## Limitaciones conocidas

- Solo detecta registros **nuevos** (por `time_column`), no UPDATE ni DELETE.
- Sin TLS en PostgreSQL.
- Un error de sintaxis en `config.toml` puede mostrar por pantalla la línea afectada, incluida la del password si está ahí.
- El cierre de la ventana de consola no dispara el apagado ordenado (solo Ctrl-C / Ctrl-Break).

## Desarrollo

```
cargo test                                   # unitarios + SQLite
cargo test --test db_targets -- --ignored --test-threads=1   # Postgres y Oracle reales
```

Los tests de Postgres y Oracle leen `TEST_PG_*` y `TEST_ORA_*` (`HOST`, `PORT`, `USER`, `PASSWORD`, `NAME`). El CI (`.github/workflows/ci.yml`) compila y testea en `windows-latest` y publica `db-monitor.exe`.

## TODO

**Pendiente de verificar**
- [ ] Ejecutar en Windows 10: medir RAM (< 100 MB) y CPU (< 5 %) en operación continua, y revisar la consola (colores ANSI en `cmd.exe`).
- [ ] Oracle: comprobar que `set_call_timeout` corta consultas lentas y cómo se convierten DATE, LOB y RAW a texto.

**Funcionalidad**
- [ ] Ejecutar como Windows Service o con icono de bandeja (`monitor::run(cfg, shutdown)` ya está desacoplado de la consola).
- [ ] TLS para PostgreSQL.
- [ ] Apagado ordenado al cerrar la ventana de consola (`tokio::signal::windows::ctrl_close`).
- [ ] Detectar UPDATE y DELETE (hoy solo registros nuevos).

**Robustez**
- [ ] Rotación con límite de archivos de log (`max_log_files`); hoy no se borran solos.
- [ ] Validar `query_timeout_secs < interval_secs`.
- [ ] No mostrar la línea del password en los errores de sintaxis de `config.toml`.
- [ ] Abrir SQLite en solo lectura y tratar un archivo inexistente como error de configuración.
- [ ] Keepalives de TCP más cortos en PostgreSQL (hoy tardan 2 h en detectar una conexión muerta).
- [ ] Hacer que `non_blocking` no pierda líneas de log en ráfagas.
- [ ] Usar tablas distintas en los tests de Postgres para poder ejecutarlos en paralelo.
