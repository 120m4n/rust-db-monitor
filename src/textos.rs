//! Textos visibles (toasts, ventana de advertencia, menú) personalizables con `[textos]` en config.toml.
//! Son plantillas con `{variables}`: `{app}`, `{tabla}` y `{periodo}` valen en todas (se resuelven al
//! cargar la config); el resto solo donde se indica en `campos()`.

use anyhow::bail;

const GLOBALES: &[&str] = &["app", "tabla", "periodo"];

#[derive(Clone, Debug)]
pub struct Textos {
    pub app: String,
    pub sin_datos_titulo: String,
    pub sin_datos_cuerpo: String,
    pub sin_datos_ventana: String,
    pub recuperado_titulo: String,
    pub recuperado_cuerpo: String,
    pub sin_conexion_titulo: String,
    pub sin_conexion_cuerpo: String,
    pub reconectado_titulo: String,
    pub reconectado_cuerpo_ok: String,
    pub reconectado_cuerpo_sin_datos: String,
    pub inicio_titulo: String,
    pub inicio_cuerpo: String,
    pub estado_conectando: String,
    pub estado_ok: String,
    pub estado_sin_datos: String,
    pub estado_sin_conexion: String,
    pub ya_en_ejecucion: String,
}

impl Default for Textos {
    fn default() -> Self {
        let s = |x: &str| x.to_string();
        Textos {
            app: s("db-monitor"),
            sin_datos_titulo: s("{app}: sin datos"),
            sin_datos_cuerpo: s("Sin registros nuevos en {tabla} en los últimos {periodo}"),
            sin_datos_ventana: s("Sin registros nuevos en {tabla} en los últimos {periodo}.\nÚltimo chequeo: {hora}"),
            recuperado_titulo: s("{app}: OK"),
            recuperado_cuerpo: s("{tabla} volvió a recibir registros"),
            sin_conexion_titulo: s("{app}: sin conexión"),
            sin_conexion_cuerpo: s("No se puede conectar a la base de datos; reintentando"),
            reconectado_titulo: s("{app}: conexión restablecida"),
            reconectado_cuerpo_ok: s("Conexión restablecida · {tabla} OK"),
            reconectado_cuerpo_sin_datos: s("Conexión restablecida · {tabla} sin registros en {periodo}"),
            inicio_titulo: s("{app} en ejecución"),
            inicio_cuerpo: s("Vigilando {tabla}. El icono está en {donde}."),
            estado_conectando: s("Conectando…"),
            estado_ok: s("OK · {n} registros en {periodo} · chequeo {hora}"),
            estado_sin_datos: s("Sin registros en {periodo} · chequeo {hora}"),
            estado_sin_conexion: s("Sin conexión · reintentando · desde {desde}"),
            ya_en_ejecucion: s("{app} ya está en ejecución"),
        }
    }
}

type Campo<'a> = (&'static str, &'a mut String, &'static [&'static str]);

impl Textos {
    /// Clave en `[textos]`, texto y variables propias (además de las globales).
    fn campos(&mut self) -> [Campo<'_>; 18] {
        [
            ("app", &mut self.app, &[]),
            ("sin_datos.titulo", &mut self.sin_datos_titulo, &[]),
            ("sin_datos.cuerpo", &mut self.sin_datos_cuerpo, &[]),
            ("sin_datos.ventana", &mut self.sin_datos_ventana, &["hora"]),
            ("recuperado.titulo", &mut self.recuperado_titulo, &[]),
            ("recuperado.cuerpo", &mut self.recuperado_cuerpo, &[]),
            ("sin_conexion.titulo", &mut self.sin_conexion_titulo, &[]),
            ("sin_conexion.cuerpo", &mut self.sin_conexion_cuerpo, &[]),
            ("reconectado.titulo", &mut self.reconectado_titulo, &[]),
            ("reconectado.cuerpo_ok", &mut self.reconectado_cuerpo_ok, &[]),
            ("reconectado.cuerpo_sin_datos", &mut self.reconectado_cuerpo_sin_datos, &[]),
            ("inicio.titulo", &mut self.inicio_titulo, &[]),
            ("inicio.cuerpo", &mut self.inicio_cuerpo, &["donde"]),
            ("estado.conectando", &mut self.estado_conectando, &[]),
            ("estado.ok", &mut self.estado_ok, &["n", "hora"]),
            ("estado.sin_datos", &mut self.estado_sin_datos, &["hora"]),
            ("estado.sin_conexion", &mut self.estado_sin_conexion, &["desde"]),
            ("ya_en_ejecucion", &mut self.ya_en_ejecucion, &[]),
        ]
    }

    /// Aplica `[textos]` sobre los textos por defecto. Rechaza claves y `{variables}` desconocidas:
    /// mejor no arrancar que mostrar un aviso a medias en la sala de control.
    pub fn from_toml(t: toml::Table) -> anyhow::Result<Self> {
        let mut x = Textos::default();
        let mut campos = x.campos();
        let mut pares = Vec::new();
        aplanar("", t, &mut pares);
        for (clave, valor) in pares {
            let Some((_, texto, _)) = campos.iter_mut().find(|c| c.0 == clave) else {
                bail!("textos.{clave}: clave desconocida (ver README, sección [textos])");
            };
            let toml::Value::String(s) = valor else { bail!("textos.{clave}: debe ser un texto entre comillas") };
            **texto = s;
        }
        for (clave, texto, propias) in &campos {
            for v in variables(texto) {
                // {app} no puede usarse dentro del propio app
                let ok = propias.contains(&v) || (*clave != "app" && GLOBALES.contains(&v));
                if !ok {
                    let mut admite: Vec<_> = propias.iter().map(|p| format!("{{{p}}}")).collect();
                    if *clave != "app" {
                        admite.splice(0..0, GLOBALES.iter().map(|g| format!("{{{g}}}")));
                    }
                    let admite = if admite.is_empty() { "ninguna".into() } else { admite.join(" ") };
                    bail!("textos.{clave}: variable {{{v}}} desconocida; admite: {admite}");
                }
            }
        }
        drop(campos);
        Ok(x)
    }

    /// Sustituye las variables globales; se llama una vez al cargar la config.
    pub fn resolver(&mut self, tabla: &str, window_secs: u64) {
        let (app, periodo) = (self.app.clone(), human_window(window_secs));
        for (clave, texto, _) in self.campos() {
            if clave != "app" {
                *texto = render(texto, &[("app", &app), ("tabla", tabla), ("periodo", &periodo)]);
            }
        }
    }
}

/// `{"estado": {"ok": "x"}}` → `("estado.ok", "x")`.
fn aplanar(prefijo: &str, t: toml::Table, out: &mut Vec<(String, toml::Value)>) {
    for (k, v) in t {
        let clave = if prefijo.is_empty() { k } else { format!("{prefijo}.{k}") };
        match v {
            toml::Value::Table(sub) => aplanar(&clave, sub, out),
            v => out.push((clave, v)),
        }
    }
}

/// Nombres entre llaves: `"a {x} b {y}"` → `["x", "y"]`. Una `{` sin cerrar es texto normal.
fn variables(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        rest = &rest[i + 1..];
        match rest.find('}') {
            Some(j) => {
                out.push(&rest[..j]);
                rest = &rest[j + 1..];
            }
            None => break,
        }
    }
    out
}

