//! Envío opcional de los avisos a ntfy (https://ntfy.sh o un servidor propio).
//! Usa el `curl` del sistema (incluido en Windows 10/11 y macOS) para no sumar una pila HTTP/TLS;
//! en Windows además usa el almacén de certificados del sistema (proxies corporativos).

use crate::config::Ntfy;

/// Argumentos de `curl` para publicar en `n.server` con el formato JSON de ntfy.
/// `priority`: 4 = rojo o amarillo, 3 = normal, 2 = baja. El token nunca va aquí (lo vería cualquier
/// proceso del equipo en la línea de comandos): va por stdin con `--config -` (ver `curl_config`).
pub fn curl_args(n: &Ntfy, title: &str, body: &str, priority: u8) -> Vec<String> {
    let tag = if priority >= 4 { "warning" } else { "white_check_mark" };
    let json = format!(
        r#"{{"topic":{},"title":{},"message":{},"priority":{priority},"tags":["{tag}"]}}"#,
        quote(&n.topic),
        quote(title),
        quote(body)
    );
    let mut a: Vec<String> = ["-sS", "--fail", "--max-time", "15", "-H", "Content-Type: application/json"]
        .map(String::from)
        .into();
    if !n.token.is_empty() {
        a.extend(["--config".into(), "-".into()]);
    }
    a.extend(["--data-binary".into(), json, n.server.trim_end_matches('/').into()]);
    a
}

/// Config de curl para stdin con la cabecera del token; `None` sin token.
pub fn curl_config(n: &Ntfy) -> Option<String> {
    if n.token.is_empty() {
        return None;
    }
    let v = format!("Authorization: Bearer {}", n.token).replace('\\', "\\\\").replace('"', "\\\"");
    Some(format!("header = \"{v}\"\n"))
}

/// Cadena JSON: solo hace falta escapar `"`, `\` y los caracteres de control.
fn quote(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Publica en un hilo propio; un fallo (sin red, curl ausente) solo se loguea, sin reintento.
pub fn send(n: &Ntfy, title: &str, body: &str, priority: u8) {
    use std::io::Write;
    use std::process::Stdio;
    let args = curl_args(n, title, body, priority);
    let config = curl_config(n);
    std::thread::spawn(move || {
        let mut cmd = std::process::Command::new("curl");
        cmd.args(&args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            // sin esto, cada envío abre una consola un instante (el .exe no tiene consola)
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let r = cmd.spawn().and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(config.as_deref().unwrap_or("").as_bytes())?;
            } // stdin cerrado aquí: curl deja de esperar la config
            child.wait_with_output()
        });
        match r {
            Ok(o) if o.status.success() => tracing::debug!("ntfy: enviado"),
            Ok(o) => tracing::warn!("ntfy: falló ({}): {}", o.status, String::from_utf8_lossy(&o.stderr).trim()),
            Err(e) => tracing::warn!("ntfy: no se pudo ejecutar curl: {e}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(token: &str) -> Ntfy {
        Ntfy { topic: "sala-control".into(), server: "https://ntfy.sh/".into(), token: token.into(), heartbeat: None }
    }

    #[test]
    fn json_post_with_priority_and_escaping() {
        let a = curl_args(&n(""), "db-monitor: sin conexión", "dice \"hola\"\\\nfin", 4);
        assert_eq!(a.last().unwrap(), "https://ntfy.sh");
        assert!(!a.iter().any(|x| x.starts_with("Authorization")));
        let json = &a[a.iter().position(|x| x == "--data-binary").unwrap() + 1];
        assert_eq!(
            json,
            r#"{"topic":"sala-control","title":"db-monitor: sin conexión","message":"dice \"hola\"\\\nfin","priority":4,"tags":["warning"]}"#
        );
        assert!(a.contains(&"--fail".to_string()));
    }

    #[test]
    fn recovery_is_normal_priority_and_token_is_sent() {
        let a = curl_args(&n("tk_123"), "db-monitor: OK", "T volvió", 3);
        assert_eq!(curl_config(&n("tk_123")).unwrap(), "header = \"Authorization: Bearer tk_123\"\n");
        let json = &a[a.iter().position(|x| x == "--data-binary").unwrap() + 1];
        assert!(json.ends_with(r#""priority":3,"tags":["white_check_mark"]}"#), "{json}");
    }

    #[test]
    fn token_nunca_en_args() {
        let a = curl_args(&n("tk_\"x"), "t", "b", 4);
        assert!(!a.iter().any(|x| x.contains("tk_")), "{a:?}");
        assert!(a.windows(2).any(|w| w == ["--config", "-"]));
        assert_eq!(curl_config(&n("tk_\"x")).unwrap(), "header = \"Authorization: Bearer tk_\\\"x\"\n");
        assert!(curl_config(&n("")).is_none());
        assert!(!curl_args(&n(""), "t", "b", 3).iter().any(|x| x == "--config"));
    }
}
