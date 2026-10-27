//! Envío opcional de los avisos a ntfy (https://ntfy.sh o un servidor propio).
//! Usa el `curl` del sistema (incluido en Windows 10/11 y macOS) para no sumar una pila HTTP/TLS;
//! en Windows además usa el almacén de certificados del sistema (proxies corporativos).

use crate::config::Ntfy;

/// Argumentos de `curl` para publicar en `n.server` con el formato JSON de ntfy.
/// `urgent`: prioridad alta (rojo o amarillo); si no, normal.
pub fn curl_args(n: &Ntfy, title: &str, body: &str, urgent: bool) -> Vec<String> {
    let (priority, tag) = if urgent { (4, "warning") } else { (3, "white_check_mark") };
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
        a.extend(["-H".into(), format!("Authorization: Bearer {}", n.token)]);
    }
    a.extend(["--data-binary".into(), json, n.server.trim_end_matches('/').into()]);
    a
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

/// Publica en un hilo propio; un fallo (sin red, curl ausente) solo se loguea.
pub fn send(n: &Ntfy, title: &str, body: &str, urgent: bool) {
    let args = curl_args(n, title, body, urgent);
    std::thread::spawn(move || {
        let mut cmd = std::process::Command::new("curl");
        cmd.args(&args);
        #[cfg(windows)]
        {
            // sin esto, cada envío abre una consola un instante (el .exe no tiene consola)
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        match cmd.output() {
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
        Ntfy { topic: "sala-control".into(), server: "https://ntfy.sh/".into(), token: token.into() }
    }

    #[test]
    fn json_post_with_priority_and_escaping() {
        let a = curl_args(&n(""), "db-monitor: sin conexión", "dice \"hola\"\\\nfin", true);
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
        let a = curl_args(&n("tk_123"), "db-monitor: OK", "T volvió", false);
        assert!(a.contains(&"Authorization: Bearer tk_123".to_string()));
        let json = &a[a.iter().position(|x| x == "--data-binary").unwrap() + 1];
        assert!(json.ends_with(r#""priority":3,"tags":["white_check_mark"]}"#), "{json}");
    }
}
