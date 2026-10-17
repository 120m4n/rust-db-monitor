
use std::future::Future;
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::db::{self, Row};
use crate::freshness;

#[derive(Serialize, Deserialize)]
struct State {
    last_id: i64,
}

pub fn render_query(template: &str, last_id: i64) -> String {
    template.replace("{last_id}", &last_id.to_string())
}

/// Mayor `id` de las filas (o `current` si no hay); error si alguna fila no trae un `id` entero.
pub fn next_watermark(rows: &[Row], current: i64) -> anyhow::Result<i64> {
    let mut max = current;
    for r in rows {
        let raw = r
            .get("id")
            .and_then(|v| v.as_deref())
            .ok_or_else(|| anyhow!("row without a non-null `id` column: {r:?}"))?;
        max = max.max(parse_id(raw).ok_or_else(|| anyhow!("`id` is not an integer: {r:?}"))?);
    }
    Ok(max)
}

/// Entero, o decimal sin parte fraccionaria (`numeric(10,2)` llega como "5.00").
fn parse_id(s: &str) -> Option<i64> {
    let s = s.trim();
    s.parse::<i64>().ok().or_else(|| {
        let f: f64 = s.parse().ok()?;
        (f.fract() == 0.0 && f.abs() < 9e15).then(|| f as i64)
    })
}

pub fn backoff(attempt: u32, max_secs: u64) -> Duration {
    let secs = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    Duration::from_secs(secs.min(max_secs))
}

pub fn load_state(path: &Path) -> anyhow::Result<i64> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(toml::from_str::<State>(&t)
            .with_context(|| format!("corrupt state file {}", path.display()))?
            .last_id),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e).with_context(|| format!("cannot read state file {}", path.display())),
    }
}

/// Escritura atómica: archivo temporal + rename.
pub fn save_state(path: &Path, last_id: i64) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(toml::to_string(&State { last_id }).map_err(std::io::Error::other)?.as_bytes())?;
        f.sync_all() // sin esto, un corte de luz deja state.toml vacío tras el rename
    };
    write().with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("cannot replace {}", path.display()))
}

