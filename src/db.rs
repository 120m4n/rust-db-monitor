use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context};
use rusqlite::types::ValueRef;
use tokio_postgres::{NoTls, SimpleQueryMessage};

use crate::config::{Database, Kind};

/// Fila con todos los valores como texto (`None` = NULL); columnas en el nombre que da el motor
/// (Oracle se normaliza a minúsculas).
pub type Row = BTreeMap<String, Option<String>>;

pub enum Client {
    Pg(tokio_postgres::Client),
    Sqlite(Arc<Mutex<rusqlite::Connection>>, rusqlite::InterruptHandle),
    Oracle(Arc<oracle::Connection>),
}

impl Client {
    pub fn is_closed(&self) -> bool {
        match self {
            Client::Pg(c) => c.is_closed(),
            // ponytail: sqlite/oracle sin sondeo; un fallo en fetch dispara la reconexión
            _ => false,
        }
    }

    /// Aborta la consulta en curso (sqlite/oracle corren en hilos bloqueantes que un `drop` del future no detiene).
    pub fn interrupt(&self) {
        match self {
            Client::Sqlite(_, h) => h.interrupt(),
            Client::Oracle(c) => {
                let _ = c.break_execution();
            }
            Client::Pg(_) => {}
        }
    }

    /// Oracle corre en un hilo bloqueante que `tokio::time::timeout` no cancela: el timeout va en el driver.
    pub fn set_query_timeout(&self, d: Duration) -> anyhow::Result<()> {
        if let Client::Oracle(c) = self {
            c.set_call_timeout(Some(d))?;
        }
        Ok(())
    }
}

/// Inserta la columna; un nombre repetido (p. ej. `SELECT a.id, b.id`) haría que una pise a la otra.
fn put(row: &mut Row, name: String, v: Option<String>) -> anyhow::Result<()> {
    if row.insert(name.clone(), v).is_some() {
        return Err(anyhow!("duplicate column name `{name}` in query result"));
    }
    Ok(())
}

pub async fn connect(cfg: &Database) -> anyhow::Result<Client> {
    let timeout = Duration::from_secs(cfg.connect_timeout_secs);
    match cfg.kind {
        Kind::Postgres => {
            let mut pg = tokio_postgres::Config::new();
            pg.host(&cfg.host)
                .port(cfg.port.unwrap_or(5432))
                .user(&cfg.user)
                .password(&cfg.password)
                .dbname(&cfg.dbname)
                .connect_timeout(timeout);
            // connect_timeout solo cubre el TCP; el arranque/auth puede colgarse con un servidor que no responde
            let (client, conn) = tokio::time::timeout(timeout, pg.connect(NoTls))
                .await
                .map_err(|_| anyhow!("postgres connect timed out"))?
                .context("postgres connect failed")?;
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    tracing::warn!("postgres connection ended: {e}");
                }
            });
            Ok(Client::Pg(client))
        }
        Kind::Sqlite => {
            // sin CREATE: un archivo inexistente es un error de config, no una BD vacía
            let conn = rusqlite::Connection::open_with_flags(
                &cfg.dbname,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
            )
            .with_context(|| format!("cannot open sqlite db {}", cfg.dbname))?;
            conn.busy_timeout(timeout)?;
            let handle = conn.get_interrupt_handle();
            Ok(Client::Sqlite(Arc::new(Mutex::new(conn)), handle))
        }
        Kind::Oracle => {
            let (user, pw) = (cfg.user.clone(), cfg.password.clone());
            let addr = format!("{}:{}/{}", cfg.host, cfg.port.unwrap_or(1521), cfg.dbname);
            let job = tokio::task::spawn_blocking(move || oracle::Connection::connect(user, pw, addr));
            let conn = tokio::time::timeout(timeout, job)
                .await
                .map_err(|_| anyhow!("oracle connect timed out"))??
                .context("oracle connect failed")?;
            Ok(Client::Oracle(Arc::new(conn)))
        }
    }
}

pub async fn fetch(client: &Client, sql: &str) -> anyhow::Result<Vec<Row>> {
    match client {
        Client::Pg(c) => {
            let mut out = Vec::new();
            for m in c.simple_query(sql).await? {
                if let SimpleQueryMessage::Row(r) = m {
                    let mut row = Row::new();
                    for (i, col) in r.columns().iter().enumerate() {
                        put(&mut row, col.name().to_string(), r.get(i).map(String::from))?;
                    }
                    out.push(row);
                }
            }
            Ok(out)
        }
        Client::Sqlite(c, _) => {
            let (c, sql) = (c.clone(), sql.to_string());
            tokio::task::spawn_blocking(move || {
                let conn = c.lock().map_err(|_| anyhow!("sqlite mutex poisoned"))?;
                let mut stmt = conn.prepare(&sql)?;
                let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
                let mut rows = stmt.query([])?;
                let mut out = Vec::new();
                while let Some(r) = rows.next()? {
                    let mut row = Row::new();
                    for (i, n) in names.iter().enumerate() {
                        let v = match r.get_ref(i)? {
                            ValueRef::Null => None,
                            ValueRef::Integer(v) => Some(v.to_string()),
                            ValueRef::Real(v) => Some(v.to_string()),
                            ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
                            ValueRef::Blob(b) => Some(format!("<blob {} bytes>", b.len())),
                        };
                        put(&mut row, n.clone(), v)?;
                    }
                    out.push(row);
                }
                Ok(out)
            })
            .await?
        }
        Client::Oracle(c) => {
            let (c, sql) = (c.clone(), sql.to_string());
            tokio::task::spawn_blocking(move || {
                let rs = c.query(&sql, &[])?;
                let names: Vec<String> =
                    rs.column_info().iter().map(|i| i.name().to_lowercase()).collect();
                let mut out = Vec::new();
                for r in rs {
                    let r = r?;
                    let mut row = Row::new();
                    for (i, n) in names.iter().enumerate() {
                        put(&mut row, n.clone(), r.get::<usize, Option<String>>(i)?)?;
                    }
                    out.push(row);
                }
                Ok(out)
            })
            .await?
        }
    }
}
