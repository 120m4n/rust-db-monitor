use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub database: Database,
    pub monitor: Monitor,
    #[serde(default)]
    pub logging: Logging,
    /// Chequeo opcional: avisa si la tabla no recibe filas nuevas en `window_secs`.
    #[serde(default)]
    pub freshness: Option<Freshness>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Freshness {
    /// `tabla` o `esquema.tabla`.
    pub table: String,
    /// Columna con llave única (no nula).
    pub key_column: String,
    /// Columna time/datetime con el momento de inserción.
    pub time_column: String,
    pub window_secs: u64,
}

/// Los nombres van sin comillas dentro del SQL: solo identificadores simples.
fn valid_ident(s: &str) -> bool {
    let part = |p: &str| {
        p.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '#'))
    };
    s.split('.').count() <= 2 && s.split('.').all(part)
}

#[derive(Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Postgres,
    Sqlite,
    Oracle,
}

#[derive(Deserialize, Clone)]
pub struct Database {
    pub kind: Kind,
    #[serde(default)]
    pub host: String,
    pub port: Option<u16>,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    /// postgres: base; oracle: service name; sqlite: ruta del archivo.
    pub dbname: String,
    #[serde(default = "d_connect_timeout")]
    pub connect_timeout_secs: u64,
}

impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Database")
            .field("kind", &self.kind)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"***")
            .field("dbname", &self.dbname)
            .finish()
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct Monitor {
    pub query: String,
    #[serde(default = "d_interval")]
    pub interval_secs: u64,
    #[serde(default = "d_query_timeout")]
    pub query_timeout_secs: u64,
    #[serde(default = "d_backoff_max")]
    pub backoff_max_secs: u64,
    #[serde(default = "d_state_file")]
    pub state_file: PathBuf,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Logging {
    #[serde(default = "d_log_dir")]
    pub dir: PathBuf,
    #[serde(default = "d_level")]
    pub level: String,
}

impl Default for Logging {
    fn default() -> Self {
        Logging { dir: d_log_dir(), level: d_level() }
    }
}

fn d_connect_timeout() -> u64 { 10 }
fn d_interval() -> u64 { 60 }
fn d_query_timeout() -> u64 { 30 }
fn d_backoff_max() -> u64 { 300 }
fn d_state_file() -> PathBuf { "state.toml".into() }
fn d_log_dir() -> PathBuf { "logs".into() }
fn d_level() -> String { "info".into() }

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        let db = &self.database;
        if db.dbname.is_empty() {
            bail!("database.dbname is required");
        }
        if db.kind != Kind::Sqlite && (db.host.is_empty() || db.user.is_empty()) {
            bail!("database.host and database.user are required for {:?}", db.kind);
        }
        if self.monitor.interval_secs == 0 {
            bail!("monitor.interval_secs must be >= 1");
        }
        if !self.monitor.query.contains("{last_id}") {
            bail!("monitor.query must contain the {{last_id}} placeholder");
        }
        if let Some(f) = &self.freshness {
            for (name, v) in [("table", &f.table), ("key_column", &f.key_column), ("time_column", &f.time_column)] {
                if !valid_ident(v) {
                    bail!("freshness.{name} `{v}` is not a plain identifier");
                }
            }
            if f.window_secs == 0 {
                bail!("freshness.window_secs must be >= 1");
            }
        }
        Ok(())
    }
}

