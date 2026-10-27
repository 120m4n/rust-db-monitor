// Incrusta el icono en el .exe. build.rs corre en el host: se mira el target, no cfg!(windows).
fn main() {
    println!("cargo:rerun-if-changed=assets/db-monitor.rc");
    println!("cargo:rerun-if-changed=assets/db-monitor.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/db-monitor.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("no se pudo compilar assets/db-monitor.rc");
    }
}
