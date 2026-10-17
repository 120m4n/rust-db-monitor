# DB Monitor (Rust, Windows 10) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Aplicación de consola Rust que cada N segundos consulta PostgreSQL, detecta registros nuevos por marca de agua (`id`), los registra en logs diarios y se recupera sola de caídas de conexión.

**Architecture:** Un solo crate binario con 4 módulos (`config`, `db`, `monitor`, `main`). `monitor::run` es un bucle `interval` + reconexión con backoff exponencial que recibe un futuro de apagado (así un Windows Service futuro solo cambia `main`, no el núcleo). La detección de cambios es una marca de agua `id` persistida en `state.json`.

**Tech Stack:** Rust stable, tokio (features mínimas), tokio-postgres 0.7, rusqlite (bundled), oracle 0.6, serde + toml 0.8, tracing + tracing-subscriber + tracing-appender, anyhow.

**Spec:** `spec-kit-rust-db-monitor.md` (ver "Cambios respecto a la spec" abajo)

## Cambios respecto a la spec (decisiones tomadas)

| Spec original | Problema | Decisión |
|---|---|---|
| RF-002: PostgreSQL + SQL Server + MySQL | Deps y criterios de aceptación solo cubren PostgreSQL | **Revisión 2 (decisión del usuario): targets = SQLite, PostgreSQL, Oracle; el resto se descarta.** Sin trait: `enum db::Client { Pg, Sqlite, Oracle }` con `match`. Drivers: `tokio-postgres`, `rusqlite` (feature `bundled`, sin dependencias externas), `oracle` 0.6 (requiere Oracle Instant Client en runtime). SQLite/Oracle son síncronos → `spawn_blocking`. |
| RF-003 "cada minuto" vs Objetivo "cada N minutos" | Contradicción | `interval_secs` configurable, default 60. |
| "Detectar cambios o nuevos registros" | Sin mecanismo definido | Marca de agua: la query debe devolver columna `id` (entero) y contener `{last_id}`; se persiste en `state.json` para sobrevivir reinicios. |
| "Logs diarios" | Falta `tracing-appender` | Añadido (rolling diario). `chrono` eliminado (tracing ya timestampea). |
| `tokio features=["full"]` | Contradice RAM < 100 MB | `rt`, `macros`, `time`, `signal`. |
| Estructura `db-monitor/` | El repo ya es el proyecto | Crate en la raíz de `rust-db-monitor/`. |
| "Preparada para Windows Service" | Vago | Criterio concreto: `monitor::run(cfg, shutdown_future)` no conoce ctrl-c ni consola. |
| Sin manejo de secretos | Password en texto plano en config | `DB_PASSWORD` (env) sobrescribe `config.toml`; `Debug` de config oculta el password. |
| Rutas relativas | Un servicio de Windows arranca en `System32` | Config por defecto junto al `.exe`; `logs/` y `state.json` relativos al directorio del config. |
| Sin TLS | — | v1 `NoTls` (BD en red interna). Anotado como límite conocido. |
| Sin timeouts | Una query colgada bloquea el monitor 24/7 | `connect_timeout_secs`, `query_timeout_secs`. |

## Global Constraints

- Plataforma: Windows 10 (build en `windows-latest` CI; desarrollo en macOS debe compilar y pasar tests unitarios).
- RAM < 100 MB, CPU < 5 %: sin `tokio full`, sin acumular filas entre ciclos.
- Operación 24/7: ningún error de BD/red termina el proceso; solo config inválida o `state.json` corrupto al arrancar.
- Config externa en `config.toml`; ejecución cada 60 s por defecto.
- Logs diarios en `logs/`; mostrar versión y estado inicial al arrancar.
- Apagado controlado: cierra conexión, vacía logs, sale con código 0.

## Review Focus

