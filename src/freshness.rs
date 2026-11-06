use anyhow::{anyhow, Context};

use crate::config::{Clock, Freshness, Kind};
use crate::db::{fetch, Client, Row};

/// Falla si la tabla o alguna de las dos columnas no existe; no lee filas.
fn exists_sql(f: &Freshness) -> String {
    format!("SELECT {}, {} FROM {} WHERE 1=0", f.key_column, f.time_column, f.table)
}

/// Filas insertadas en los últimos `window_secs`, medido con el reloj de la propia BD, en la zona de
/// `f.clock` (sin poner: utc en sqlite, hora del servidor en postgres/oracle).
/// sqlite: `time_column` como texto `YYYY-MM-DD HH:MM:SS` (p. ej. `CURRENT_TIMESTAMP`, que es UTC).
// ponytail: tolerancia fija de 5 min al reloj del adquisidor; filas más en el futuro no cuentan
fn recent_sql(kind: Kind, f: &Freshness) -> String {
    const FUTURO: u64 = 300;
    let (key, time, table, w) = (&f.key_column, &f.time_column, &f.table, f.window_secs);
    let clock = f.clock.unwrap_or(if kind == Kind::Sqlite { Clock::Utc } else { Clock::Local });
    let (since, until) = match (kind, clock) {
        (Kind::Postgres, c) => {
            let now = if c == Clock::Utc { "(now() AT TIME ZONE 'UTC')" } else { "now()" };
            (format!("{now} - INTERVAL '{w} seconds'"), format!("{now} + INTERVAL '{FUTURO} seconds'"))
        }
        (Kind::Oracle, Clock::Local) => (format!("SYSDATE - {w}/86400"), format!("SYSDATE + {FUTURO}/86400")),
        (Kind::Oracle, Clock::Utc) => {
            let now = "SYS_EXTRACT_UTC(SYSTIMESTAMP)";
            (format!("{now} - NUMTODSINTERVAL({w}, 'SECOND')"), format!("{now} + NUMTODSINTERVAL({FUTURO}, 'SECOND')"))
        }
        (Kind::Sqlite, c) => {
            let lt = if c == Clock::Local { "'localtime', " } else { "" };
            (format!("datetime('now', {lt}'-{w} seconds')"), format!("datetime('now', {lt}'+{FUTURO} seconds')"))
        }
    };
    format!("SELECT COUNT({key}) AS n FROM {table} WHERE {time} >= {since} AND {time} <= {until}")
}

/// true si el error dice que la tabla o una columna no existe (no un fallo pasajero como un lock).
pub fn tabla_inexistente(e: &anyhow::Error) -> bool {
    use tokio_postgres::error::SqlState;
    let pg = e.chain().filter_map(|c| c.downcast_ref::<tokio_postgres::Error>()).any(|p| {
        matches!(p.code(), Some(c) if *c == SqlState::UNDEFINED_TABLE || *c == SqlState::UNDEFINED_COLUMN)
    });
    let msg = format!("{e:#}");
    pg || ["no such table", "no such column", "ORA-00942", "ORA-00904"].iter().any(|m| msg.contains(m))
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
            clock: None,
        }
    }

    #[test]
    fn recent_sql_por_motor_y_reloj() {
        let con = |clock| Freshness { clock: Some(clock), ..f() };
        let pg = recent_sql(Kind::Postgres, &f());
        assert!(pg.contains("COUNT(id)") && pg.contains("FROM events"), "{pg}");
        assert!(pg.contains("created_at >= now() - INTERVAL '90 seconds'") && pg.contains("created_at <= now() + INTERVAL '300 seconds'"), "{pg}");
        let pg = recent_sql(Kind::Postgres, &con(Clock::Utc));
        assert!(pg.contains(">= (now() AT TIME ZONE 'UTC') - INTERVAL '90 seconds'"), "{pg}");
        let ora = recent_sql(Kind::Oracle, &f());
        assert!(ora.contains("created_at >= SYSDATE - 90/86400") && ora.contains("created_at <= SYSDATE + 300/86400"), "{ora}");
        let ora = recent_sql(Kind::Oracle, &con(Clock::Utc));
        assert!(ora.contains(">= SYS_EXTRACT_UTC(SYSTIMESTAMP) - NUMTODSINTERVAL(90, 'SECOND')"), "{ora}");
        let lite = recent_sql(Kind::Sqlite, &f());
        assert!(lite.contains("created_at >= datetime('now', '-90 seconds')") && lite.contains("created_at <= datetime('now', '+300 seconds')"), "{lite}");
        let lite = recent_sql(Kind::Sqlite, &con(Clock::Local));
        assert!(lite.contains(">= datetime('now', 'localtime', '-90 seconds')"), "{lite}");
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