/// Bucle principal: conecta (con backoff), consulta cada `interval_secs` y persiste la marca de agua.
/// Solo devuelve `Err` si `state_file` es ilegible/corrupto; vuelve `Ok` al disparar `shutdown`.
pub async fn run(cfg: Config, shutdown: impl Future<Output = ()>) -> anyhow::Result<()> {
    use tokio::time::{interval, sleep, timeout, MissedTickBehavior};

    tokio::pin!(shutdown);
    let m = &cfg.monitor;
    let mut last_id = load_state(&m.state_file)?;
    let mut saved_id = last_id;
    let mut table_checked = cfg.freshness.is_none();
    let mut attempt = 0u32;
    loop {
        // true = la sesión falló y hay que esperar backoff antes de reconectar
        let failed: bool = 'session: {
            let client = tokio::select! {
                _ = &mut shutdown => return stopped(),
                r = db::connect(&cfg.database) => match r {
                    Ok(c) => c,
                    Err(e) => { tracing::warn!("conexión fallida (intento {}): {e:#}", attempt + 1); break 'session true; }
                },
            };
            if let Err(e) = client.set_query_timeout(Duration::from_secs(m.query_timeout_secs)) {
                tracing::warn!("no se pudo fijar el timeout de consulta: {e:#}");
            }
            tracing::info!("conectado");
            // una sola vez por proceso: tabla o columnas inexistentes son un error de configuración
            if let (false, Some(f)) = (table_checked, &cfg.freshness) {
                match freshness::validate_table(&client, f).await {
                    Ok(()) => table_checked = true,
                    Err(_) if client.is_closed() => break 'session true,
                    Err(e) => return Err(e),
                }
            }
            let mut tick = interval(Duration::from_secs(m.interval_secs));
            tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                tokio::select! { _ = &mut shutdown => return stopped(), _ = tick.tick() => {} }
                if client.is_closed() {
                    tracing::warn!("conexión cerrada");
                    break 'session true;
                }
                let sql = render_query(&m.query, last_id);
                let rows = tokio::select! {
                    _ = &mut shutdown => { client.interrupt(); return stopped() },
                    r = timeout(Duration::from_secs(m.query_timeout_secs), db::fetch(&client, &sql)) => match r {
                        Ok(Ok(rows)) => rows,
                        Ok(Err(e)) => { tracing::warn!("consulta fallida: {e:#}"); break 'session true; }
                        Err(_) => {
                            tracing::warn!("consulta excedió {}s", m.query_timeout_secs);
                            client.interrupt();
                            break 'session true;
                        }
                    },
                };
                attempt = 0;
                // se loguea solo lo validado: un lote con id inválido no se re-loguea cada ciclo
                match next_watermark(&rows, last_id) {
                    Ok(next) => {
                        for row in &rows {
                            tracing::info!(?row, "nuevo registro");
                        }
                        last_id = next;
                    }
                    Err(e) => tracing::error!("marca de agua no avanzó: {e:#}"),
                }
                if saved_id != last_id {
                    match save_state(&m.state_file, last_id) {
                        Ok(()) => saved_id = last_id,
                        Err(e) => tracing::error!("no se pudo guardar el estado (se reintenta): {e:#}"),
                    }
                }
                tracing::info!("ciclo: {} filas, last_id={last_id}", rows.len());
                if let Some(f) = &cfg.freshness {
                    let n = tokio::select! {
                        _ = &mut shutdown => { client.interrupt(); return stopped() },
                        r = timeout(Duration::from_secs(m.query_timeout_secs), freshness::recent_count(&client, cfg.database.kind, f)) => match r {
                            Ok(Ok(n)) => n,
                            Ok(Err(e)) => { tracing::warn!("chequeo de frescura fallido: {e:#}"); break 'session true; }
                            Err(_) => {
                                tracing::warn!("chequeo de frescura excedió {}s", m.query_timeout_secs);
                                client.interrupt();
                                break 'session true;
                            }
                        },
                    };
                    if n == 0 {
                        tracing::warn!("sin registros nuevos en `{}` durante los últimos {}s", f.table, f.window_secs);
                    } else {
                        tracing::info!("frescura: {n} registros en `{}` en los últimos {}s", f.table, f.window_secs);
                    }
                }
            }
        };
        if failed {
            let wait = backoff(attempt, m.backoff_max_secs);
            attempt = attempt.saturating_add(1);
            tokio::select! { _ = &mut shutdown => return stopped(), _ = sleep(wait) => {} }
        }
    }
}

fn stopped() -> anyhow::Result<()> {
    tracing::info!("apagado");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: Option<&str>) -> Row {
        Row::from([("id".to_string(), id.map(String::from))])
    }

    #[test]
    fn render_replaces_placeholder() {
        assert_eq!(render_query("id > {last_id} -- {last_id}", 42), "id > 42 -- 42");
    }

    #[test]
    fn watermark_takes_max_not_last() {
        let rows = [row(Some("5")), row(Some("9")), row(Some("7"))];
        assert_eq!(next_watermark(&rows, 0).unwrap(), 9);
    }

    #[test]
    fn watermark_empty_keeps_current() {
        assert_eq!(next_watermark(&[], 12).unwrap(), 12);
    }

    #[test]
    fn watermark_bad_ids_are_errors() {
        assert!(next_watermark(&[row(None)], 0).is_err());
        assert!(next_watermark(&[row(Some("abc"))], 0).is_err());
        assert!(next_watermark(&[Row::new()], 0).is_err());
    }

    #[test]
    fn watermark_accepts_whole_decimals() {
        assert_eq!(next_watermark(&[row(Some("5.00"))], 0).unwrap(), 5);
        assert!(next_watermark(&[row(Some("5.5"))], 0).is_err());
    }

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff(0, 300), Duration::from_secs(1));
        assert_eq!(backoff(3, 300), Duration::from_secs(8));
        assert_eq!(backoff(20, 300), Duration::from_secs(300));
        assert_eq!(backoff(200, 300), Duration::from_secs(300));
    }

    #[test]
    fn state_roundtrip_missing_and_corrupt() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("state.toml");
        assert_eq!(load_state(&p).unwrap(), 0);
        save_state(&p, 77).unwrap();
        assert_eq!(load_state(&p).unwrap(), 77);
        std::fs::write(&p, "garbage ===").unwrap();
        let e = load_state(&p).unwrap_err();
        assert!(format!("{e:#}").contains("state.toml"));
    }
}