1. Password con caracteres especiales / `Debug` de `Config` → nunca aparece el password en logs ni en errores.
2. La query devuelve `id` NULL, no numérico o ausente → error del ciclo registrado, marca de agua intacta, sin reconexión ni crash.
3. BD caída al **arrancar** → reintenta con backoff, no sale.
4. Ctrl-C durante backoff o durante una query lenta → sale en < 2 s y no avanza la marca de agua de un ciclo incompleto.
5. Query más lenta que el intervalo → ciclos no se solapan ni se acumulan en ráfaga (`MissedTickBehavior::Delay`).

## File Structure

```
Cargo.toml
config.example.toml        # plantilla documentada (config.toml real va en .gitignore)
.gitignore                 # target/, config.toml, logs/, state.json
.github/workflows/ci.yml   # windows-latest: test + build --release
src/main.rs                # args, logging, banner, ctrl-c → monitor::run
src/config.rs              # structs serde, load(), validate(), rutas relativas, redacción de password
src/db.rs                  # connect(), fetch() (simple_query → texto)
src/monitor.rs             # render_query, next_watermark, backoff, state load/save, run()
tests/db_targets.rs        # integración: sqlite siempre; postgres/oracle #[ignore] con env TEST_PG_*/TEST_ORA_*
```

---

### Task 1: Scaffold + `config`

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `config.example.toml`, `src/main.rs` (stub `fn main(){}`), `src/config.rs`

**Interfaces:**
- Produces (en `config.rs`):
  ```rust
  #[derive(Deserialize, Clone)] pub struct Config { pub database: Database, pub monitor: Monitor, #[serde(default)] pub logging: Logging }
  #[derive(Deserialize, Clone, Copy, PartialEq, Debug)] #[serde(rename_all="lowercase")] pub enum Kind { Postgres, Sqlite, Oracle }
  #[derive(Deserialize, Clone)] pub struct Database { pub kind: Kind, #[serde(default)] pub host: String, pub port: Option<u16> /*pg 5432, oracle 1521*/, #[serde(default)] pub user: String, #[serde(default)] pub password: String, pub dbname: String /*pg: base; oracle: service name; sqlite: ruta del archivo (relativa al dir del config)*/, pub connect_timeout_secs: u64 /*10*/ }
  #[derive(Deserialize, Clone)] pub struct Monitor { pub query: String, pub interval_secs: u64 /*60*/, pub query_timeout_secs: u64 /*30*/, pub backoff_max_secs: u64 /*300*/, pub state_file: PathBuf /*"state.json"*/ }
  #[derive(Deserialize, Clone)] pub struct Logging { pub dir: PathBuf /*"logs"*/, pub level: String /*"info"*/ }
  // validate: host y user obligatorios salvo kind=sqlite; dbname siempre
  pub fn load(path: &Path) -> anyhow::Result<Config>   // lee, aplica DB_PASSWORD, resuelve rutas relativas contra path.parent(), llama validate
  impl Config { pub fn validate(&self) -> anyhow::Result<()> }
  ```
  `impl Debug for Database` escribe `password: "***"`.

- [ ] **Step 1: `Cargo.toml`** con deps de la sección Tech Stack (`tokio = { version="1", features=["rt","macros","time","signal"] }`, `tokio-postgres="0.7"`, `serde` derive, `toml="0.8"`, `tracing="0.1"`, `tracing-subscriber` features `env-filter`, `tracing-appender="0.2"`, `anyhow="1"`); `[profile.release] lto=true, strip=true, codegen-units=1`. `tempfile` en `[dev-dependencies]`.
- [ ] **Step 2: Escribir tests fallidos** en `config.rs` (`#[cfg(test)]`):
  ```rust
  fn minimal() -> &'static str { r#"
  [database]
  host="h" 
  user="u"
  dbname="d"
  [monitor]
  query="SELECT id FROM t WHERE id > {last_id} ORDER BY id"
  "# }
  #[test] fn defaults_applied()      // port==5432, interval_secs==60, query_timeout_secs==30, logging.dir=="logs" (tras load: absoluta bajo el dir del config)
  #[test] fn rejects_query_without_last_id_placeholder()   // Err, mensaje contiene "{last_id}"
  #[test] fn rejects_zero_interval()
  #[test] fn env_password_overrides_file()   // DB_PASSWORD=envpw gana sobre password="filepw"
  #[test] fn debug_redacts_password()        // format!("{:?}", cfg.database) no contiene "s3cr3t'\"x" y contiene "***"
  #[test] fn relative_paths_resolve_against_config_dir()   // tempdir/config.toml → state_file == tempdir/state.json
  #[test] fn missing_file_error_names_path()
  ```
  Los tests de env usan un único test secuencial o `serial` manual (un solo `#[test]` que fija y limpia la var) para evitar carreras.
