# Alertas visibles y icono de bandeja — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convertir `db-monitor` en un perro guardián solo de frescura, sin consola, con icono de bandeja de 3 colores, toasts en los cambios de estado y una ventana de advertencia insistente.

**Architecture:** `monitor::run` (hilo propio con tokio `current_thread`) informa cada resultado con un callback `Check`. Una máquina de estados pura (`alerts.rs`) traduce `Check` + reloj en acciones (color, toast, ventana). `main.rs` corre el bucle de eventos `tao` en el hilo principal, ejecuta las acciones con el código de plataforma de `desktop.rs` y maneja la gracia de 60 s con `ControlFlow::WaitUntil`.

**Tech Stack:** Rust 2021, tokio, `tao` 0.37, `tray-icon` 0.26 (menú vía `tray_icon::menu`), `notify-rust` 4, `chrono` 0.4 (solo reloj local), `windows-sys` 0.61 (solo Windows), `osascript` en macOS.

**Spec:** `docs/superpowers/specs/2026-10-27-alertas-tray-design.md`

## Global Constraints

- Plataformas: Windows 11 (principal) y macOS Apple Silicon (desarrollo). Linux: no objetivo.
- Debe compilar con `docker compose run --rm build-windows` (`x86_64-pc-windows-gnu`) y en el CI MSVC (`windows-latest`).
- `cargo test` no lanza toasts, ventanas ni iconos.
- Se mantiene: `config.toml` junto al exe, rutas relativas al config, `DB_PASSWORD`, `shutdown_timeout(2s)`, interrupción de consultas lentas.
- Gracia de conexión: 60 s, fija en el código.
- Mensaje para `[monitor]` viejo, literal: `la sección [monitor] ya no existe: mueve interval_secs, query_timeout_secs y backoff_max_secs a [freshness] y borra query/state_file`
- Título fijo de la ventana de advertencia: `db-monitor: sin datos`.
- Textos de UI, comentarios y logs en español (convención del repo).
- Ejecutar cargo con `export PATH=$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH` (ver CLAUDE.md).
- Cada commit termina con:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_013aihWTJxYtdpHNhmuDTgJU
  ```

## Review Focus

1. `config.toml` de la versión anterior con `[monitor]` → error claro con el mensaje literal, no arranque silencioso con defaults (test en Task 1).
2. Base caída al arrancar y, al volver, la tabla no existe → `run` devuelve `Err` con el nombre de la tabla, aunque haya fallado antes muchas veces (test en Task 2).
3. Conexión que parpadea durante horas (falla y vuelve antes de 60 s, una y otra vez) → nunca un toast (test en Task 3).
4. Tabla sin datos y luego consultas que caducan por más de 60 s → amarillo + toast, la ventana de advertencia no se cierra (test en Task 3).
5. Proceso anterior murió sin liberar → el lock se puede volver a tomar (test en Task 4).

---

### Task 1: Núcleo solo de frescura (quitar `[monitor]`)

**Files:**
- Modify: `src/config.rs` (quitar `Monitor`, `state_file`; `freshness` obligatoria; tests)
- Modify: `src/monitor.rs` (quitar marca de agua y sus tests)
- Modify: `tests/db_targets.rs` (sección `monitor::run` y `freshness`)
- Modify: `config.example.toml`, `config.iec104.example.toml`, `README.md`, `docs/guia-build-y-prueba.md`, `CLAUDE.md`

**Interfaces:**
- Produces:
  - `pub struct Config { pub database: Database, pub freshness: Freshness, pub logging: Logging }`
  - `pub struct Freshness { table, key_column, time_column: String, window_secs: u64, interval_secs: u64 /*default 60*/, query_timeout_secs: u64 /*default 30*/, backoff_max_secs: u64 /*default 300*/ }`
  - `monitor::run(cfg: Config, shutdown: impl Future<Output = ()>) -> anyhow::Result<()>` (sin estado; valida la tabla en la primera conexión y luego cuenta cada `interval_secs`)
  - `monitor::backoff` sin cambios.

- [ ] **Step 1: Reescribir los tests de `config.rs`**

`MIN` pasa a ser `[database]` postgres + `[freshness]` con `table = "events"`, `key_column = "id"`, `time_column = "created_at"`, `window_secs = 120`. Tests:
- `defaults_applied_and_paths_resolved`: `interval_secs == 60`, `query_timeout_secs == 30`, `backoff_max_secs == 300`, `connect_timeout_secs == 10`, `logging.dir == dir/logs`.
- `rejects_zero_interval` (`interval_secs = 0` en `[freshness]`) y `rejects_zero_window`.
- `freshness_is_required`: config sin `[freshness]` → `Err`.
- `rejects_old_monitor_section`: `MIN` + `[monitor]\nquery = "x"\n` → `Err` cuyo `{e:#}` contiene `la sección [monitor] ya no existe`.
- `freshness_identifiers_validated`: los casos actuales `"t; DROP TABLE x"`, `"a.b.c"`, `"1abc"` → `Err`; `"app.events"` → Ok.
- Se mantienen `sqlite_needs_no_host_but_postgres_does`, `rejects_unknown_kind`, `password_env_override_and_redaction`, `missing_file_error_names_path`. Se borra `rejects_query_without_placeholder`.

- [ ] **Step 2: Verificar que fallan**

Run: `cargo test --lib config`
Expected: no compila o FAIL (los campos aún están en `Monitor`).

- [ ] **Step 3: Implementar en `config.rs`**

Mover los tres campos con sus `d_*` a `Freshness`; borrar `Monitor`, `d_state_file`. En `load`, antes de deserializar a `Config`, parsear a `toml::Table` y si `contains_key("monitor")` hacer `bail!` con el mensaje literal de Global Constraints.

- [ ] **Step 4: Adaptar `monitor.rs`**

Borrar `State`, `render_query`, `next_watermark`, `parse_id`, `load_state`, `save_state` y sus tests (queda `backoff_doubles_and_caps`). En `run`: sin `last_id`; `table_checked` empieza en `false`; el ciclo solo hace el chequeo de frescura que hoy va después de la consulta (mismas líneas `INFO frescura…` / `WARN sin registros…`), usando `cfg.freshness.*` para intervalo, timeout y backoff. Actualizar el doc-comment de `run` (único `Err`: tabla/columnas inválidas).

- [ ] **Step 5: Adaptar `tests/db_targets.rs`**

- Borrar: `run_sqlite_advances_watermark_and_picks_up_new_rows`, `run_survives_non_numeric_id_without_moving_watermark`, `run_resumes_from_persisted_state`, `run_postgres_advances_watermark`, `run_retries_state_save_until_it_succeeds`.
- `cfg(database)` ahora arma `Config { database, freshness: fresh(), logging }` con `interval_secs: 1, query_timeout_secs: 5, backoff_max_secs: 2`.
- `run_keeps_going_when_freshness_window_is_empty`: solo `r.unwrap()` (sin `load_state`).
- Se mantienen `run_shutdown_interrupts_backoff_when_db_down` y `run_exits_with_error_when_freshness_table_missing`, y todos los de `connect`/`fetch`/`interrupt`/frescura.

- [ ] **Step 6: Tests en verde**

Run: `cargo test`
Expected: todo PASS (ignorados: los de Postgres/Oracle reales).

- [ ] **Step 7: Documentación**

- `config.example.toml` y `config.iec104.example.toml`: sin `[monitor]`; `interval_secs`, `query_timeout_secs`, `backoff_max_secs` dentro de `[freshness]` (iec104: 300 / 60 / 300); quitar comentarios de `state.toml`/primer arranque.
- `README.md`: `[freshness]` obligatoria con la tabla de claves nueva; borrar la sección `[monitor]`, reglas de `query`, marca de agua, "fila con id inválido", "ids no monótonos", `state.toml` corrupto; agregar la nota de migración (mensaje literal); TODO: quitar los ítems que dejan de aplicar (ids no monótonos, tope de filas en `query`).
- `docs/guia-build-y-prueba.md` secciones 2–4: config sin `[monitor]`; quitar comprobaciones de filas nuevas y de `state.toml`.
- `CLAUDE.md`: arquitectura de `config.rs`/`monitor.rs` sin marca de agua.

- [ ] **Step 8: Commit**

```bash
git add -A src tests config.example.toml config.iec104.example.toml README.md docs/guia-build-y-prueba.md CLAUDE.md
git commit -m "refactor!: freshness-only watchdog, drop [monitor] and watermark"
```

---

### Task 2: `run` informa cada resultado (`Check`)

**Files:**
- Modify: `src/monitor.rs`, `src/main.rs` (pasar `|_| {}` mientras no exista la UI), `tests/db_targets.rs`

**Interfaces:**
- Consumes: Task 1 (`Config`, `run`).
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq)] pub enum Check { Fresh { n: u64 }, Stale, Failed }` en `monitor.rs`
  - `pub async fn run(cfg: Config, shutdown: impl Future<Output = ()>, on_check: impl FnMut(Check)) -> anyhow::Result<()>`
  - Se llama `on_check(Failed)` en cada punto donde la sesión termina con `break 'session true` (connect fallido, conexión cerrada, consulta fallida o caducada, validación fallida con la conexión cerrada); `Fresh`/`Stale` tras cada `COUNT`. Nunca antes de devolver `Err`.

