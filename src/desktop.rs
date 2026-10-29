//! Código de plataforma: icono, toasts, ventanas nativas y bloqueo de instancia única.
//! Windows es el objetivo; macOS se soporta para desarrollar.

use std::path::Path;

use crate::alerts::Light;

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

/// Logo de 32×32 con el color de estado. Son RGBA crudos generados por `assets/build-icons.sh`:
/// así no hace falta una dependencia para decodificar PNG.
pub fn icon(light: Light) -> tray_icon::Icon {
    const N: u32 = 32;
    let rgba: &[u8] = match light {
        Light::Green => include_bytes!("../assets/tray/green.rgba"),
        Light::Yellow => include_bytes!("../assets/tray/yellow.rgba"),
        Light::Red => include_bytes!("../assets/tray/red.rgba"),
    };
    tray_icon::Icon::from_rgba(rgba.to_vec(), N, N).expect("valid rgba icon")
}

/// Notificación del sistema; si falla (permisos, modo concentración) solo se loguea.
/// En su propio hilo: en macOS `show()` es bloqueante y congelaría el bucle de eventos.
/// En Windows firma con `APP_ID` solo si `setup_toasts` encontró su acceso directo; si no, como PowerShell.
pub fn toast(title: &str, body: &str) {
    let mut n = notify_rust::Notification::new();
    n.summary(title).body(body);
    #[cfg(windows)]
    if imp::OWN_SENDER.get().is_some() {
        n.app_id(APP_ID);
    }
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

/// AppUserModelID de los toasts en Windows. Va en un acceso directo del menú Inicio: Windows solo muestra
/// toasts de un AppUserModelID propio si una app "instalada" lo declara así; registrarlo en el registro
/// no basta y el toast se descarta sin error (regresión de eabae29).
pub const APP_ID: &str = "DbMonitor";

pub use imp::{error_box, info_box, setup_toasts, Warning};

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetDlgItem, MessageBoxW, PostMessageW, SetWindowTextW, MB_ICONERROR, MB_ICONINFORMATION,
        MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, WM_CLOSE,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn message_box(text: &str, title: &str, style: u32) {
        let (t, c) = (wide(text), wide(title));
        unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), style) };
    }

    pub fn error_box(title: &str, msg: &str) {
        message_box(msg, title, MB_OK | MB_ICONERROR | MB_SETFOREGROUND);
    }

    pub(super) static OWN_SENDER: std::sync::OnceLock<()> = std::sync::OnceLock::new();

    /// Remitente propio de los toasts: `name` (el nombre del acceso directo es lo que muestra Windows).
    /// Si el acceso directo ya existe con `APP_ID`, se usa desde ya. Si no, se crea y esta ejecución sigue
    /// como PowerShell: Windows tarda en reconocer un acceso directo nuevo y el primer toast se perdería.
    // ponytail: si cambia [textos].app queda el acceso directo viejo; install-task.ps1 -Desinstalar los borra
    pub fn setup_toasts(name: &str) {
        let Some(dir) = std::env::var_os("APPDATA") else { return };
        let file: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '-' } else { c }).collect();
        let lnk = std::path::PathBuf::from(dir)
            .join(r"Microsoft\Windows\Start Menu\Programs")
            .join(format!("{}.lnk", file.trim()));
        let Ok(exe) = std::env::current_exe() else { return };
        // COM en un hilo propio: no interfiere con la inicialización OLE del bucle de eventos
        let l = lnk.clone();
        let r = std::thread::spawn(move || unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            if atajo::app_id(&l).ok().as_deref() == Some(super::APP_ID) {
                return Ok(true);
            }
            atajo::crear(&l, &exe, super::APP_ID).map(|()| false)
        })
        .join();
        match r {
            Ok(Ok(true)) => {
                let _ = OWN_SENDER.set(());
                tracing::info!("toasts: remitente \"{name}\" ({})", lnk.display());
            }
            Ok(Ok(false)) => tracing::info!(
                "toasts: creado {}; el remitente \"{name}\" se usa desde el próximo arranque (ahora: PowerShell)",
                lnk.display()
            ),
            Ok(Err(e)) => tracing::warn!("toasts: no se pudo crear {} ({e}); remitente PowerShell", lnk.display()),
            Err(_) => tracing::warn!("toasts: fallo al crear el acceso directo; remitente PowerShell"),
        }
    }

    /// Acceso directo `.lnk` con la propiedad `System.AppUserModel.ID`.
    mod atajo {
        use std::mem::ManuallyDrop;
        use std::path::Path;

        use windows::core::{Interface, Result, BSTR, HSTRING};
        use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
        use windows::Win32::System::Com::StructuredStorage::{
            PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
        };
        use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ};
        use windows::Win32::System::Variant::VT_LPWSTR;
        use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
        use windows::Win32::UI::Shell::{IShellLinkW, SHStrDupW, ShellLink};

        fn nuevo() -> Result<IShellLinkW> {
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
        }

        /// AppUserModelID de un acceso directo existente.
        pub fn app_id(lnk: &Path) -> Result<String> {
            let link = nuevo()?;
            unsafe {
                link.cast::<IPersistFile>()?.Load(&HSTRING::from(lnk), STGM_READ)?;
                let v = link.cast::<IPropertyStore>()?.GetValue(&PKEY_AppUserModel_ID)?;
                Ok(BSTR::try_from(&v)?.to_string())
            }
        }

        pub fn crear(lnk: &Path, exe: &Path, app_id: &str) -> Result<()> {
            let link = nuevo()?;
            unsafe {
                link.SetPath(&HSTRING::from(exe))?;
                if let Some(dir) = exe.parent() {
                    link.SetWorkingDirectory(&HSTRING::from(dir))?;
                }
                // VT_LPWSTR como InitPropVariantFromString; la cadena la libera PROPVARIANT al soltarse
                let v = PROPVARIANT {
                    Anonymous: PROPVARIANT_0 {
                        Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                            vt: VT_LPWSTR,
                            wReserved1: 0,
                            wReserved2: 0,
                            wReserved3: 0,
                            Anonymous: PROPVARIANT_0_0_0 { pwszVal: SHStrDupW(&HSTRING::from(app_id))? },
                        }),
                    },
                };
                let store = link.cast::<IPropertyStore>()?;
                store.SetValue(&PKEY_AppUserModel_ID, &v)?;
                store.Commit()?;
                link.cast::<IPersistFile>()?.Save(&HSTRING::from(lnk), true)
            }
        }
    }

    pub fn info_box(title: &str, msg: &str) {
        message_box(msg, title, MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND);
    }

    /// Control de texto de un `MessageBox`.
    const MESSAGE_TEXT_ID: i32 = 0xFFFF;

    /// Ventana de advertencia única: `MessageBoxW` en un hilo propio para no bloquear el bucle de eventos.
    /// Se la encuentra por su título.
    // ponytail: FindWindowW busca en todo el escritorio; otra ventana con el mismo título la confundiría
    pub struct Warning {
        title: String,
        open: Arc<AtomicBool>,
    }

    impl Warning {
        pub fn new(title: &str) -> Self {
            Warning { title: title.into(), open: Arc::new(AtomicBool::new(false)) }
        }

        fn find(&self) -> HWND {
            let title = wide(&self.title);
            unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) }
        }

        pub fn show(&mut self, text: &str) {
            if self.open.swap(true, Ordering::SeqCst) {
                // ya abierta: se actualiza el texto (hora del último chequeo)
                let hwnd = self.find();
                if !hwnd.is_null() {
                    let t = wide(text);
                    unsafe {
                        let label = GetDlgItem(hwnd, MESSAGE_TEXT_ID);
                        if !label.is_null() {
                            SetWindowTextW(label, t.as_ptr());
                        }
                    }
                }
                return;
            }
            let (open, text, title) = (self.open.clone(), text.to_string(), self.title.clone());
            std::thread::spawn(move || {
                message_box(&text, &title, MB_OK | MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND);
                open.store(false, Ordering::SeqCst);
            });
        }

        pub fn close(&mut self) {
            if !self.open.load(Ordering::SeqCst) {
                return;
            }
            let hwnd = self.find();
            if !hwnd.is_null() {
                unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::process::{Child, Command};

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

    pub fn error_box(title: &str, msg: &str) {
        let _ = dialog(msg, title, "stop").status();
    }

    /// En macOS el remitente lo decide el sistema.
    pub fn setup_toasts(_name: &str) {}

    pub fn info_box(title: &str, msg: &str) {
        let _ = dialog(msg, title, "note").status();
    }

    /// Ventana de advertencia única: proceso `osascript`; abierta mientras el proceso vive.
    pub struct Warning {
        title: String,
        child: Option<Child>,
    }

    impl Warning {
        pub fn new(title: &str) -> Self {
            Warning { title: title.into(), child: None }
        }

        pub fn show(&mut self, text: &str) {
            if let Some(c) = &mut self.child {
                // ponytail: solo desarrollo; el texto (la hora) no se actualiza mientras esté abierta
                if matches!(c.try_wait(), Ok(None)) {
                    return;
                }
            }
            match dialog(text, &self.title, "caution").spawn() {
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
    fn tray_icons_are_valid_rgba() {
        for l in [Light::Green, Light::Yellow, Light::Red] {
            icon(l);
        }
    }

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
