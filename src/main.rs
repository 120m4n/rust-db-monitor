use std::path::PathBuf;
use std::process::ExitCode;

use db_monitor::{config, monitor};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn main() -> ExitCode {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
    let code = rt.block_on(app());
    // sin esto, Drop del runtime espera sin límite a un spawn_blocking colgado (consulta lenta)
    rt.shutdown_timeout(std::time::Duration::from_secs(2));
    code
}

async fn app() -> ExitCode {
    // Por defecto junto al .exe: un servicio de Windows no arranca en el directorio del programa.
    let path = std::env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        let exe = std::env::current_exe().unwrap_or_default();
        exe.with_file_name("config.toml")
    });
    let cfg = match config::load(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e:#}");
            return ExitCode::FAILURE;
        }
    };

    // el guard debe vivir hasta el final de main para vaciar el log al apagar
    let (file, _guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(&cfg.logging.dir, "db-monitor.log"));
    tracing_subscriber::registry()
        .with(EnvFilter::try_new(&cfg.logging.level).unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().with_writer(file).with_ansi(false))
        .with(fmt::layer())
        .init();

    let db = &cfg.database;
    tracing::info!(
        "db-monitor v{} | {:?} {}{}{} | cada {}s | estado: {}",
        env!("CARGO_PKG_VERSION"),
        db.kind,
        db.host,
        if db.host.is_empty() { "" } else { "/" },
        db.dbname,
        cfg.monitor.interval_secs,
        cfg.monitor.state_file.display(),
    );

    let stop = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    match monitor::run(cfg, stop).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}