- [ ] **Step 3:** `cargo test config` → FAIL (no compila / no definido).
- [ ] **Step 4:** Implementar `config.rs` según Interfaces. `load` envuelve errores con `.with_context(|| path.display())`.
- [ ] **Step 5:** `cargo test config` → PASS.
- [ ] **Step 6: `config.example.toml`** con cada clave comentada; aviso: "la query DEBE devolver columna `id` entera, contener `{last_id}`, `ORDER BY id` y `LIMIT` (acota RAM)". Ejemplo: `SELECT id, message FROM events WHERE id > {last_id} ORDER BY id LIMIT 1000`.
- [ ] **Step 7: Commit** `feat: scaffold crate and config loading`

### Task 2: Lógica pura de `monitor`

**Files:**
- Create: `src/monitor.rs` (solo funciones puras + estado; `run` llega en Task 4)

**Interfaces:**
- Consumes: `db::Row` (definido aquí temporalmente como `pub type Row = std::collections::BTreeMap<String, Option<String>>` en `db.rs`; crear `src/db.rs` con solo ese alias).
- Produces:
  ```rust
  pub fn render_query(template: &str, last_id: i64) -> String             // reemplaza todas las "{last_id}"
  pub fn next_watermark(rows: &[Row], current: i64) -> anyhow::Result<i64> // max(id) si > current; Err si alguna fila no tiene "id" parseable a i64
  pub fn backoff(attempt: u32, max_secs: u64) -> std::time::Duration       // min(2^attempt, max_secs) s; no desborda con attempt grande
  pub fn load_state(path: &Path) -> anyhow::Result<i64>                    // ausente → 0; JSON corrupto → Err con la ruta
  pub fn save_state(path: &Path, last_id: i64) -> anyhow::Result<()>       // escribe path.tmp y rename (atómico); formato {"last_id":N}
  ```

- [ ] **Step 1: Tests fallidos:**
  ```rust
  #[test] fn render_replaces_placeholder()        // ("id > {last_id}", 42) == "id > 42"
  #[test] fn watermark_takes_max_not_last()       // ids [5,9,7], current 0 → 9
  #[test] fn watermark_empty_keeps_current()      // [], 12 → 12
  #[test] fn watermark_null_id_is_error()         // id None → Err
  #[test] fn watermark_non_numeric_is_error()     // id "abc" → Err
  #[test] fn watermark_missing_column_is_error()
  #[test] fn backoff_doubles_and_caps()           // 0→1s, 3→8s, 20→max (300s), 200 → max sin panic
  #[test] fn state_roundtrip()                    // tempdir
  #[test] fn state_missing_is_zero()
  #[test] fn state_corrupt_errors_with_path()
  ```
- [ ] **Step 2:** `cargo test monitor` → FAIL.
- [ ] **Step 3:** Implementar. `backoff` con `1u64.checked_shl(attempt).unwrap_or(u64::MAX)`; estado con serde (`struct State{last_id:i64}`) y `serde_json`… **no** añadir dependencia: serializar/parsear con `toml` (`state.toml`) — cambiar default `state_file` a `"state.toml"` en Task 1 y en `.gitignore`.
- [ ] **Step 4:** `cargo test monitor` → PASS.
- [ ] **Step 5: Commit** `feat: watermark, backoff and state persistence`

### Task 3: `db`

**Files:**
- Modify: `src/db.rs`
- Create: `tests/db_targets.rs`

