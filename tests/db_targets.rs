use std::time::{Duration, Instant};

use db_monitor::config::{Database, Kind};
use db_monitor::db::{connect, fetch, Client};

fn db(kind: Kind, host: &str, port: Option<u16>, dbname: &str) -> Database {
    Database {
        kind,
        host: host.into(),
        port,
        user: String::new(),
        password: String::new(),
        dbname: dbname.into(),
        connect_timeout_secs: 3,
    }
}

fn env(prefix: &str, key: &str) -> String {
    std::env::var(format!("{prefix}_{key}")).unwrap_or_else(|_| panic!("{prefix}_{key} not set"))
}

fn from_env(kind: Kind, prefix: &str) -> Database {
    Database {
        user: env(prefix, "USER"),
        password: env(prefix, "PASSWORD"),
        ..db(kind, &env(prefix, "HOST"), env(prefix, "PORT").parse().ok(), &env(prefix, "NAME"))
    }
}

/// Tabla monitor_t(id, msg) con 3 filas, la 2ª con msg NULL.
async fn assert_three_rows(c: &Client) {
    let rows = fetch(c, "SELECT id, msg FROM monitor_t ORDER BY id").await.unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["id"].as_deref(), Some("1"));
    assert_eq!(rows[0]["msg"].as_deref(), Some("a"));
    assert_eq!(rows[1]["msg"], None);
}

#[tokio::test]
async fn sqlite_roundtrip_and_missing_file() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("t.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE monitor_t(id INTEGER, msg TEXT);
         INSERT INTO monitor_t VALUES (1,'a'),(2,NULL),(3,'c');",
    )
    .unwrap();
    drop(conn);
    let c = connect(&db(Kind::Sqlite, "", None, path.to_str().unwrap())).await.unwrap();
    assert_three_rows(&c).await;
    assert!(fetch(&c, "SELECT * FROM nope").await.is_err());

    let missing = d.path().join("absent.db");
    assert!(connect(&db(Kind::Sqlite, "", None, missing.to_str().unwrap())).await.is_err());
    assert!(!missing.exists(), "must not create an empty database");
}

#[tokio::test]
async fn postgres_unreachable_fails_within_timeout() {
    let t = Instant::now();
    let r = connect(&db(Kind::Postgres, "127.0.0.1", Some(1), "x")).await;
    assert!(r.is_err());
    assert!(t.elapsed() < Duration::from_secs(4));
}

#[tokio::test]
#[ignore = "needs TEST_PG_{HOST,PORT,USER,PASSWORD,NAME}; run with --test-threads=1 (share table monitor_t)"]
async fn postgres_roundtrip() {
    let c = connect(&from_env(Kind::Postgres, "TEST_PG")).await.unwrap();
    let Client::Pg(pg) = &c else { unreachable!() };
    pg.batch_execute(
        "DROP TABLE IF EXISTS monitor_t; CREATE TABLE monitor_t(id bigint, msg text);
         INSERT INTO monitor_t VALUES (1,'a'),(2,NULL),(3,'c');",
    )
    .await
    .unwrap();
    assert_three_rows(&c).await;
}

#[tokio::test]
#[ignore = "needs TEST_ORA_{HOST,PORT,USER,PASSWORD,NAME} + Oracle Instant Client"]
async fn oracle_roundtrip() {
    let c = connect(&from_env(Kind::Oracle, "TEST_ORA")).await.unwrap();
    let Client::Oracle(o) = &c else { unreachable!() };
    let _ = o.execute("DROP TABLE monitor_t", &[]);
    o.execute("CREATE TABLE monitor_t(id NUMBER, msg VARCHAR2(20))", &[]).unwrap();
    for (i, m) in [(1, Some("a")), (2, None), (3, Some("c"))] {
        o.execute("INSERT INTO monitor_t VALUES (:1, :2)", &[&i, &m]).unwrap();
    }
    o.commit().unwrap();
    assert_three_rows(&c).await;
}

// ---- monitor::run ----
use db_monitor::config::{Config, Logging, Monitor};
use db_monitor::monitor::{load_state, run};

fn cfg(database: Database, dir: &std::path::Path) -> Config {
    Config {
        database,
        monitor: Monitor {
            query: "SELECT id, msg FROM monitor_t WHERE id > {last_id} ORDER BY id".into(),
            interval_secs: 1,
            query_timeout_secs: 5,
            backoff_max_secs: 2,
            state_file: dir.join("state.toml"),
        },
        logging: Logging::default(),
    }
}

/// Ejecuta `run` y dispara el apagado a los `ms` ms; devuelve lo que tardó en volver.
async fn run_for(c: Config, ms: u64) -> (anyhow::Result<()>, Duration) {
    let t = Instant::now();
    let r = run(c, tokio::time::sleep(Duration::from_millis(ms))).await;
    (r, t.elapsed())
}