- [ ] **Step 1: Tests de integración**

Helper `run_collect(c: Config, ms: u64) -> (anyhow::Result<()>, Vec<Check>)` (acumula en un `Vec` vía el callback). Tests:
- `run_reports_fresh_with_count`: tabla `ev` con 2 filas `datetime('now')`, `window_secs = 60`, 1500 ms → primer `Check` es `Fresh { n: 2 }`.
- `run_reports_stale_when_window_empty`: fila de hace 1 día → contiene `Stale` y ningún `Fresh`.
- `run_reports_failed_when_db_down`: postgres `127.0.0.1:1`, 1500 ms → al menos un `Failed`, todos `Failed`, `r` es Ok.
- `run_fails_late_when_table_missing_after_outage` (Review Focus 2): sqlite cuyo archivo **no existe** al empezar; a los 1300 ms se crea vacío (sin tabla `ev`); `run_for` 6000 ms → `Err` cuyo `{e:#}` contiene `ev`, y antes hubo al menos un `Failed`.

- [ ] **Step 2: Verificar que fallan**

Run: `cargo test --test db_targets run_`
Expected: no compila (`run` aún tiene 2 parámetros).

- [ ] **Step 3: Implementar el callback en `run` y actualizar los llamadores** (`main.rs`, `run_for` existente pasa `|_| {}`).

