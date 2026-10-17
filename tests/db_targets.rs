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
#[ignore = "needs TEST_PG_{HOST,PORT,USER,PASSWORD,NAME}"]
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
