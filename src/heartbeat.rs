//! Aviso diario "sigo vivo" por ntfy (`[ntfy] heartbeat = "08:00"`, HU-08). Solo decide cuándo: el
//! envío lo hace `main.rs`. La fecha del último envío se guarda en un archivo para no repetirlo cuando
//! la tarea programada relanza la app.

use std::path::Path;

use anyhow::bail;
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};

/// `"08:00"` → `(8, 0)`.
pub fn parse_hora(s: &str) -> anyhow::Result<(u32, u32)> {
    let ok = |p: &str| p.len() == 2 && p.chars().all(|c| c.is_ascii_digit());
    match s.split_once(':') {
        Some((h, m)) if ok(h) && ok(m) => {
            let (h, m) = (h.parse::<u32>()?, m.parse::<u32>()?);
            if h < 24 && m < 60 {
                return Ok((h, m));
            }
        }
        _ => {}
    }
    bail!("`{s}` debe ser una hora HH:MM, p. ej. \"08:00\"")
}

fn a_las(fecha: NaiveDate, (h, m): (u32, u32)) -> NaiveDateTime {
    fecha.and_time(NaiveTime::from_hms_opt(h, m, 0).expect("hora validada"))
}

/// true si ya pasó la hora de hoy y hoy aún no se envió.
pub fn toca(ahora: NaiveDateTime, hora: (u32, u32), ultimo: Option<NaiveDate>) -> bool {
    ahora >= a_las(ahora.date(), hora) && ultimo != Some(ahora.date())
}

/// Próxima vez que hay que mirar: hoy a esa hora si aún no llegó, si no mañana.
pub fn proximo(ahora: NaiveDateTime, hora: (u32, u32)) -> NaiveDateTime {
    let hoy = a_las(ahora.date(), hora);
    if ahora < hoy { hoy } else { a_las(ahora.date() + Duration::days(1), hora) }
}

/// Fecha `AAAA-MM-DD` guardada en `path`; `None` si no existe o no se entiende.
pub fn leer_ultimo(path: &Path) -> Option<NaiveDate> {
    let t = std::fs::read_to_string(path).ok()?;
    let mut p = t.trim().splitn(3, '-').map(|x| x.parse::<u32>().ok());
    let (y, m, d) = (p.next()??, p.next()??, p.next()??);
    NaiveDate::from_ymd_opt(y as i32, m, d)
}

pub fn guardar(path: &Path, fecha: NaiveDate) {
    let t = format!("{:04}-{:02}-{:02}\n", fecha.year(), fecha.month(), fecha.day());
    if let Err(e) = std::fs::write(path, t) {
        tracing::warn!("heartbeat: no se pudo guardar {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(d: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn toca_despues_de_la_hora_solo_una_vez_por_dia() {
        let hoy = t(6, 0, 0).date();
        assert!(!toca(t(6, 7, 59), (8, 0), None));
        assert!(toca(t(6, 8, 0), (8, 0), None));
        assert!(toca(t(6, 15, 30), (8, 0), Some(t(5, 0, 0).date())));
        assert!(!toca(t(6, 8, 0), (8, 0), Some(hoy)));
        assert!(toca(t(7, 8, 0), (8, 0), Some(hoy)));
    }

    #[test]
    fn proximo_salta_al_dia_siguiente() {
        assert_eq!(proximo(t(6, 7, 0), (8, 0)), t(6, 8, 0));
        assert_eq!(proximo(t(6, 8, 0), (8, 0)), t(7, 8, 0));
        assert_eq!(proximo(t(30, 9, 0), (8, 0)), NaiveDate::from_ymd_opt(2026, 12, 1).unwrap().and_hms_opt(8, 0, 0).unwrap());
    }

    #[test]
    fn parse_hora_rechaza_25_00_y_texto() {
        assert_eq!(parse_hora("08:00").unwrap(), (8, 0));
        assert_eq!(parse_hora("23:59").unwrap(), (23, 59));
        for s in ["25:00", "08:60", "8:00", "ocho", "", "08:00:00"] {
            assert!(parse_hora(s).is_err(), "{s}");
        }
    }

    #[test]
    fn guardar_y_leer_ultimo() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("db-monitor.heartbeat");
        assert_eq!(leer_ultimo(&p), None);
        guardar(&p, t(6, 0, 0).date());
        assert_eq!(leer_ultimo(&p), Some(t(6, 0, 0).date()));
        std::fs::write(&p, "basura").unwrap();
        assert_eq!(leer_ultimo(&p), None);
    }
}