**Interfaces:**
- Consumes: `config::Database`.
- Produces:
  ```rust
  pub type Row = BTreeMap<String, Option<String>>;
  pub enum Client { Pg(tokio_postgres::Client), Sqlite(Arc<Mutex<rusqlite::Connection>>), Oracle(Arc<oracle::Connection>) }
  impl Client { pub fn is_closed(&self) -> bool }  // Pg: client.is_closed(); Sqlite/Oracle: false (el error de fetch dispara la reconexión)
  pub async fn connect(cfg: &Database) -> anyhow::Result<Client>
  pub async fn fetch(client: &Client, sql: &str) -> anyhow::Result<Vec<Row>>  // todos los valores como texto
  ```
  Pg: NoTls, connect_timeout, `simple_query` (solo `SimpleQueryMessage::Row`), spawn del driver con `warn` al fallar. Sqlite: `busy_timeout(connect_timeout)`, abrir con `SQLITE_OPEN_READ_WRITE` sin crear (archivo inexistente = error, no BD vacía), `spawn_blocking`, valores vía `ValueRef` → texto. Oracle: connect string `host:port/dbname`, `set_call_timeout(query_timeout)` (un `spawn_blocking` no se cancela con `timeout`), `spawn_blocking`, columnas con `Option<String>`. La query lleva `last_id` (entero, sin riesgo de inyección). Oracle no tiene `LIMIT`: documentar `FETCH FIRST n ROWS ONLY` en `config.example.toml`.

- [ ] **Step 1: Tests de integración** en `tests/db_targets.rs` (renombrado desde `postgres.rs`): `sqlite_roundtrip` (corre siempre, tempfile: 3 filas, `fetch` → 3 `Row` con `"id"==Some("1")`, NULL → `None`; archivo inexistente → Err) y `postgres_roundtrip` / `oracle_roundtrip` (`#[ignore]`, leen `TEST_PG_*` / `TEST_ORA_*`: host, port, user, password, dbname; mismas aserciones sobre una tabla creada con `DROP`/`CREATE` propia); `connect` Pg a `127.0.0.1:1` falla en < `connect_timeout_secs + 1`. Para usarlo, convertir el crate en lib+bin (`src/lib.rs` con `pub mod config; pub mod db; pub mod monitor;`; `main.rs` usa `db_monitor::…`).
- [ ] **Step 2:** `cargo test --test db_targets` → FAIL (sin implementación).
- [ ] **Step 3:** Implementar `connect`/`fetch`. Errores con contexto sin incluir password.
- [ ] **Step 4:** Sqlite sin nada externo; Postgres con `docker run -e POSTGRES_PASSWORD=pw -p 55432:5432 postgres:16` y `TEST_PG_*`; Oracle solo si hay instancia + Instant Client (si no, queda sin verificar y se informa).
- [ ] **Step 5: Commit** `feat: postgres client wrapper`

### Task 4: `monitor::run` (bucle, reconexión, apagado)

**Files:**
- Modify: `src/monitor.rs`, `tests/db_targets.rs`

**Interfaces:**
- Consumes: Task 1–3.
- Produces: `pub async fn run(cfg: Config, shutdown: impl Future<Output = ()>) -> anyhow::Result<()>`

Comportamiento (decide el implementador solo el código; estas son las reglas):
1. `last_id = load_state(...)?` (Err → retorna, es el único fallo fatal aparte de config).
2. Bucle de conexión: `connect` falla → `warn` con intento y espera `backoff(attempt)`; espera es `select!` contra `shutdown`. Éxito → `attempt = 0`, log `info "conectado"`.
3. Con conexión: `tokio::time::interval(interval_secs)` con `MissedTickBehavior::Delay`. Cada tick: `fetch` bajo `timeout(query_timeout_secs)`, todo dentro de `select!` contra `shutdown`.
   - Error/timeout de `fetch` o `client.is_closed()` → log `warn`, volver al paso 2 (reconectar).
   - Éxito → por cada fila `info!(?row, "nuevo registro")`; `next_watermark` (Err → `error!` y se sigue en el siguiente tick **sin** avanzar ni reconectar); si avanza → `save_state` (Err → `error!`, continúa) y actualizar `last_id`; log `info "ciclo: N filas, last_id=X"`.
