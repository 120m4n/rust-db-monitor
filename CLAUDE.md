# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Console app (Rust, target Windows 10) that polls a DB every N seconds for new rows, optionally warns when a table stops receiving data, and logs to daily files. Engines: PostgreSQL, SQLite, Oracle. README.md (Spanish) is the user-facing config reference; code comments and log messages are Spanish too — keep that.

## Commands

```sh
cargo test                                                    # unit + SQLite integration (no external DBs)
cargo test <name>                                             # single test, e.g. cargo test sqlite_freshness
cargo test --test db_targets -- --ignored --test-threads=1    # real Postgres/Oracle; needs TEST_PG_* / TEST_ORA_* (HOST, PORT, USER, PASSWORD, NAME)
cargo build --release                                         # CI builds db-monitor.exe on windows-latest
target/release/db-monitor path/to/config.toml                 # default: config.toml next to the exe
scripts/sqlite-demo.sh demo.db 10 [secs]                      # create/populate `events` SQLite table for manual runs
```

Postgres tests share table `monitor_t`, hence `--test-threads=1`. Oracle needs Instant Client at runtime (on macOS the `.dylib`s next to the binary).

On this Mac the rustup toolchain isn't on PATH: `export PATH=$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH`. Its `rust-objcopy` is broken (missing libLLVM), so release builds need `CARGO_PROFILE_RELEASE_STRIP=false cargo build --release && strip target/release/db-monitor`.

## Architecture

- `config.rs` — `load()` parses TOML, `DB_PASSWORD` env overrides `password`, relative paths (`logging.dir`, `state_file`, sqlite `dbname`) resolve against the config file's directory, then `validate()`.
- `db.rs` — `Client` enum (Pg / Sqlite / Oracle) with `connect`, `fetch` (all rows come back as `Row` = column → `Option<String>`, duplicate column names are an error), `set_query_timeout`, `is_closed`, `interrupt`. SQLite and Oracle are sync drivers run in `spawn_blocking`; dropping the future doesn't stop them, so `interrupt()` exists to abort a running query on timeout/shutdown. SQLite opens READ_WRITE without CREATE (missing file = error).
- `monitor.rs` — `run(cfg, shutdown)`: outer reconnect loop with exponential `backoff`, inner `interval` tick loop. Each cycle: `render_query` substitutes `{last_id}`, `fetch`, `next_watermark` (max integer `id`; any bad id fails the cycle and the watermark doesn't advance), `save_state` (atomic tmp + fsync + rename). Only fatal errors return `Err` (corrupt state file, freshness table missing); network/DB errors just reconnect. `run` is decoupled from the console so it can later be hosted as a Windows service.
- `freshness.rs` — optional `[freshness]` check: `validate_table` once per process on first connect, then a per-engine `COUNT` of rows within `window_secs` using the DB clock (identifiers are validated as plain idents since they're interpolated into SQL).
- `main.rs` — current-thread tokio runtime, `shutdown_timeout(2s)` so a hung blocking query can't block exit; tracing to stdout + non-blocking daily file appender; Ctrl-C is the shutdown future.

Deliberate shortcuts are marked with `ponytail:` comments. Known limitations and pending work are listed in README's "Limitaciones conocidas" and "TODO" sections.
