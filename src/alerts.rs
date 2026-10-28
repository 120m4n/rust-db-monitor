//! Qué mostrar (color, toast, ventana) según los resultados de `monitor::run`. Sin UI: solo decide.

use std::time::{Duration, Instant};

use crate::monitor::Check;
use crate::textos::{render, Textos};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Light {
    Green,
    Yellow,
    Red,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    SetLight(Light),
    Toast { title: String, body: String },
    /// Asegurar abierta la ventana de advertencia; si ya lo está, no hacer nada.
    ShowWarning(String),
    CloseWarning,
}

/// Tiempo que debe durar un corte antes de avisar.
pub const GRACE: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, PartialEq)]
enum Last {
    Nothing,
    Fresh(u64),
    Stale,
}

pub struct Alerts {
    /// Ya resueltos (`Textos::resolver`): quedan solo las variables de cada evento.
    tx: Textos,
    light: Light,
    last: Last,
    last_clock: String,
    /// Primer fallo del corte en curso (o el arranque, hasta el primer resultado bueno).
    failing_since: Option<(Instant, String)>,
    /// Ya se avisó "sin conexión".
    announced: bool,
}

impl Alerts {
    /// `clock` es la hora local `HH:MM`, la calcula quien llama.
    pub fn new(tx: Textos, now: Instant, clock: &str) -> Self {
        Alerts {
            tx,
            light: Light::Yellow,
            last: Last::Nothing,
            last_clock: String::new(),
            failing_since: Some((now, clock.into())),
            announced: false,
        }
    }

    pub fn on_check(&mut self, c: Check, now: Instant, clock: &str) -> Vec<Action> {
        let (light, last) = match c {
            Check::Failed => {
                self.failing_since.get_or_insert_with(|| (now, clock.into()));
                return Vec::new();
            }
            Check::Fresh { n } => (Light::Green, Last::Fresh(n)),
            Check::Stale => (Light::Red, Last::Stale),
        };
        let (prev, reconnected) = (self.light, self.announced);
        self.light = light;
        self.last = last;
        self.last_clock = clock.into();
        self.failing_since = None;
        self.announced = false;

        let tx = &self.tx;
        let mut out = Vec::new();
        if light != prev {
            out.push(Action::SetLight(light));
        }
        let toast = |title: &str, body: &str| Action::Toast { title: title.into(), body: body.into() };
        match light {
            Light::Green => {
                if reconnected {
                    out.push(toast(&tx.reconectado_titulo, &tx.reconectado_cuerpo_ok));
                } else if prev == Light::Red {
                    out.push(toast(&tx.recuperado_titulo, &tx.recuperado_cuerpo));
                }
                // puede quedar abierta si estaba en rojo antes del corte
                if reconnected || prev == Light::Red {
                    out.push(Action::CloseWarning);
                }
            }
            _ => {
                if reconnected {
                    out.push(toast(&tx.reconectado_titulo, &tx.reconectado_cuerpo_sin_datos));
                } else if prev != Light::Red {
                    out.push(toast(&tx.sin_datos_titulo, &tx.sin_datos_cuerpo));
                }
                out.push(Action::ShowWarning(render(&tx.sin_datos_ventana, &[("hora", clock)])));
            }
        }
        out
    }

