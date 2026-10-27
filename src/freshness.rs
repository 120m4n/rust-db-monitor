use anyhow::{anyhow, Context};

use crate::config::{Freshness, Kind};
use crate::db::{fetch, Client, Row};

/// Falla si la tabla o alguna de las dos columnas no existe; no lee filas.
fn exists_sql(f: &Freshness) -> String {
    format!("SELECT {}, {} FROM {} WHERE 1=0", f.key_column, f.time_column, f.table)
}

/// Filas insertadas en los últimos `window_secs`, medido con el reloj de la propia BD.
/// sqlite: `time_column` debe ser texto UTC `YYYY-MM-DD HH:MM:SS` (p. ej. `CURRENT_TIMESTAMP`).
fn recent_sql(kind: Kind, f: &Freshness) -> String {
    let (key, time, table, w) = (&f.key_column, &f.time_column, &f.table, f.window_secs);
    let since = match kind {
        Kind::Postgres => format!("now() - INTERVAL '{w} seconds'"),
        Kind::Sqlite => format!("datetime('now', '-{w} seconds')"),
        Kind::Oracle => format!("SYSDATE - {w}/86400"),
    };
    format!("SELECT COUNT({key}) AS n FROM {table} WHERE {time} >= {since}")
}

fn count(rows: &[Row]) -> anyhow::Result<u64> {
    rows.first()
        .and_then(|r| r.get("n"))
        .and_then(|v| v.as_deref())
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| anyhow!("freshness query returned no numeric `n`: {rows:?}"))
}

pub async fn validate_table(client: &Client, f: &Freshness) -> anyhow::Result<()> {
    fetch(client, &exists_sql(f))
        .await
        .map(drop)
        .with_context(|| format!("freshness table `{}` ({}, {}) is not usable", f.table, f.key_column, f.time_column))
}

pub async fn recent_count(client: &Client, kind: Kind, f: &Freshness) -> anyhow::Result<u64> {
    count(&fetch(client, &recent_sql(kind, f)).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f() -> Freshness {
        Freshness {
            table: "events".into(),
            key_column: "id".into(),
            time_column: "created_at".into(),
            window_secs: 90,
            interval_secs: 60,
            query_timeout_secs: 30,
            backoff_max_secs: 300,
        }
    }

    #[test]
    fn recent_sql_uses_each_engines_clock() {
        let pg = recent_sql(Kind::Postgres, &f());
        assert!(pg.contains("COUNT(id)") && pg.contains("FROM events") && pg.contains("now() - INTERVAL '90 seconds'"));
        let lite = recent_sql(Kind::Sqlite, &f());
        assert!(lite.contains("created_at >= datetime('now', '-90 seconds')"));
        let ora = recent_sql(Kind::Oracle, &f());
        assert!(ora.contains("created_at >= SYSDATE - 90/86400"));
    }

    #[test]
    fn exists_sql_selects_nothing_but_checks_table_and_columns() {
        assert_eq!(exists_sql(&f()), "SELECT id, created_at FROM events WHERE 1=0");
    }

    #[test]
    fn count_parses_n_and_rejects_garbage() {
        let row = |v: &str| vec![crate::db::Row::from([("n".to_string(), Some(v.to_string()))])];
        assert_eq!(count(&row("0")).unwrap(), 0);
        assert_eq!(count(&row("12")).unwrap(), 12);
        assert!(count(&row("x")).is_err());
        assert!(count(&[]).is_err());
    }
}
