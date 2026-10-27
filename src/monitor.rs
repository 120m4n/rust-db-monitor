use std::future::Future;
use std::time::Duration;

use crate::config::Config;
use crate::db;
use crate::freshness;

/// Resultado de cada chequeo, para quien muestre el estado (bandeja, toasts).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Check {
    Fresh { n: u64 },
    Stale,
    /// La sesión falló (conexión o consulta); se reintenta con backoff.
    Failed,
}

pub fn backoff(attempt: u32, max_secs: u64) -> Duration {
    let secs = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    Duration::from_secs(secs.min(max_secs))
}

/// Bucle principal: conecta (con backoff), valida la tabla una vez y cuenta filas recientes cada `interval_secs`.
/// Solo devuelve `Err` si la tabla o sus columnas no sirven; vuelve `Ok` al disparar `shutdown`.
/// `on_check` recibe cada resultado; nunca se llama antes de devolver `Err`.
pub async fn run(
    cfg: Config,
    shutdown: impl Future<Output = ()>,
    mut on_check: impl FnMut(Check),
) -> anyhow::Result<()> {
    use tokio::time::{interval, sleep, timeout, MissedTickBehavior};

    tokio::pin!(shutdown);
    let f = &cfg.freshness;
    let query_timeout = Duration::from_secs(f.query_timeout_secs);
    let mut table_checked = false;
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
            if let Err(e) = client.set_query_timeout(query_timeout) {
                tracing::warn!("no se pudo fijar el timeout de consulta: {e:#}");
            }
            tracing::info!("conectado");
            // una sola vez por proceso: tabla o columnas inexistentes son un error de configuración
            if !table_checked {
                match freshness::validate_table(&client, f).await {
                    Ok(()) => table_checked = true,
                    Err(_) if client.is_closed() => break 'session true,
                    Err(e) => return Err(e),
                }
            }
            let mut tick = interval(Duration::from_secs(f.interval_secs));
            tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                tokio::select! { _ = &mut shutdown => return stopped(), _ = tick.tick() => {} }
                if client.is_closed() {
                    tracing::warn!("conexión cerrada");
                    break 'session true;
                }
                let n = tokio::select! {
                    _ = &mut shutdown => { client.interrupt(); return stopped() },
                    r = timeout(query_timeout, freshness::recent_count(&client, cfg.database.kind, f)) => match r {
                        Ok(Ok(n)) => n,
                        Ok(Err(e)) => { tracing::warn!("chequeo de frescura fallido: {e:#}"); break 'session true; }
                        Err(_) => {
                            tracing::warn!("chequeo de frescura excedió {}s", f.query_timeout_secs);
                            client.interrupt();
                            break 'session true;
                        }
                    },
                };
                attempt = 0;
                on_check(if n == 0 { Check::Stale } else { Check::Fresh { n } });
                if n == 0 {
                    tracing::warn!("sin registros nuevos en `{}` durante los últimos {}s", f.table, f.window_secs);
                } else {
                    tracing::info!("frescura: {n} registros en `{}` en los últimos {}s", f.table, f.window_secs);
                }
            }
        };
        if failed {
            on_check(Check::Failed);
            let wait = backoff(attempt, f.backoff_max_secs);
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

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff(0, 300), Duration::from_secs(1));
        assert_eq!(backoff(3, 300), Duration::from_secs(8));
        assert_eq!(backoff(20, 300), Duration::from_secs(300));
        assert_eq!(backoff(200, 300), Duration::from_secs(300));
    }
}
