use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use serde::Deserialize;

use crate::textos::Textos;

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub database: Database,
    /// Avisa si la tabla no recibe filas nuevas en `window_secs`.
    pub freshness: Freshness,
    #[serde(default)]
    pub logging: Logging,
    /// Opcional: reenvía los avisos a ntfy.
    #[serde(default)]
    pub ntfy: Option<Ntfy>,
    /// `[textos]`: se lee aparte (`Textos::from_toml`) para validar claves y variables.
    #[serde(skip)]
    pub textos: Textos,
}

#[derive(Deserialize, Clone)]
pub struct Ntfy {
    /// Tema; quien se suscribe a él recibe los avisos. Solo letras, dígitos, `-` y `_`.
    pub topic: String,
    #[serde(default = "d_ntfy_server")]
    pub server: String,
    /// Token de acceso (servidores con autenticación); `NTFY_TOKEN` tiene prioridad.
    #[serde(default)]
    pub token: String,
}

impl fmt::Debug for Ntfy {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Ntfy").field("topic", &self.topic).field("server", &self.server).field("token", &"***").finish()
    }
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
    #[serde(default = "d_interval")]
    pub interval_secs: u64,
    #[serde(default = "d_query_timeout")]
    pub query_timeout_secs: u64,
    #[serde(default = "d_backoff_max")]
    pub backoff_max_secs: u64,
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
fn d_ntfy_server() -> String { "https://ntfy.sh".into() }
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
        let f = &self.freshness;
        for (name, v) in [("table", &f.table), ("key_column", &f.key_column), ("time_column", &f.time_column)] {
            if !valid_ident(v) {
                bail!("freshness.{name} `{v}` is not a plain identifier");
            }
        }
        if f.window_secs == 0 {
            bail!("freshness.window_secs must be >= 1");
        }
        if f.interval_secs == 0 {
            bail!("freshness.interval_secs must be >= 1");
        }
        if let Some(n) = &self.ntfy {
            if n.topic.is_empty() || !n.topic.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')) {
                bail!("ntfy.topic `{}` must be letters, digits, `-` or `_`", n.topic);
            }
            if !(n.server.starts_with("https://") || n.server.starts_with("http://")) {
                bail!("ntfy.server `{}` must start with https:// or http://", n.server);
            }
        }
        Ok(())
    }
}

