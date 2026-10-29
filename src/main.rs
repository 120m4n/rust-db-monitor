// Sin consola en Windows: solo el icono de la bandeja.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::{Duration, Instant};

use db_monitor::alerts::{Action, Alerts, Light};
use db_monitor::monitor::{self, Check};
use db_monitor::{config, desktop, ntfy, textos};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

enum UiEvent {
    Check(Check),
    Menu(MenuId),
    /// El hilo del monitor terminó: `Err` es un error fatal (tabla inválida).
    Stopped(Result<(), String>),
}

fn clock() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

/// `app` es el nombre de `[textos]`, o "db-monitor" si la config aún no se leyó.
fn fatal(app: &str, msg: &str) -> ! {
    desktop::error_box(app, msg);
    exit(1)
}

fn main() {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    // --tarea: lo lanza la tarea programada, que reintenta cada 5 min; si ya corre, sale sin ventana
    let tarea = args.iter().position(|a| a == "--tarea").map(|i| args.remove(i)).is_some();
    // Por defecto junto al .exe: lanzado con doble clic no arranca en el directorio del programa.
    let path = args.into_iter().next().map(PathBuf::from).unwrap_or_else(|| {
        let exe = std::env::current_exe().unwrap_or_default();
        exe.with_file_name("config.toml")
    });
    let cfg = config::load(&path).unwrap_or_else(|e| fatal("db-monitor", &format!("{e:#}")));
    let tx = cfg.textos.clone();
    // se suelta al salir del proceso (exit no corre Drop, pero el SO libera el lock)
    let _lock = match desktop::acquire_lock(&path.parent().unwrap_or(Path::new(".")).join("db-monitor.lock")) {
        Ok(Some(f)) => f,
        Ok(None) => {
            if !tarea {
                desktop::info_box(&tx.app, &tx.ya_en_ejecucion);
            }
            exit(0)
        }
        Err(e) => fatal(&tx.app, &format!("{e:#}")),
    };

    // el guard debe soltarse antes de exit para vaciar el log
    let (file, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(&cfg.logging.dir, "db-monitor.log"));
    tracing_subscriber::registry()
        .with(EnvFilter::try_new(&cfg.logging.level).unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().with_writer(file).with_ansi(false))
        .with(fmt::layer())
        .init();

    desktop::setup_toasts(&tx.app);

    let (db, f) = (&cfg.database, &cfg.freshness);
    tracing::info!(
        "db-monitor v{} | {:?} {}{}{} | `{}` cada {}s, ventana {}s",
        env!("CARGO_PKG_VERSION"),
        db.kind,
        db.host,
        if db.host.is_empty() { "" } else { "/" },
        db.dbname,
        f.table,
        f.interval_secs,
        f.window_secs,
    );

    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UiEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let menu_proxy = std::sync::Mutex::new(event_loop.create_proxy());
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = menu_proxy.lock().map(|p| p.send_event(UiEvent::Menu(e.id)));
    }));

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let proxy = event_loop.create_proxy();
    let monitor_cfg = cfg.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
        // Ctrl-C solo sirve con terminal (desarrollo); en Windows se sale con el menú
        let stop = async {
            // si no se puede registrar Ctrl-C, ese brazo no debe terminar el programa
            let ctrl_c = async {
                if tokio::signal::ctrl_c().await.is_err() {
                    std::future::pending::<()>().await
                }
            };
            tokio::select! { _ = stop_rx => {}, _ = ctrl_c => {} }
        };
        let r = rt.block_on(monitor::run(monitor_cfg, stop, |c| {
            let _ = proxy.send_event(UiEvent::Check(c));
        }));
        // sin esto, Drop del runtime espera sin límite a un spawn_blocking colgado (consulta lenta)
        rt.shutdown_timeout(Duration::from_secs(2));
        let _ = proxy.send_event(UiEvent::Stopped(r.map_err(|e| format!("{e:#}"))));
    });

    let mut alerts = Alerts::new(tx.clone(), Instant::now(), &clock());
    let mut warning = desktop::Warning::new(&tx.sin_datos_titulo);
    let status_item = MenuItem::new(alerts.status(), false, None);
    let logs_item = MenuItem::new("Abrir logs", true, None);
    let quit_item = MenuItem::new("Salir", true, None);
    let menu = Menu::new();
    menu.append_items(&[&status_item, &PredefinedMenuItem::separator(), &logs_item, &quit_item])
        .unwrap_or_else(|e| fatal(&tx.app, &format!("no se pudo crear el menú: {e}")));
    let log_dir = cfg.logging.dir.clone();
    let ntfy = cfg.ntfy.clone();
    let mut tray: Option<TrayIcon> = None;
    let mut stop_tx = Some(stop_tx);
    let mut guard = Some(guard);

    event_loop.run(move |event, _, control_flow| {
        let actions = match event {
            // en macOS el icono no puede crearse antes de que arranque el bucle
            Event::NewEvents(StartCause::Init) => {
                let t = TrayIconBuilder::new()
                    .with_menu(Box::new(menu.clone()))
                    .with_tooltip(alerts.status())
                    .with_icon(desktop::icon(Light::Yellow))
                    .build()
                    .unwrap_or_else(|e| fatal(&tx.app, &format!("no se pudo crear el icono de bandeja: {e}")));
                tray = Some(t);
                // sin consola, un usuario nuevo no sabe que arrancó: se le dice dónde está el icono
                let donde = if cfg!(target_os = "macos") { "la barra de menú" } else { "la bandeja del sistema" };
                let body = textos::render(&tx.inicio_cuerpo, &[("donde", donde)]);
                tracing::info!("aviso: {} · {body}", tx.inicio_titulo);
                desktop::toast(&tx.inicio_titulo, &body);
                Vec::new()
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => alerts.on_tick(Instant::now()),
            Event::UserEvent(UiEvent::Check(c)) => alerts.on_check(c, Instant::now(), &clock()),
            Event::UserEvent(UiEvent::Menu(id)) => {
                if id == *logs_item.id() {
                    desktop::open_dir(&log_dir);
                } else if id == *quit_item.id() {
                    if let Some(tx) = stop_tx.take() {
                        let _ = tx.send(());
                    }
                }
                Vec::new()
            }
            Event::UserEvent(UiEvent::Stopped(r)) => {
                warning.close();
                tray = None;
                let code = match r {
                    Ok(()) => 0,
                    Err(msg) => {
                        tracing::error!("{msg}");
                        desktop::error_box(&tx.app, &msg);
                        1
                    }
                };
                drop(guard.take());
                exit(code);
            }
            _ => return,
        };
        // rojo o amarillo en este lote: aviso urgente para ntfy
        let urgent = actions.iter().any(|a| matches!(a, Action::SetLight(Light::Red | Light::Yellow)));
        for a in actions {
            match a {
                Action::SetLight(l) => {
                    if let Some(t) = &tray {
                        let _ = t.set_icon(Some(desktop::icon(l)));
                    }
                }
                Action::Toast { title, body } => {
                    tracing::info!("aviso: {title} · {body}");
                    desktop::toast(&title, &body);
                    if let Some(n) = &ntfy {
                        ntfy::send(n, &title, &body, urgent);
                    }
                }
                Action::ShowWarning(text) => warning.show(&text),
                Action::CloseWarning => warning.close(),
            }
        }
        let status = alerts.status();
        status_item.set_text(&status);
        if let Some(t) = &tray {
            let _ = t.set_tooltip(Some(&status));
        }
        *control_flow = match alerts.deadline() {
            Some(d) => ControlFlow::WaitUntil(d),
            None => ControlFlow::Wait,
        };
    });
}