- [ ] **Step 4: Tests en verde**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src tests
git commit -m "feat: monitor::run reports each check result via callback"
```

---

### Task 3: Máquina de estados de alertas (`alerts.rs`)

**Files:**
- Create: `src/alerts.rs` (con `#[cfg(test)] mod tests`)
- Modify: `src/lib.rs` (`pub mod alerts;`)

**Interfaces:**
- Consumes: `monitor::Check`.
- Produces:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq)] pub enum Light { Green, Yellow, Red }
  #[derive(Debug, Clone, PartialEq)] pub enum Action {
      SetLight(Light),
      Toast { title: String, body: String },
      ShowWarning(String),   // "asegurar abierta"; la UI no hace nada si ya está abierta
      CloseWarning,
  }
  pub const GRACE: Duration = Duration::from_secs(60);
  pub fn human_window(secs: u64) -> String;
  pub struct Alerts { /* privado */ }
  impl Alerts {
      pub fn new(table: &str, window_secs: u64, now: Instant, clock: &str) -> Self;
      pub fn on_check(&mut self, c: Check, now: Instant, clock: &str) -> Vec<Action>;
      pub fn on_tick(&mut self, now: Instant) -> Vec<Action>;   // vence la gracia
      pub fn deadline(&self) -> Option<Instant>;                // cuándo llamar on_tick
      pub fn status(&self) -> String;                           // línea del menú y tooltip
  }
  ```
  `clock` es `HH:MM` local, calculado por el llamador.

- [ ] **Step 1: Tests** (tabla `"T"`, `window_secs = 7200`, `t0 = Instant::now()`)

Textos exactos (W = `2 h`):
- `human_window`: `7200→"2 h"`, `3600→"1 h"`, `5400→"90 min"`, `45→"45 s"`, `90→"90 s"`.
- `starts_yellow_connecting`: `status() == "Conectando…"`, `deadline() == Some(t0 + GRACE)`.
- `first_fresh_is_silent`: `on_check(Fresh{n:42}, t0+5s, "08:15") == [SetLight(Green)]`; `status() == "OK · 42 registros en 2 h · chequeo 08:15"`; `deadline() == None`.
- `fresh_to_stale`: → `[SetLight(Red), Toast{title:"db-monitor: sin datos", body:"Sin registros nuevos en T en los últimos 2 h"}, ShowWarning("Sin registros nuevos en T en los últimos 2 h.\nÚltimo chequeo: 08:20")]`; `status() == "Sin registros en 2 h · chequeo 08:20"`.
- `stale_repeats_only_warning`: tres `Stale` seguidos tras el primero → cada uno `[ShowWarning(..)]` con su reloj, sin `Toast`.
- `stale_to_fresh`: → `[SetLight(Green), Toast{title:"db-monitor: OK", body:"T volvió a recibir registros"}, CloseWarning]`.
- `first_stale_at_startup_alerts`: `new` → `Stale` → mismas 3 acciones que `fresh_to_stale`.
- `failure_within_grace_is_silent`: Green, `Failed` en t1 → `[]`, `deadline() == Some(t1+GRACE)`; `on_tick(t1+59s) == []`; `Fresh` en t1+30s → `[]`, `deadline() == None`.
- `failure_after_grace_alerts`: Green, `Failed` en t1 ("09:00"), `on_tick(t1+60s)` → `[SetLight(Yellow), Toast{title:"db-monitor: sin conexión", body:"No se puede conectar a la base de datos; reintentando"}]`; `status() == "Sin conexión · reintentando · desde 09:00"`; más `Failed` → `[]`.
- `startup_without_connection_alerts`: `new(.., t0, "07:58")`, `on_tick(t0+60s)` → mismas 2 acciones; `status()` `"… desde 07:58"`.
- `reconnect_fresh`: Yellow anunciado → `Fresh{n:3}` → `[SetLight(Green), Toast{title:"db-monitor: conexión restablecida", body:"Conexión restablecida · T OK"}, CloseWarning]`.
- `reconnect_stale`: Yellow anunciado → `Stale` → `[SetLight(Red), Toast{title:"db-monitor: conexión restablecida", body:"Conexión restablecida · T sin registros en 2 h"}, ShowWarning(..)]`.
- `flapping_never_toasts` (Review Focus 3): desde Green, 50 veces `Failed` en t y `Fresh` en t+20s (t avanza 25 s), con `on_tick(deadline)` cuando `deadline()` < siguiente evento → ninguna acción `Toast` en total.
- `stale_then_outage_keeps_warning` (Review Focus 4): Red, `Failed`, `on_tick(+60s)` → contiene `SetLight(Yellow)` y `Toast`, **no** contiene `CloseWarning`.

- [ ] **Step 2: Verificar que fallan**

Run: `cargo test --lib alerts`
Expected: no compila.

- [ ] **Step 3: Implementar `src/alerts.rs`**

Estado interno: luz actual, si la caída ya se anunció, instante + reloj del primer fallo pendiente, último `n` y último reloj de chequeo. `Failed` solo arma el cronómetro (si no hay uno ya); un `Fresh`/`Stale` lo desarma. `CloseWarning` se emite en toda entrada a Green desde un estado no-Green.

- [ ] **Step 4: Tests en verde**

Run: `cargo test --lib alerts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/alerts.rs src/lib.rs
git commit -m "feat: alert state machine with 60s connection grace"
```

---

### Task 4: Código de plataforma (`desktop.rs`)

**Files:**
- Create: `src/desktop.rs`
- Modify: `src/lib.rs` (`pub mod desktop;`), `Cargo.toml`

**Interfaces:**
- Consumes: `alerts::Light`.
- Produces:
  - `pub fn acquire_lock(path: &Path) -> anyhow::Result<Option<std::fs::File>>` — `None` si otro proceso lo tiene (`File::try_lock`, `TryLockError::WouldBlock`).
  - `pub fn icon(light: Light) -> tray_icon::Icon` — círculo 32×32 RGBA, fondo transparente; verde `#2E7D32`, amarillo `#F9A825`, rojo `#C62828`.
  - `pub fn toast(title: &str, body: &str)` — `notify_rust::Notification`; si falla, `tracing::warn!` y sigue (nunca panic).
  - `pub struct Warning` + `impl Warning { pub fn new() -> Self; pub fn show(&mut self, text: &str); pub fn close(&mut self) }` — `show` no hace nada si ya hay una abierta. Título `db-monitor: sin datos`.
  - `pub fn error_box(msg: &str)` y `pub fn info_box(msg: &str)` — bloqueantes, título `db-monitor`.
  - `pub fn open_dir(path: &Path)` — `explorer <dir>` / `open <dir>` con `std::process::Command`; error → `tracing::warn!`.