4. `shutdown` en cualquier punto → drop del cliente, log `info "apagado"`, `Ok(())`.

- [ ] **Step 1: Tests de integración** (`#[ignore]`, Postgres real): (a) **sqlite siempre, y pg/oracle `#[ignore]`**: `run` con `interval_secs=1` y shutdown tras 3 s sobre tabla con 2 filas → `state.toml` contiene `last_id=2`; insertar fila 3 durante la ejecución → termina con 3; (b) shutdown disparado a los 500 ms con Pg a host inalcanzable retorna `Ok` en < 2 s (cubre Review Focus 3 y 4, **no requiere Postgres**, no ignorar este); (c) (sqlite) `id` no numérico → `run` sigue vivo y `state.toml` no cambia.
- [ ] **Step 2:** correr → FAIL.
- [ ] **Step 3:** Implementar `run`.
- [ ] **Step 4:** correr → PASS. Verificación manual de reconexión (criterio de aceptación): con Postgres en docker, `docker stop` 30 s y `docker start` → logs muestran `warn` de reconexión con backoff creciente y luego `conectado`, sin reprocesar filas.
- [ ] **Step 5: Commit** `feat: monitor loop with reconnect and graceful shutdown`

### Task 5: `main`, logging, CI

**Files:**
- Modify: `src/main.rs`
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `config::load`, `monitor::run`.

- [ ] **Step 1:** `main.rs`: `#[tokio::main(flavor = "current_thread")]` (menos RAM/CPU). Arg opcional = ruta de config; default `current_exe().parent()/config.toml`. Cargar config **antes** de iniciar logging solo para obtener `logging.dir/level`; si falla, imprimir a stderr y `exit(1)`.
- [ ] **Step 2:** Logging: `tracing_appender::rolling::daily(dir, "db-monitor.log")` con `non_blocking` (guard vivo hasta el final de `main` para vaciar al apagar) + capa stdout; `EnvFilter` desde `logging.level`.
- [ ] **Step 3:** Banner `info!("db-monitor v{} | db={}@{}:{}/{} | cada {}s", env!("CARGO_PKG_VERSION"), …)` (sin password).
- [ ] **Step 4:** `shutdown` = `tokio::signal::ctrl_c()`. (Windows: cubre Ctrl-C/Ctrl-Break; cierre de consola se trata como ctrl_close si se añade luego con `tokio::signal::windows`.)
- [ ] **Step 5: CI** `windows-latest`: `cargo test` y `cargo build --release`; sube `target/release/db-monitor.exe` como artefacto.
- [ ] **Step 6: Verificación de criterios de aceptación** (manual, en Windows 10 con el artefacto de CI): arranca; lee `config.toml`; conecta; ciclos cada 60 s; existe `logs/db-monitor.log.YYYY-MM-DD`; reconexión (Task 4 step 4); Ctrl-C sale con código 0; RAM en el Administrador de tareas < 100 MB tras 10 min.
- [ ] **Step 7: Commit** `feat: wire main, daily logging and CI`

---

## Self-Review

- **Cobertura:** RF-001 → T1+T5; RF-002 → T3 (sqlite, postgres, oracle; resto descartado); RF-003 → T4; RF-004 → T2+T4; RF-005 → T4; RF-006 → T4+T5. RNF RAM/CPU → features mínimas, `current_thread`, sin acumulación; verificación manual T5. Criterio "preparada para Service/Tray" → firma de `run`.
- **Consistencia de tipos:** `Row` definido en `db.rs` (T2 crea el alias, T3 lo mantiene); `state_file` es `state.toml` en todo el plan.
- **Review Focus:** 1 → T1 `debug_redacts_password`; 2 → T2 watermark tests + T4(c); 3,4 → T4(b); 5 → regla `Delay` en T4 (sin test automático; revisar en código).

## Fuera de alcance (v1)

MySQL/SQL Server y cualquier otro motor, TLS, Windows Service/Tray (la firma de `run` lo permite), detección de UPDATE/DELETE (solo registros nuevos por `id`), métricas.