fn sqlite_db(dir: &std::path::Path, rows: &str) -> Database {
    let path = dir.join("t.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE monitor_t(id, msg); INSERT INTO monitor_t VALUES {rows};"
    ))
    .unwrap();
    db(Kind::Sqlite, "", None, path.to_str().unwrap())
}

#[tokio::test]
async fn run_sqlite_advances_watermark_and_picks_up_new_rows() {
    let d = tempfile::tempdir().unwrap();
    let database = sqlite_db(d.path(), "(1,'a'),(2,'b')");
    let path = database.dbname.clone();
    let insert = tokio::task::spawn_blocking(move || {
        std::thread::sleep(Duration::from_millis(1300));
        rusqlite::Connection::open(path)
            .unwrap()
            .execute("INSERT INTO monitor_t VALUES (3,'c')", [])
            .unwrap();
    });
    let (r, _) = run_for(cfg(database, d.path()), 3200).await;
    insert.await.unwrap();
    r.unwrap();
    assert_eq!(load_state(&d.path().join("state.toml")).unwrap(), 3);
}

#[tokio::test]
async fn run_survives_non_numeric_id_without_moving_watermark() {
    let d = tempfile::tempdir().unwrap();
    let (r, _) = run_for(cfg(sqlite_db(d.path(), "('abc','x')"), d.path()), 2200).await;
    r.unwrap();
    assert_eq!(load_state(&d.path().join("state.toml")).unwrap(), 0);
}

#[tokio::test]
async fn run_shutdown_interrupts_backoff_when_db_down() {
    let d = tempfile::tempdir().unwrap();
    let (r, took) = run_for(cfg(db(Kind::Postgres, "127.0.0.1", Some(1), "x"), d.path()), 500).await;
    r.unwrap();
    assert!(took < Duration::from_secs(2), "took {took:?}");
}

#[tokio::test]
async fn run_resumes_from_persisted_state() {
    let d = tempfile::tempdir().unwrap();
    db_monitor::monitor::save_state(&d.path().join("state.toml"), 2).unwrap();
    let (r, _) = run_for(cfg(sqlite_db(d.path(), "(1,'a'),(2,'b'),(5,'e')"), d.path()), 1500).await;
    r.unwrap();
    assert_eq!(load_state(&d.path().join("state.toml")).unwrap(), 5);
}

#[tokio::test]
#[ignore = "needs TEST_PG_*; run with --test-threads=1 (share table monitor_t)"]
async fn run_postgres_advances_watermark() {
    let d = tempfile::tempdir().unwrap();
    let database = from_env(Kind::Postgres, "TEST_PG");
    let c = connect(&database).await.unwrap();
    let Client::Pg(pg) = &c else { unreachable!() };
    pg.batch_execute(
        "DROP TABLE IF EXISTS monitor_t; CREATE TABLE monitor_t(id bigint, msg text);
         INSERT INTO monitor_t VALUES (1,'a'),(2,'b');",
    )
    .await
    .unwrap();
    let (r, _) = run_for(cfg(database, d.path()), 1500).await;
    r.unwrap();
    assert_eq!(load_state(&d.path().join("state.toml")).unwrap(), 2);
}

// ---- fixes tras la revisión final ----
#[tokio::test]
async fn postgres_connect_that_accepts_tcp_but_never_answers_times_out() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let t = Instant::now();
    assert!(connect(&db(Kind::Postgres, "127.0.0.1", Some(port), "x")).await.is_err());
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn fetch_rejects_duplicate_column_names() {
    let d = tempfile::tempdir().unwrap();
    let c = connect(&sqlite_db(d.path(), "(1,'a')")).await.unwrap();
    assert!(fetch(&c, "SELECT 1 AS id, 2 AS id").await.is_err());
}

#[tokio::test]
async fn interrupt_aborts_slow_sqlite_query() {
    let d = tempfile::tempdir().unwrap();
    let c = std::sync::Arc::new(connect(&sqlite_db(d.path(), "(1,'a')")).await.unwrap());
    let c2 = c.clone();
    let slow = tokio::spawn(async move {
        fetch(&c2, "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c) SELECT count(*) FROM c").await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    c.interrupt();
    let r = tokio::time::timeout(Duration::from_secs(2), slow).await.expect("query still running");
    assert!(r.unwrap().is_err());
}

#[tokio::test]
async fn run_retries_state_save_until_it_succeeds() {
    let d = tempfile::tempdir().unwrap();
    let mut c = cfg(sqlite_db(d.path(), "(1,'a'),(2,'b')"), d.path());
    c.monitor.state_file = d.path().join("sub").join("state.toml"); // el dir no existe aún
    let sub = d.path().join("sub");
    let mk = tokio::task::spawn_blocking(move || {
        std::thread::sleep(Duration::from_millis(1300));
        std::fs::create_dir(sub).unwrap();
    });
    let (r, _) = run_for(c, 3300).await;
    mk.await.unwrap();
    r.unwrap();
    assert_eq!(load_state(&d.path().join("sub").join("state.toml")).unwrap(), 2);
}