- [ ] **Step 1: Dependencias en `Cargo.toml`**

```toml
tao = "0.37"
tray-icon = "0.26"
notify-rust = "4"
chrono = { version = "0.4", default-features = false, features = ["clock"] }

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.61", features = ["Win32_Foundation", "Win32_UI_WindowsAndMessaging"] }
```

- [ ] **Step 2: Test del lock** (Review Focus 5, en `desktop.rs`)

`lock_is_exclusive_and_released_on_drop`: tempdir; `acquire_lock(p)` → `Some(a)`; segundo `acquire_lock(p)` → `None`; `drop(a)`; tercero → `Some`.

- [ ] **Step 3: Verificar que falla**

Run: `cargo test --lib desktop`
Expected: no compila.

- [ ] **Step 4: Implementar `desktop.rs`**

- Windows: `MessageBoxW` (`MB_OK | MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND`) en un `std::thread` propio; `Arc<AtomicBool>` "abierta" que el hilo baja al volver. `close`: `FindWindowW(null, "db-monitor: sin datos")` + `PostMessageW(hwnd, WM_CLOSE, 0, 0)`. `error_box`/`info_box`: `MessageBoxW` directo con `MB_ICONERROR` / `MB_ICONINFORMATION`.
- macOS: `osascript -e 'display dialog "<texto>" with title "<título>" buttons {"Aceptar"} default button 1 with icon caution'` como `Child`; abierta = `child.try_wait()` es `Ok(None)`; `close` = `child.kill()`. Escapar `\` y `"` del texto. `error_box`/`info_box`: lo mismo con `.status()` (bloqueante) e `icon stop` / `icon note`.
- Plataformas distintas de esas dos: no se soportan (no hace falta `cfg` para Linux).

