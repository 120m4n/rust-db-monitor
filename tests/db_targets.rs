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
use db_monitor::config::{Config, Freshness, Logging};
use db_monitor::monitor::{run, Check};

fn cfg(database: Database) -> Config {
    Config { database, freshness: fresh(), logging: Logging::default() }
}

/// Ejecuta `run` y dispara el apagado a los `ms` ms; devuelve lo que tardó en volver.
async fn run_for(c: Config, ms: u64) -> (anyhow::Result<()>, Duration) {
    let t = Instant::now();
    let r = run(c, tokio::time::sleep(Duration::from_millis(ms)), |_| {}).await;
    (r, t.elapsed())
}

/// Ejecuta `run` durante `ms` ms y devuelve cada `Check` informado.
async fn run_collect(c: Config, ms: u64) -> (anyhow::Result<()>, Vec<Check>) {
    let mut checks = Vec::new();
    let r = run(c, tokio::time::sleep(Duration::from_millis(ms)), |k| checks.push(k)).await;
    (r, checks)
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
async fn run_shutdown_interrupts_backoff_when_db_down() {
    let (r, took) = run_for(cfg(db(Kind::Postgres, "127.0.0.1", Some(1), "x")), 500).await;
    r.unwrap();
    assert!(took < Duration::from_secs(2), "took {took:?}");
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

// ---- freshness ----
use db_monitor::freshness::{recent_count, validate_table};

fn fresh() -> Freshness {
    Freshness {
        table: "ev".into(),
        key_column: "id".into(),
        time_column: "ts".into(),
        window_secs: 60,
        interval_secs: 1,
        query_timeout_secs: 5,
        backoff_max_secs: 2,
    }
}

#[tokio::test]
async fn sqlite_freshness_counts_only_rows_inside_window() {
    let d = tempfile::tempdir().unwrap();
    let database = sqlite_db(d.path(), "(1,'a')");
    rusqlite::Connection::open(&database.dbname)
        .unwrap()
        .execute_batch(
            "CREATE TABLE ev(id INTEGER PRIMARY KEY, ts TEXT);
             INSERT INTO ev VALUES (1, datetime('now','-1 hour')), (2, datetime('now','-10 seconds')), (3, datetime('now'));",
        )
        .unwrap();
    let c = connect(&database).await.unwrap();
    validate_table(&c, &fresh()).await.unwrap();
    assert_eq!(recent_count(&c, Kind::Sqlite, &fresh()).await.unwrap(), 2);
    let stale = Freshness { window_secs: 5, ..fresh() };
    assert_eq!(recent_count(&c, Kind::Sqlite, &stale).await.unwrap(), 1);
}

#[tokio::test]
async fn validate_table_fails_for_missing_table_or_column() {
    let d = tempfile::tempdir().unwrap();
    let database = sqlite_db(d.path(), "(1,'a')");
    rusqlite::Connection::open(&database.dbname)
        .unwrap()
        .execute_batch("CREATE TABLE ev(id INTEGER PRIMARY KEY, ts TEXT);")
        .unwrap();
    let c = connect(&database).await.unwrap();
    let e = validate_table(&c, &Freshness { table: "nope".into(), ..fresh() }).await.unwrap_err();
    assert!(format!("{e:#}").contains("nope"));
    assert!(validate_table(&c, &Freshness { time_column: "missing".into(), ..fresh() }).await.is_err());
}

#[tokio::test]
async fn run_exits_with_error_when_freshness_table_missing() {
    let d = tempfile::tempdir().unwrap();
    let mut c = cfg(sqlite_db(d.path(), "(1,'a')"));
    c.freshness = Freshness { table: "nope".into(), ..fresh() };
    let (r, took) = run_for(c, 5000).await;
    assert!(format!("{:#}", r.unwrap_err()).contains("nope"));
    assert!(took < Duration::from_secs(2), "took {took:?}");
}

#[tokio::test]
async fn run_keeps_going_when_freshness_window_is_empty() {
    let d = tempfile::tempdir().unwrap();
    let database = sqlite_db(d.path(), "(1,'a'),(2,'b')");
    rusqlite::Connection::open(&database.dbname)
        .unwrap()
        .execute_batch("CREATE TABLE ev(id INTEGER PRIMARY KEY, ts TEXT); INSERT INTO ev VALUES (1, datetime('now','-1 day'));")
        .unwrap();
    let (r, _) = run_for(cfg(database), 2500).await;
    r.unwrap();
}

// ---- Check ----
fn sqlite_with_ev(dir: &std::path::Path, rows: &str) -> Database {
    let database = sqlite_db(dir, "(1,'a')");
    rusqlite::Connection::open(&database.dbname)
        .unwrap()
        .execute_batch(&format!("CREATE TABLE ev(id INTEGER PRIMARY KEY, ts TEXT); INSERT INTO ev VALUES {rows};"))
        .unwrap();
    database
}

#[tokio::test]
async fn run_reports_fresh_with_count() {
    let d = tempfile::tempdir().unwrap();
    let database = sqlite_with_ev(d.path(), "(1, datetime('now')), (2, datetime('now'))");
    let (r, checks) = run_collect(cfg(database), 1500).await;
    r.unwrap();
    assert_eq!(checks.first(), Some(&Check::Fresh { n: 2 }), "{checks:?}");
}

#[tokio::test]
async fn run_reports_stale_when_window_empty() {
    let d = tempfile::tempdir().unwrap();
    let database = sqlite_with_ev(d.path(), "(1, datetime('now','-1 day'))");
    let (r, checks) = run_collect(cfg(database), 1500).await;
    r.unwrap();
    assert!(checks.contains(&Check::Stale), "{checks:?}");
    assert!(!checks.iter().any(|c| matches!(c, Check::Fresh { .. })), "{checks:?}");
}

#[tokio::test]
async fn run_reports_failed_when_db_down() {
    let (r, checks) = run_collect(cfg(db(Kind::Postgres, "127.0.0.1", Some(1), "x")), 1500).await;
    r.unwrap();
    assert!(!checks.is_empty() && checks.iter().all(|c| *c == Check::Failed), "{checks:?}");
}

#[tokio::test]
async fn run_fails_late_when_table_missing_after_outage() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("late.db");
    let p2 = path.clone();
    let create = tokio::task::spawn_blocking(move || {
        std::thread::sleep(Duration::from_millis(1300));
        rusqlite::Connection::open(p2).unwrap().execute_batch("CREATE TABLE other(x);").unwrap();
    });
    let (r, checks) = run_collect(cfg(db(Kind::Sqlite, "", None, path.to_str().unwrap())), 6000).await;
    create.await.unwrap();
    assert!(format!("{:#}", r.unwrap_err()).contains("ev"));
    assert!(checks.contains(&Check::Failed), "{checks:?}");
}

// ---- revisión final: validación colgada ----
/// Bloquea la BD con un lock exclusivo durante `ms` ms (las lecturas esperan `busy_timeout`).
fn hold_exclusive(path: String, ms: u64) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let c = rusqlite::Connection::open(path).unwrap();
        c.execute_batch("BEGIN EXCLUSIVE;").unwrap();
        std::thread::sleep(Duration::from_millis(ms));
        c.execute_batch("COMMIT;").unwrap();
    })
}

#[tokio::test]
async fn run_shutdown_interrupts_hung_validation() {
    let d = tempfile::tempdir().unwrap();
    let database = sqlite_with_ev(d.path(), "(1, datetime('now'))");
    let lock = hold_exclusive(database.dbname.clone(), 2500);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (r, took) = run_for(cfg(database), 500).await;
    lock.await.unwrap();
    r.unwrap();
    assert!(took < Duration::from_millis(1500), "took {took:?}");
}

#[tokio::test]
async fn run_retries_when_validation_times_out() {
    let d = tempfile::tempdir().unwrap();
    let database = sqlite_with_ev(d.path(), "(1, datetime('now'))");
    let lock = hold_exclusive(database.dbname.clone(), 2000);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut c = cfg(database);
    c.freshness.query_timeout_secs = 1; // menor que busy_timeout (3 s)
    let (r, checks) = run_collect(c, 6000).await;
    lock.await.unwrap();
    r.unwrap();
    assert_eq!(checks.first(), Some(&Check::Failed), "{checks:?}");
    assert!(checks.contains(&Check::Fresh { n: 1 }), "{checks:?}");
}