/// Lee el config, aplica `DB_PASSWORD`, resuelve rutas relativas contra el dir del config y valida.
pub fn load(path: &Path) -> anyhow::Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read config {}", path.display()))?;
    let mut c: Config =
        toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))?;
    if let Ok(pw) = std::env::var("DB_PASSWORD") {
        c.database.password = pw;
    }
    c.validate().with_context(|| format!("invalid config {}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    c.logging.dir = base.join(&c.logging.dir);
    c.monitor.state_file = base.join(&c.monitor.state_file);
    if c.database.kind == Kind::Sqlite {
        c.database.dbname = base.join(&c.database.dbname).to_string_lossy().into_owned();
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: &str = r#"
[database]
kind = "postgres"
host = "h"
user = "u"
dbname = "d"
[monitor]
query = "SELECT id FROM t WHERE id > {last_id} ORDER BY id"
"#;

    fn write(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("config.toml");
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn defaults_applied_and_paths_resolved() {
        let d = tempfile::tempdir().unwrap();
        std::env::remove_var("DB_PASSWORD");
        let c = load(&write(d.path(), MIN)).unwrap();
        assert_eq!(c.monitor.interval_secs, 60);
        assert_eq!(c.monitor.query_timeout_secs, 30);
        assert_eq!(c.database.connect_timeout_secs, 10);
        assert_eq!(c.logging.level, "info");
        assert_eq!(c.logging.dir, d.path().join("logs"));
        assert_eq!(c.monitor.state_file, d.path().join("state.toml"));
    }

    #[test]
    fn rejects_query_without_placeholder() {
        let d = tempfile::tempdir().unwrap();
        let body = MIN.replace("{last_id}", "0");
        let e = load(&write(d.path(), &body)).unwrap_err();
        assert!(format!("{e:#}").contains("{last_id}"));
    }

    #[test]
    fn rejects_zero_interval() {
        let d = tempfile::tempdir().unwrap();
        let body = format!("{MIN}interval_secs = 0\n");
        assert!(load(&write(d.path(), &body)).is_err());
    }

    #[test]
    fn sqlite_needs_no_host_but_postgres_does() {
        let d = tempfile::tempdir().unwrap();
        let sqlite = MIN.replace("postgres", "sqlite").replace("host = \"h\"\nuser = \"u\"\n", "");
        let c = load(&write(d.path(), &sqlite)).unwrap();
        assert_eq!(c.database.dbname, d.path().join("d").to_str().unwrap());
        let pg = MIN.replace("host = \"h\"\n", "");
        assert!(load(&write(d.path(), &pg)).is_err());
    }

    #[test]
    fn freshness_is_optional_and_validated() {
        let d = tempfile::tempdir().unwrap();
        assert!(load(&write(d.path(), MIN)).unwrap().freshness.is_none());
        let sect = |t: &str, w: u64| format!(
            "{MIN}[freshness]\ntable = \"{t}\"\nkey_column = \"id\"\ntime_column = \"created_at\"\nwindow_secs = {w}\n");
        let ok = load(&write(d.path(), &sect("app.events", 120))).unwrap().freshness.unwrap();
        assert_eq!((ok.table.as_str(), ok.window_secs), ("app.events", 120));
        assert!(load(&write(d.path(), &sect("t; DROP TABLE x", 5))).is_err());
        assert!(load(&write(d.path(), &sect("a.b.c", 5))).is_err());
        assert!(load(&write(d.path(), &sect("1abc", 5))).is_err());
        assert!(load(&write(d.path(), &sect("events", 0))).is_err());
    }

    #[test]
    fn rejects_unknown_kind() {
        let d = tempfile::tempdir().unwrap();
        let body = MIN.replace("postgres", "mysql");
        assert!(load(&write(d.path(), &body)).is_err());
    }

    #[test]
    fn password_env_override_and_redaction() {
        let d = tempfile::tempdir().unwrap();
        let body = MIN.replace("user = \"u\"", "user = \"u\"\npassword = \"filepw\"");
        let p = write(d.path(), &body);
        std::env::remove_var("DB_PASSWORD");
        assert_eq!(load(&p).unwrap().database.password, "filepw");
        std::env::set_var("DB_PASSWORD", "s3cr3t'\"x");
        let c = load(&p).unwrap();
        std::env::remove_var("DB_PASSWORD");
        assert_eq!(c.database.password, "s3cr3t'\"x");
        let dbg = format!("{:?}", c.database);
        assert!(!dbg.contains("s3cr3t") && dbg.contains("***"));
    }

    #[test]
    fn missing_file_error_names_path() {
        let e = load(Path::new("/nonexistent/config.toml")).unwrap_err();
        assert!(format!("{e:#}").contains("/nonexistent/config.toml"));
    }
}