- [ ] **Step 5: Tests y builds**

Run: `cargo test` → PASS. Run: `docker compose run --rm build-windows` → compila; la lista de `DLL Name` solo trae DLL del sistema.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/desktop.rs src/lib.rs
git commit -m "feat: desktop glue (tray icons, toasts, warning window, lock)"
```

---

### Task 5: `main.rs` con bandeja y sin consola

**Files:**
- Modify: `src/main.rs`, `README.md`, `docs/guia-build-y-prueba.md`, `CLAUDE.md`

**Interfaces:**
- Consumes: `config::load`, `monitor::{run, Check}`, `alerts::{Alerts, Action, Light}`, todo `desktop::*`.

- [ ] **Step 1: Implementar `main.rs`**

Orden y decisiones:
1. `#![cfg_attr(windows, windows_subsystem = "windows")]`.
2. Ruta del config igual que hoy. `config::load` falla → `desktop::error_box(&format!("{e:#}"))`, salir con 1.
3. `desktop::acquire_lock(<dir del config>/db-monitor.lock)`: `None` → `info_box("db-monitor ya está en ejecución")`, salir con 0. Guardar el `File` hasta el final.
4. Logging como hoy (archivo + stdout). Guardar el guard en una variable que se suelte explícitamente antes de `std::process::exit`.
5. `tao::event_loop::EventLoopBuilder::<UiEvent>::with_user_event()`; en macOS `with_activation_policy(ActivationPolicy::Accessory)`. `enum UiEvent { Check(Check), Menu(MenuId), Stopped(Result<(), String>) }`. `MenuEvent::set_event_handler` reenvía al proxy.
6. Hilo del monitor: runtime `current_thread`; `shutdown` = el primero entre un `tokio::sync::oneshot::Receiver<()>` y `tokio::signal::ctrl_c()`; `on_check` → `proxy.send_event(UiEvent::Check(c))`; al terminar `rt.shutdown_timeout(2s)` y enviar `Stopped`.
7. Crear el `TrayIcon` en `Event::NewEvents(StartCause::Init)` (en macOS no puede crearse antes de que arranque el bucle): icono `Light::Yellow`, tooltip y menú `[estado (deshabilitado), separador, "Abrir logs", "Salir"]`.
8. `UiEvent::Check` → `alerts.on_check(c, Instant::now(), &chrono::Local::now().format("%H:%M").to_string())`; ejecutar cada `Action` (`SetLight` → `tray.set_icon`, `Toast` → `desktop::toast`, `ShowWarning` → `warning.show`, `CloseWarning` → `warning.close`); refrescar texto del item de estado y tooltip con `alerts.status()`.
9. `ControlFlow::WaitUntil(d)` si `alerts.deadline()` es `Some(d)`, si no `Wait`; `StartCause::ResumeTimeReached` → `alerts.on_tick(Instant::now())` y ejecutar acciones.
10. Menú: "Abrir logs" → `desktop::open_dir(&cfg.logging.dir)`; "Salir" → enviar el oneshot (sin confirmación).
11. `Stopped(Ok)` → `warning.close()`, soltar guard, `exit(0)`. `Stopped(Err(msg))` → `tracing::error!`, `warning.close()`, `error_box(&msg)`, soltar guard, `exit(1)`.

