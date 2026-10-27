//! Código de plataforma: icono, toasts, ventanas nativas y bloqueo de instancia única.
//! Windows es el objetivo; macOS se soporta para desarrollar.

use std::path::Path;

use crate::alerts::Light;

pub const WARNING_TITLE: &str = "db-monitor: sin datos";

/// Toma el lock exclusivo de `path`; `None` si otro proceso ya lo tiene. Se libera al soltar el `File`
/// (o cuando muere el proceso).
pub fn acquire_lock(path: &Path) -> anyhow::Result<Option<std::fs::File>> {
    use anyhow::Context;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .with_context(|| format!("cannot open lock {}", path.display()))?;
    match f.try_lock() {
        Ok(()) => Ok(Some(f)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e).with_context(|| format!("cannot lock {}", path.display())),
    }
}

/// Círculo de color de 32×32 sobre fondo transparente.
pub fn icon(light: Light) -> tray_icon::Icon {
    const N: u32 = 32;
    let [r, g, b] = match light {
        Light::Green => [0x2E, 0x7D, 0x32],
        Light::Yellow => [0xF9, 0xA8, 0x25],
        Light::Red => [0xC6, 0x28, 0x28],
    };
    let c = (N as f32 - 1.0) / 2.0;
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let inside = (x as f32 - c).hypot(y as f32 - c) <= c;
            rgba.extend_from_slice(&[r, g, b, if inside { 0xFF } else { 0 }]);
        }
    }
    tray_icon::Icon::from_rgba(rgba, N, N).expect("valid rgba icon")
}

/// Notificación del sistema; si falla (permisos, modo concentración) solo se loguea.
/// En su propio hilo: en macOS `show()` es bloqueante y congelaría el bucle de eventos.
pub fn toast(title: &str, body: &str) {
    let mut n = notify_rust::Notification::new();
    n.summary(title).body(body);
    std::thread::spawn(move || {
        if let Err(e) = n.show() {
            tracing::warn!("no se pudo mostrar la notificación: {e}");
        }
    });
}

pub fn open_dir(path: &Path) {
    let cmd = if cfg!(windows) { "explorer" } else { "open" };
    if let Err(e) = std::process::Command::new(cmd).arg(path).spawn() {
        tracing::warn!("no se pudo abrir {}: {e}", path.display());
    }
}

pub use imp::{error_box, info_box, Warning};

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, MessageBoxW, PostMessageW, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK,
        MB_SETFOREGROUND, MB_TOPMOST, WM_CLOSE,
    };

    use super::WARNING_TITLE;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn message_box(text: &str, title: &str, style: u32) {
        let (t, c) = (wide(text), wide(title));
        unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), style) };
    }

    pub fn error_box(msg: &str) {
        message_box(msg, "db-monitor", MB_OK | MB_ICONERROR | MB_SETFOREGROUND);
    }

    pub fn info_box(msg: &str) {
        message_box(msg, "db-monitor", MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND);
    }

    /// Ventana de advertencia única: `MessageBoxW` en un hilo propio para no bloquear el bucle de eventos.
    pub struct Warning {
        open: Arc<AtomicBool>,
    }

    impl Warning {
        pub fn new() -> Self {
            Warning { open: Arc::new(AtomicBool::new(false)) }
        }

        pub fn show(&mut self, text: &str) {
            if self.open.swap(true, Ordering::SeqCst) {
                return;
            }
            let (open, text) = (self.open.clone(), text.to_string());
            std::thread::spawn(move || {
                message_box(&text, WARNING_TITLE, MB_OK | MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND);
                open.store(false, Ordering::SeqCst);
            });
        }

        pub fn close(&mut self) {
            if !self.open.load(Ordering::SeqCst) {
                return;
            }
            let title = wide(WARNING_TITLE);
            unsafe {
                let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
                if !hwnd.is_null() {
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::process::{Child, Command};

    use super::WARNING_TITLE;

    fn dialog(text: &str, title: &str, icon: &str) -> Command {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            r#"display dialog "{}" with title "{}" buttons {{"Aceptar"}} default button 1 with icon {icon}"#,
            esc(text),
            esc(title)
        );
        let mut c = Command::new("osascript");
        c.arg("-e").arg(script);
        c
    }

    pub fn error_box(msg: &str) {
        let _ = dialog(msg, "db-monitor", "stop").status();
    }

    pub fn info_box(msg: &str) {
        let _ = dialog(msg, "db-monitor", "note").status();
    }

    /// Ventana de advertencia única: proceso `osascript`; abierta mientras el proceso vive.
    pub struct Warning {
        child: Option<Child>,
    }

    impl Warning {
        pub fn new() -> Self {
            Warning { child: None }
        }

        pub fn show(&mut self, text: &str) {
            if let Some(c) = &mut self.child {
                if matches!(c.try_wait(), Ok(None)) {
                    return;
                }
            }
            match dialog(text, WARNING_TITLE, "caution").spawn() {
                Ok(c) => self.child = Some(c),
                Err(e) => tracing::warn!("no se pudo abrir la ventana de advertencia: {e}"),
            }
        }

        pub fn close(&mut self) {
            if let Some(mut c) = self.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_and_released_on_drop() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("db-monitor.lock");
        let a = acquire_lock(&p).unwrap().expect("first lock");
        assert!(acquire_lock(&p).unwrap().is_none());
        drop(a);
        assert!(acquire_lock(&p).unwrap().is_some());
    }
}
