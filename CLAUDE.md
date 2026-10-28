# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Console app (Rust, target Windows 10) that polls a DB every N seconds for new rows, optionally warns when a table stops receiving data, and logs to daily files. Engines: PostgreSQL, SQLite, Oracle. README.md (Spanish) is the user-facing config reference; code comments and log messages are Spanish too — keep that.

## Commands

```sh
cargo test                                                    # unit + SQLite integration (no external DBs)
cargo test <name>                                             # single test, e.g. cargo test sqlite_freshness
cargo test --test db_targets -- --ignored --test-threads=1    # real Postgres/Oracle; needs TEST_PG_* / TEST_ORA_* (HOST, PORT, USER, PASSWORD, NAME)
cargo build --release                                         # native build (on Windows: db-monitor.exe with MSVC)
target/release/db-monitor path/to/config.toml                 # default: config.toml next to the exe
docker compose run --rm build-windows                         # cross-compile dist/windows/db-monitor.exe (x86_64-pc-windows-gnu, MinGW)
scripts/sqlite-demo.sh demo.db 10 [secs]                      # create/populate `events` SQLite table for manual runs
```

Windows deployment: `scripts/install-task.ps1` registers `scripts/db-monitor-task.xml` as a logon scheduled task (interactive session, not a service — the tray/toasts need a desktop); relaunch every 5 min with IgnoreNew. Untestable on the Mac (no pwsh).

Postgres tests share table `monitor_t`, hence `--test-threads=1`. Oracle needs Instant Client at runtime (on macOS the `.dylib`s next to the binary).

On this Mac the rustup toolchain isn't on PATH: `export PATH=$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH`. Its `rust-objcopy` is broken (missing libLLVM), so release builds need `CARGO_PROFILE_RELEASE_STRIP=false cargo build --release && strip target/release/db-monitor`. `./build.sh` does all of this and leaves only `dist/db-monitor` + both example configs (runs `cargo clean`; `--keep` skips it).

## Architecture

- `config.rs` — `load()` parses TOML, `DB_PASSWORD` env overrides `password`, relative paths (`logging.dir`, sqlite `dbname`) resolve against the config file's directory, then `validate()`. `[freshness]` is the required core section; a leftover `[monitor]` section is rejected explicitly.
- `db.rs` — `Client` enum (Pg / Sqlite / Oracle) with `connect`, `fetch` (all rows come back as `Row` = column → `Option<String>`, duplicate column names are an error), `set_query_timeout`, `is_closed`, `interrupt`. SQLite and Oracle are sync drivers run in `spawn_blocking`; dropping the future doesn't stop them, so `interrupt()` exists to abort a running query on timeout/shutdown. SQLite opens READ_WRITE without CREATE (missing file = error).
- `monitor.rs` — `run(cfg, shutdown)`: outer reconnect loop with exponential `backoff`, inner `interval` tick loop. Validates the freshness table once on the first successful connection, then each cycle runs `freshness::recent_count`. Only a missing/unusable table returns `Err`; network/DB errors just reconnect.
- `freshness.rs` — the `[freshness]` check: `validate_table` once per process on first connect, then a per-engine `COUNT` of rows within `window_secs` using the DB clock (identifiers are validated as plain idents since they're interpolated into SQL).
- `monitor.rs` reports every result through `on_check(Check::{Fresh{n}, Stale, Failed})`; it has no UI.
- `alerts.rs` — pure state machine (no UI, unit-tested): `Check` + clock → `Action`s (icon colour, toast, show/close warning window). Owns the 60 s grace before a connection failure turns the icon yellow, via `deadline()`/`on_tick()`.
- `desktop.rs` — platform glue: tray icon bitmaps, toasts (`notify-rust`, in a thread because `show()` blocks on macOS), single warning window (Windows: `MessageBoxW` in a thread, closed via `FindWindowW`+`WM_CLOSE`; macOS: an `osascript` child), error/info boxes, `db-monitor.lock` single-instance lock.
- `main.rs` — `windows_subsystem = "windows"` (no console). The main thread runs the `tao` event loop with the `tray-icon` (macOS requires both on the main thread, created at `StartCause::Init`); `monitor::run` runs on its own thread with a current-thread tokio runtime and `shutdown_timeout(2s)`, sending `Check`s to the UI through an `EventLoopProxy`. Menu "Salir" (or Ctrl-C) resolves the shutdown future; fatal errors come back as `UiEvent::Stopped(Err)` → error box, exit 1. `cargo test` never touches UI code.

- `textos.rs` — every operator-visible string (toasts, warning window, menu status, boxes) is a template in `Textos`; `[textos]` overlays it (read separately in `config::load`, unknown keys/`{vars}` rejected), and `resolver()` fills `{app}`/`{tabla}`/`{periodo}` once at load. Per-event vars (`{hora}`, `{n}`, `{desde}`, `{donde}`) are rendered in `alerts.rs`/`main.rs`. Logs stay hardcoded. Don't add user-facing literals elsewhere.
- `ntfy.rs` — optional `[ntfy]`: every state-change toast is also POSTed (JSON) to ntfy by spawning the system `curl` (no HTTP/TLS crate; `CREATE_NO_WINDOW` on Windows). Priority 4 when the batch sets red/yellow, 3 otherwise.
- Icons: `assets/logo.svg` / `logo-small.svg` → `assets/build-icons.sh` → `db-monitor.ico` (embedded by `build.rs` via `embed-resource`, only when the target is Windows) and `assets/tray/*.rgba` (`include_bytes!` in `desktop::icon`, no PNG decoder).

Deliberate shortcuts are marked with `ponytail:` comments. Known limitations and pending work are listed in README's "Limitaciones conocidas" and "TODO" sections.