- [ ] **Step 2: Build**

Run: `cargo build` → sin errores ni warnings nuevos.

- [ ] **Step 3: Prueba manual en macOS** (carpeta en el scratchpad, `scripts/sqlite-demo.sh`, config sqlite con `window_secs = 10`, `interval_secs = 2`)

| Acción | Esperado |
|---|---|
| `sqlite-demo.sh demo.db 1 3` corriendo y lanzar el binario | sin consola nueva; icono verde; tooltip `OK · …` |
| Parar el script ≥ 12 s | rojo + toast "sin datos" + diálogo |
| Cerrar el diálogo | reaparece en el siguiente ciclo (~2 s) |
| Insertar una fila | verde + toast "OK" + el diálogo se cierra solo |
| Lanzar un segundo binario con el mismo config | diálogo "db-monitor ya está en ejecución"; sale |
| Menú → Abrir logs | Finder en `logs/` |
| Menú → Salir | el icono desaparece; log termina en `apagado`; código 0 |
| Config postgres a `127.0.0.1:1` | amarillo "Conectando…"; a los 60 s toast "sin conexión" |
| Config con `table = "nope"` | diálogo de error con el motivo; código 1 |
| Config con `[monitor]` | diálogo de error con el mensaje literal; código 1 |

- [ ] **Step 4: Build de Windows**

Run: `docker compose run --rm build-windows` → compila; `x86_64-w64-mingw32-objdump -p` muestra `PE32+ executable (GUI)` y solo DLL del sistema.

- [ ] **Step 5: Documentación**

- `README.md`: "Uso" (sin consola, icono, menú, una instancia, `db-monitor.lock`), tabla de colores y avisos (de la spec), errores fatales en ventana, apagado con "Salir" (Ctrl-C solo con terminal); quitar del TODO "Windows Service o icono de bandeja" → dejar solo Windows Service, y "apagado al cerrar la consola".
- `docs/guia-build-y-prueba.md` sección 3: comprobaciones de la tabla del Step 3.
- `CLAUDE.md`: arquitectura (hilos, `alerts.rs` puro, `desktop.rs` por plataforma, `cargo test` no toca la UI).

- [ ] **Step 6: Commit**

```bash
git add src/main.rs README.md docs/guia-build-y-prueba.md CLAUDE.md
git commit -m "feat: tray icon app without console, toasts and warning window"
```