pub fn render(plantilla: &str, vars: &[(&str, &str)]) -> String {
    let mut s = plantilla.to_string();
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// `7200` → `2 horas`, `5400` → `90 minutos`, `45` → `45 segundos`.
pub fn human_window(secs: u64) -> String {
    let (n, uno, varios) = match secs {
        s if s % 3600 == 0 => (s / 3600, "hora", "horas"),
        s if s % 60 == 0 => (s / 60, "minuto", "minutos"),
        s => (s, "segundo", "segundos"),
    };
    format!("{n} {}", if n == 1 { uno } else { varios })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn de(toml_text: &str) -> anyhow::Result<Textos> {
        Textos::from_toml(toml::from_str(toml_text).unwrap())
    }

    #[test]
    fn human_window_units() {
        assert_eq!(human_window(7200), "2 horas");
        assert_eq!(human_window(3600), "1 hora");
        assert_eq!(human_window(5400), "90 minutos");
        assert_eq!(human_window(180), "3 minutos");
        assert_eq!(human_window(60), "1 minuto");
        assert_eq!(human_window(45), "45 segundos");
        assert_eq!(human_window(1), "1 segundo");
    }

    #[test]
    fn defaults_resolve_globals() {
        let mut t = Textos::default();
        t.resolver("T", 180);
        assert_eq!(t.sin_datos_titulo, "db-monitor: sin datos");
        assert_eq!(t.sin_datos_ventana, "Sin registros nuevos en T en los últimos 3 minutos.\nÚltimo chequeo: {hora}");
        assert_eq!(t.ya_en_ejecucion, "db-monitor ya está en ejecución");
    }

    #[test]
    fn overlay_keeps_unset_defaults() {
        let mut t = de("app = \"SCADA\"\n[sin_datos]\ntitulo = \"Alerta {app}\"\n").unwrap();
        t.resolver("T", 7200);
        assert_eq!(t.sin_datos_titulo, "Alerta SCADA");
        assert_eq!(t.sin_datos_cuerpo, "Sin registros nuevos en T en los últimos 2 horas");
        assert_eq!(t.recuperado_titulo, "SCADA: OK");
    }

    #[test]
    fn rejects_unknown_key() {
        let e = de("[sin_datos]\ntitlo = \"x\"\n").unwrap_err();
        assert!(e.to_string().contains("textos.sin_datos.titlo: clave desconocida"), "{e}");
        assert!(de("[nada]\ntitulo = \"x\"\n").is_err());
    }

    #[test]
    fn rejects_unknown_or_misplaced_variable() {
        let e = de("[sin_datos]\ntitulo = \"x {hora}\"\n").unwrap_err();
        assert!(e.to_string().contains("variable {hora} desconocida; admite: {app} {tabla} {periodo}"), "{e}");
        assert!(de("[estado]\nok = \"{n} {hora} {fecha}\"\n").is_err());
        assert!(de("app = \"{app}\"\n").is_err());
        assert!(de("[estado]\nok = \"{n} {hora} {tabla}\"\n").is_ok());
        // una llave sin cerrar es texto
        assert!(de("[sin_datos]\ntitulo = \"a { b\"\n").is_ok());
    }

    #[test]
    fn rejects_non_string() {
        assert!(de("[sin_datos]\ntitulo = 3\n").is_err());
    }
}