/// Lee el config, aplica `DB_PASSWORD`, resuelve rutas relativas contra el dir del config y valida.
pub fn load(path: &Path) -> anyhow::Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read config {}", path.display()))?;
    let mut raw: toml::Table =
        toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))?;
    if raw.contains_key("monitor") {
        bail!("invalid config {}: la sección [monitor] ya no existe: mueve interval_secs, query_timeout_secs y backoff_max_secs a [freshness] y borra query/state_file", path.display());
    }
    let textos = match raw.remove("textos") {
        None => Textos::default(),
        Some(toml::Value::Table(t)) => Textos::from_toml(t).with_context(|| format!("invalid config {}", path.display()))?,
        Some(_) => bail!("invalid config {}: textos debe ser una sección [textos]", path.display()),
    };
    let mut c: Config = raw.try_into().with_context(|| format!("invalid config {}", path.display()))?;
    c.textos = textos;
    if let Ok(pw) = std::env::var("DB_PASSWORD") {
        c.database.password = pw;
    }
    if let (Ok(tk), Some(n)) = (std::env::var("NTFY_TOKEN"), c.ntfy.as_mut()) {
        n.token = tk;
    }
    c.validate().with_context(|| format!("invalid config {}", path.display()))?;
    c.textos.resolver(&c.freshness.table, c.freshness.window_secs);
    let base = path.parent().unwrap_or(Path::new("."));
    c.logging.dir = base.join(&c.logging.dir);
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
[freshness]
table = "events"
key_column = "id"
time_column = "created_at"
window_secs = 120
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
        assert_eq!(c.freshness.interval_secs, 60);
        assert_eq!(c.freshness.query_timeout_secs, 30);
        assert_eq!(c.freshness.backoff_max_secs, 300);
        assert_eq!(c.database.connect_timeout_secs, 10);
        assert_eq!(c.logging.level, "info");
        assert_eq!(c.logging.dir, d.path().join("logs"));
    }

    #[test]
    fn rejects_zero_interval() {
        let d = tempfile::tempdir().unwrap();
        let body = format!("{MIN}interval_secs = 0\n");
        assert!(load(&write(d.path(), &body)).is_err());
    }

    #[test]
    fn rejects_zero_window() {
        let d = tempfile::tempdir().unwrap();
        let body = MIN.replace("window_secs = 120", "window_secs = 0");
        assert!(load(&write(d.path(), &body)).is_err());
    }

    #[test]
    fn freshness_is_required() {
        let d = tempfile::tempdir().unwrap();
        let body = MIN.split("[freshness]").next().unwrap();
        assert!(load(&write(d.path(), body)).is_err());
    }

    #[test]
    fn rejects_old_monitor_section() {
        let d = tempfile::tempdir().unwrap();
        let body = format!("{MIN}[monitor]\nquery = \"x\"\n");
        let e = load(&write(d.path(), &body)).unwrap_err();
        assert!(format!("{e:#}").contains("la sección [monitor] ya no existe"), "{e:#}");
    }

    #[test]
    fn freshness_identifiers_validated() {
        let d = tempfile::tempdir().unwrap();
        let with = |t: &str| MIN.replace("table = \"events\"", &format!("table = \"{t}\""));
        let ok = load(&write(d.path(), &with("app.events"))).unwrap().freshness;
        assert_eq!((ok.table.as_str(), ok.window_secs), ("app.events", 120));
        assert!(load(&write(d.path(), &with("t; DROP TABLE x"))).is_err());
        assert!(load(&write(d.path(), &with("a.b.c"))).is_err());
        assert!(load(&write(d.path(), &with("1abc"))).is_err());
    }

    #[test]
    fn ntfy_is_optional_and_validated() {
        let d = tempfile::tempdir().unwrap();
        std::env::remove_var("NTFY_TOKEN");
        assert!(load(&write(d.path(), MIN)).unwrap().ntfy.is_none());
        let with = |t: &str| format!("{MIN}[ntfy]\ntopic = \"{t}\"\n");
        let n = load(&write(d.path(), &with("sala_control-1"))).unwrap().ntfy.unwrap();
        assert_eq!((n.topic.as_str(), n.server.as_str(), n.token.as_str()), ("sala_control-1", "https://ntfy.sh", ""));
        assert!(load(&write(d.path(), &with(""))).is_err());
        assert!(load(&write(d.path(), &with("sala control"))).is_err());
        let bad_server = format!("{}server = \"ntfy.sh\"\n", with("t"));
        assert!(load(&write(d.path(), &bad_server)).is_err());
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
    fn example_configs_are_valid() {
        std::env::remove_var("NTFY_TOKEN");
        for f in ["config.example.toml", "config.iec104.example.toml"] {
            let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(f);
            load(&p).unwrap_or_else(|e| panic!("{f}: {e:#}"));
            // con todas las opciones comentadas (`# clave = …`, `# [sección]`) activadas también debe ser válida
            let text: String = std::fs::read_to_string(&p)
                .unwrap()
                .lines()
                .map(|l| match l.strip_prefix("# ") {
                    Some(r) if r.starts_with('[') || r.split_once(" = ").is_some_and(|(k, _)| k.chars().all(|c| c.is_ascii_lowercase() || c == '_')) => r,
                    _ => l,
                })
                .flat_map(|l| [l, "\n"])
                .collect();
            let d = tempfile::tempdir().unwrap();
            let c = load(&write(d.path(), &text)).unwrap_or_else(|e| panic!("{f} descomentado: {e:#}"));
            assert!(c.ntfy.is_some(), "{f}");
            assert_ne!(c.textos.app, "db-monitor", "{f}: falta el ejemplo [textos]");
        }
    }

    #[test]
    fn textos_loaded_validated_and_resolved() {
        let d = tempfile::tempdir().unwrap();
        let c = load(&write(d.path(), MIN)).unwrap();
        assert_eq!(c.textos.sin_datos_cuerpo, "Sin registros nuevos en events en los últimos 2 minutos");
        let body = format!("{MIN}[textos]\napp = \"SCADA\"\n[textos.sin_datos]\ntitulo = \"Alerta {{app}} {{tabla}}\"\n");
        assert_eq!(load(&write(d.path(), &body)).unwrap().textos.sin_datos_titulo, "Alerta SCADA events");
        let e = load(&write(d.path(), &format!("{MIN}[textos]\nap = \"x\"\n"))).unwrap_err();
        assert!(format!("{e:#}").contains("textos.ap: clave desconocida"), "{e:#}");
        assert!(load(&write(d.path(), &format!("textos = 1\n{MIN}"))).is_err());
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