    /// Llamar al llegar a `deadline()`: si el corte sigue, se anuncia.
    pub fn on_tick(&mut self, now: Instant) -> Vec<Action> {
        match self.deadline() {
            Some(d) if now >= d => {
                self.announced = true;
                self.light = Light::Yellow;
                vec![
                    Action::SetLight(Light::Yellow),
                    Action::Toast { title: self.tx.sin_conexion_titulo.clone(), body: self.tx.sin_conexion_cuerpo.clone() },
                ]
            }
            _ => Vec::new(),
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        match &self.failing_since {
            Some((t, _)) if !self.announced => Some(*t + GRACE),
            _ => None,
        }
    }

    /// Línea de estado del menú y tooltip.
    pub fn status(&self) -> String {
        let (tx, hora) = (&self.tx, self.last_clock.as_str());
        match (&self.failing_since, self.announced, self.last) {
            (Some((_, since)), true, _) => render(&tx.estado_sin_conexion, &[("desde", since)]),
            (_, _, Last::Nothing) => tx.estado_conectando.clone(),
            (_, _, Last::Fresh(n)) => render(&tx.estado_ok, &[("n", &n.to_string()), ("hora", hora)]),
            (_, _, Last::Stale) => render(&tx.estado_sin_datos, &[("hora", hora)]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::*;
    use Check::*;

    const S: Duration = Duration::from_secs(1);

    fn alerts(t0: Instant, clock: &str) -> Alerts {
        let mut tx = Textos::default();
        tx.resolver("T", 7200);
        Alerts::new(tx, t0, clock)
    }

    fn toast(title: &str, body: &str) -> Action {
        Toast { title: title.into(), body: body.into() }
    }

    fn warning(clock: &str) -> Action {
        ShowWarning(format!("Sin registros nuevos en T en los últimos 2 horas.\nÚltimo chequeo: {clock}"))
    }

    fn sin_datos() -> Action {
        toast("db-monitor: sin datos", "Sin registros nuevos en T en los últimos 2 horas")
    }

    fn sin_conexion() -> Action {
        toast("db-monitor: sin conexión", "No se puede conectar a la base de datos; reintentando")
    }

    /// Alerts en verde (primer chequeo bueno a t0+5s).
    fn green(t0: Instant) -> Alerts {
        let mut a = alerts(t0, "08:00");
        a.on_check(Fresh { n: 1 }, t0 + 5 * S, "08:00");
        a
    }

    /// Caída anunciada (amarillo) que empezó en t1.
    fn announced(a: &mut Alerts, t1: Instant) {
        a.on_check(Failed, t1, "09:00");
        assert!(a.on_tick(t1 + GRACE).contains(&sin_conexion()));
    }

    #[test]
    fn custom_texts() {
        let t0 = Instant::now();
        let toml_text = "[sin_datos]\ntitulo = \"Alerta\"\nventana = \"Nada en {periodo}. ({hora})\"\n[recuperado]\ncuerpo = \"Volvió\"\n";
        let mut tx = Textos::from_toml(toml::from_str(toml_text).unwrap()).unwrap();
        tx.resolver("T", 180);
        let mut a = Alerts::new(tx, t0, "08:00");
        a.on_check(Fresh { n: 1 }, t0 + 5 * S, "08:00");
        let acts = a.on_check(Stale, t0 + 10 * S, "10:40");
        assert_eq!(acts[1], toast("Alerta", "Sin registros nuevos en T en los últimos 3 minutos"));
        assert_eq!(acts[2], ShowWarning("Nada en 3 minutos. (10:40)".into()));
        assert_eq!(a.on_check(Fresh { n: 1 }, t0 + 20 * S, "10:45")[1], toast("db-monitor: OK", "Volvió"));
    }

    #[test]
    fn starts_yellow_connecting() {
        let t0 = Instant::now();
        let a = alerts(t0, "08:00");
        assert_eq!(a.status(), "Conectando…");
        assert_eq!(a.deadline(), Some(t0 + GRACE));
    }

    #[test]
    fn first_fresh_is_silent() {
        let t0 = Instant::now();
        let mut a = alerts(t0, "08:00");
        assert_eq!(a.on_check(Fresh { n: 42 }, t0 + 5 * S, "08:15"), [SetLight(Light::Green)]);
        assert_eq!(a.status(), "OK · 42 registros en 2 horas · chequeo 08:15");
        assert_eq!(a.deadline(), None);
    }

    #[test]
    fn fresh_to_stale() {
        let t0 = Instant::now();
        let mut a = green(t0);
        assert_eq!(
            a.on_check(Stale, t0 + 10 * S, "08:20"),
            [SetLight(Light::Red), sin_datos(), warning("08:20")]
        );
        assert_eq!(a.status(), "Sin registros en 2 horas · chequeo 08:20");
    }

    #[test]
    fn stale_repeats_only_warning() {
        let t0 = Instant::now();
        let mut a = green(t0);
        a.on_check(Stale, t0 + 10 * S, "08:20");
        for (i, clock) in ["08:25", "08:30", "08:35"].iter().enumerate() {
            assert_eq!(a.on_check(Stale, t0 + (20 + i as u32) * S, clock), [warning(clock)]);
        }
    }

    #[test]
    fn stale_to_fresh() {
        let t0 = Instant::now();
        let mut a = green(t0);
        a.on_check(Stale, t0 + 10 * S, "08:20");
        assert_eq!(
            a.on_check(Fresh { n: 3 }, t0 + 20 * S, "08:25"),
            [SetLight(Light::Green), toast("db-monitor: OK", "T volvió a recibir registros"), CloseWarning]
        );
    }

    #[test]
    fn first_stale_at_startup_alerts() {
        let t0 = Instant::now();
        let mut a = alerts(t0, "08:00");
        assert_eq!(
            a.on_check(Stale, t0 + 5 * S, "08:20"),
            [SetLight(Light::Red), sin_datos(), warning("08:20")]
        );
    }

    #[test]
    fn failure_within_grace_is_silent() {
        let t0 = Instant::now();
        let mut a = green(t0);
        let t1 = t0 + 10 * S;
        assert_eq!(a.on_check(Failed, t1, "09:00"), []);
        assert_eq!(a.deadline(), Some(t1 + GRACE));
        assert_eq!(a.on_tick(t1 + 59 * S), []);
        assert_eq!(a.on_check(Fresh { n: 1 }, t1 + 30 * S, "09:01"), []);
        assert_eq!(a.deadline(), None);
    }

    #[test]
    fn failure_after_grace_alerts() {
        let t0 = Instant::now();
        let mut a = green(t0);
        let t1 = t0 + 10 * S;
        a.on_check(Failed, t1, "09:00");
        assert_eq!(a.on_tick(t1 + GRACE), [SetLight(Light::Yellow), sin_conexion()]);
        assert_eq!(a.status(), "Sin conexión · reintentando · desde 09:00");
        assert_eq!(a.on_check(Failed, t1 + 70 * S, "09:02"), []);
        assert_eq!(a.deadline(), None);
    }

    #[test]
    fn startup_without_connection_alerts() {
        let t0 = Instant::now();
        let mut a = alerts(t0, "07:58");
        assert_eq!(a.on_tick(t0 + GRACE), [SetLight(Light::Yellow), sin_conexion()]);
        assert_eq!(a.status(), "Sin conexión · reintentando · desde 07:58");
    }

    #[test]
    fn reconnect_fresh() {
        let t0 = Instant::now();
        let mut a = green(t0);
        announced(&mut a, t0 + 10 * S);
        assert_eq!(
            a.on_check(Fresh { n: 3 }, t0 + 100 * S, "09:05"),
            [
                SetLight(Light::Green),
                toast("db-monitor: conexión restablecida", "Conexión restablecida · T OK"),
                CloseWarning
            ]
        );
    }

    #[test]
    fn reconnect_stale() {
        let t0 = Instant::now();
        let mut a = green(t0);
        announced(&mut a, t0 + 10 * S);
        assert_eq!(
            a.on_check(Stale, t0 + 100 * S, "09:05"),
            [
                SetLight(Light::Red),
                toast("db-monitor: conexión restablecida", "Conexión restablecida · T sin registros en 2 horas"),
                warning("09:05")
            ]
        );
    }

    #[test]
    fn flapping_never_toasts() {
        let t0 = Instant::now();
        let mut a = green(t0);
        let mut actions = Vec::new();
        for i in 0..50u32 {
            let t = t0 + 10 * S + i * 25 * S;
            actions.extend(a.on_check(Failed, t, "10:00"));
            if let Some(d) = a.deadline() {
                if d < t + 20 * S {
                    actions.extend(a.on_tick(d));
                }
            }
            actions.extend(a.on_check(Fresh { n: 1 }, t + 20 * S, "10:00"));
        }
        assert!(!actions.iter().any(|x| matches!(x, Toast { .. })), "{actions:?}");
    }

    #[test]
    fn stale_then_outage_keeps_warning() {
        let t0 = Instant::now();
        let mut a = green(t0);
        a.on_check(Stale, t0 + 10 * S, "08:20");
        let t1 = t0 + 20 * S;
        a.on_check(Failed, t1, "08:21");
        let acts = a.on_tick(t1 + GRACE);
        assert!(acts.contains(&SetLight(Light::Yellow)) && acts.contains(&sin_conexion()), "{acts:?}");
        assert!(!acts.contains(&CloseWarning), "{acts:?}");
    }
}
