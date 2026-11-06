//! Credenciales protegidas en `config.toml`: `user`/`password` de `[database]` y `token` de `[ntfy]`.
//! - `GSR:` + Base64(XOR) (HU-06, por defecto): ofuscación, evita leerlas a simple vista.
//! - `DPAPI:` + Base64 (HU-07, `[credenciales] metodo = "dpapi"`): cifrado de Windows ligado al usuario
//!   y al equipo; solo se cifra cuando la lanza la tarea programada (`--tarea`).

use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, bail, Context};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Deserialize;

pub const PREFIJO: &str = "GSR:";
pub const PREFIJO_DPAPI: &str = "DPAPI:";
/// Campos `(sección, clave)` que se protegen.
pub const CAMPOS: [(&str, &str); 3] = [("database", "user"), ("database", "password"), ("ntfy", "token")];

#[derive(Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Metodo {
    #[default]
    Gsr,
    Dpapi,
}

#[derive(Debug, PartialEq)]
pub enum Resultado {
    SinCambios,
    Reescrito,
    /// `dpapi` con credenciales sin cifrar, pero no la lanzó la tarea: no se reescribe (quedaría
    /// cifrado para un usuario distinto del de la tarea).
    PendienteTarea,
}

// ponytail: clave fija en el binario = ofuscación, no cifrado; metodo = "dpapi" (HU-07) para cifrado real
const CLAVE: &[u8; 32] = b"\x5a\x13\xc7\x8e\x21\xf4\x6b\x90\x3d\xa2\x17\xe8\x4c\x05\xb9\x72\
\x88\x1f\xd3\x64\x0e\xaf\x57\xc1\x36\x9b\x7d\x02\xe5\x48\xbc\x29";

fn xor(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().zip(CLAVE.iter().cycle()).map(|(b, k)| b ^ k).collect()
}

pub fn ofuscar(claro: &str) -> String {
    format!("{PREFIJO}{}", STANDARD.encode(xor(claro.as_bytes())))
}

/// El error nunca incluye el valor.
pub fn desofuscar(valor: &str) -> anyhow::Result<String> {
    let Some(b64) = valor.strip_prefix(PREFIJO) else { bail!("falta el prefijo {PREFIJO}") };
    let bytes = STANDARD.decode(b64.trim()).map_err(|_| anyhow!("valor {PREFIJO} inválido (Base64)"))?;
    String::from_utf8(xor(&bytes)).map_err(|_| anyhow!("valor {PREFIJO} inválido (dato corrupto)"))
}

/// true si el valor ya va protegido (`GSR:` o `DPAPI:`).
pub fn protegido(valor: &str) -> bool {
    valor.starts_with(PREFIJO) || valor.starts_with(PREFIJO_DPAPI)
}

/// Valor en claro de un campo protegido; `campo` (`database.password`) solo para el mensaje de error.
pub fn revelar(valor: &str, campo: &str) -> anyhow::Result<String> {
    if let Some(b64) = valor.strip_prefix(PREFIJO_DPAPI) {
        let blob = STANDARD.decode(b64.trim()).map_err(|_| anyhow!("{campo}: valor {PREFIJO_DPAPI} inválido (Base64)"))?;
        return dpapi::descifrar(&blob).map_err(|e| {
            anyhow!("La credencial de {campo} fue cifrada en otro equipo o usuario; escríbala de nuevo en claro en config.toml ({e})")
        });
    }
    desofuscar(valor).with_context(|| campo.to_string())
}

/// Protege los `CAMPOS` con `metodo`, conservando comentarios, orden, BOM y CRLF. `None` si no había
/// nada que hacer. Con `Gsr` no toca valores `DPAPI:` (no degrada el cifrado).
pub fn proteger_texto(texto: &str, metodo: Metodo) -> anyhow::Result<Option<String>> {
    match metodo {
        Metodo::Gsr => reescribir(texto, PREFIJO, &|s| Ok(ofuscar(s))),
        Metodo::Dpapi => reescribir(texto, PREFIJO_DPAPI, &|s| {
            Ok(format!("{PREFIJO_DPAPI}{}", STANDARD.encode(dpapi::cifrar(s.as_bytes())?)))
        }),
    }
}

/// Pasa a `destino` cada campo no vacío que no lo tenga ya: los `GSR:` se desofuscan primero
/// (migración a DPAPI); los `DPAPI:` se dejan si el destino es `GSR:`.
fn reescribir(texto: &str, destino: &str, cifrar: &dyn Fn(&str) -> anyhow::Result<String>) -> anyhow::Result<Option<String>> {
    let (bom, cuerpo) = match texto.strip_prefix('\u{feff}') {
        Some(r) => ("\u{feff}", r),
        None => ("", texto),
    };
    let mut doc: toml_edit::Document = cuerpo.parse().map_err(|_| anyhow!("config.toml con sintaxis inválida"))?;
    let mut cambiado = false;
    for (sec, key) in CAMPOS {
        let Some(item) = doc.get_mut(sec).and_then(|t| t.as_table_like_mut()).and_then(|t| t.get_mut(key)) else {
            continue;
        };
        let Some(v) = item.as_value_mut() else { continue };
        let Some(s) = v.as_str() else { continue };
        if s.is_empty() || s.starts_with(destino) || (destino == PREFIJO && s.starts_with(PREFIJO_DPAPI)) {
            continue;
        }
        let claro = if s.starts_with(PREFIJO) { desofuscar(s).with_context(|| format!("{sec}.{key}"))? } else { s.to_string() };
        let decor = v.decor().clone();
        let mut nuevo = toml_edit::Value::from(cifrar(&claro).with_context(|| format!("{sec}.{key}"))?);
        *nuevo.decor_mut() = decor;
        *v = nuevo;
        cambiado = true;
    }
    if !cambiado {
        return Ok(None);
    }
    let mut out = doc.to_string();
    if cuerpo.contains("\r\n") {
        out = out.replace("\r\n", "\n").replace('\n', "\r\n");
    }
    Ok(Some(format!("{bom}{out}")))
}

/// Relee `path` y, si tiene credenciales sin proteger, lo reescribe: escribe `<path>.tmp` y reemplaza
/// el original (en Windows con `ReplaceFileW`, que conserva ACL y atributos). Un archivo de solo
/// lectura o sin permiso de escritura es error. Llamar con el lock de instancia tomado.
/// Con `Dpapi` solo cifra si `tarea` (lanzada por la tarea programada, con el usuario de la tarea).
pub fn proteger_archivo(path: &Path, metodo: Metodo, tarea: bool) -> anyhow::Result<Resultado> {
    let texto = std::fs::read_to_string(path).with_context(|| format!("no se pudo leer {}", path.display()))?;
    if metodo == Metodo::Dpapi && !tarea {
        // sin cifrar nada: ¿quedaría algo por proteger?
        let pendiente = reescribir(&texto, PREFIJO_DPAPI, &|s| Ok(format!("{PREFIJO_DPAPI}{s}")))?.is_some();
        return Ok(if pendiente { Resultado::PendienteTarea } else { Resultado::SinCambios });
    }
    let Some(nuevo) = proteger_texto(&texto, metodo)? else { return Ok(Resultado::SinCambios) };
    let ctx = || format!("no se pudo proteger las credenciales de {}: el archivo debe ser escribible", path.display());
    // un rename reemplazaría incluso un archivo de solo lectura: comprobar antes que se puede escribir
    std::fs::OpenOptions::new().write(true).open(path).with_context(ctx)?;
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp_name);
    let r = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(nuevo.as_bytes())?;
        f.sync_all()?;
        drop(f);
        reemplazar(path, &tmp)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r.with_context(ctx)?;
    Ok(Resultado::Reescrito)
}

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    /// Entropía fija de la app: otro programa del mismo usuario no descifra con una llamada en blanco.
    const ENTROPIA: &[u8] = b"db-monitor/credenciales/v1";

    fn blob(b: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: b.len() as u32, pbData: b.as_ptr() as *mut u8 }
    }

    /// `out` lo reserva Windows: se copia y se libera con `LocalFree`.
    unsafe fn tomar(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        v
    }

    pub fn cifrar(claro: &[u8]) -> anyhow::Result<Vec<u8>> {
        let (inp, ent) = (blob(claro), blob(ENTROPIA));
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe {
            CryptProtectData(&inp, std::ptr::null(), &ent, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        if ok == 0 {
            anyhow::bail!("CryptProtectData: {}", std::io::Error::last_os_error());
        }
        Ok(unsafe { tomar(out) })
    }

    pub fn descifrar(cifrado: &[u8]) -> anyhow::Result<String> {
        let (inp, ent) = (blob(cifrado), blob(ENTROPIA));
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe {
            CryptUnprotectData(&inp, std::ptr::null_mut(), &ent, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        if ok == 0 {
            anyhow::bail!("CryptUnprotectData: {}", std::io::Error::last_os_error());
        }
        String::from_utf8(unsafe { tomar(out) }).map_err(|_| anyhow::anyhow!("dato descifrado no es UTF-8"))
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn cifrar(_: &[u8]) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("DPAPI solo disponible en Windows")
    }
    pub fn descifrar(_: &[u8]) -> anyhow::Result<String> {
        anyhow::bail!("DPAPI solo disponible en Windows")
    }
}

#[cfg(windows)]
fn reemplazar(destino: &Path, origen: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
    let w = |p: &Path| p.as_os_str().encode_wide().chain([0]).collect::<Vec<u16>>();
    let (d, o) = (w(destino), w(origen));
    // conserva ACL, atributos y fecha de creación del original
    let ok = unsafe { ReplaceFileW(d.as_ptr(), o.as_ptr(), std::ptr::null(), 0, std::ptr::null(), std::ptr::null()) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn reemplazar(destino: &Path, origen: &Path) -> std::io::Result<()> {
    std::fs::set_permissions(origen, std::fs::metadata(destino)?.permissions())?;
    std::fs::rename(origen, destino)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proteger_archivo_reescribe_una_vez() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        std::fs::write(&p, "[database]\nuser = \"u\" # quien\npassword = \"pw\"\n").unwrap();
        assert_eq!(proteger_archivo(&p, Metodo::Gsr, false).unwrap(), Resultado::Reescrito);
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(!t.contains("\"pw\"") && t.contains("password = \"GSR:") && t.contains("# quien"), "{t}");
        assert_eq!(proteger_archivo(&p, Metodo::Gsr, false).unwrap(), Resultado::SinCambios);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), t);
        assert!(!d.path().join("config.toml.tmp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn proteger_archivo_solo_lectura_es_error() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        let t = "[database]\npassword = \"pw\"\n";
        std::fs::write(&p, t).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert!(proteger_archivo(&p, Metodo::Gsr, false).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), t);
    }

    #[test]
    fn migra_gsr_y_claro_al_destino() {
        let t = format!("[database]\nuser = \"u\"\npassword = \"{}\" # pw\n[ntfy]\ntoken = \"FAKE:ya\"\n", ofuscar("pw"));
        let n = reescribir(&t, "FAKE:", &|s| Ok(format!("FAKE:{s}"))).unwrap().unwrap();
        assert!(n.contains("user = \"FAKE:u\"") && n.contains("password = \"FAKE:pw\" # pw") && n.contains("token = \"FAKE:ya\""), "{n}");
    }

    #[test]
    fn gsr_no_degrada_dpapi() {
        let t = "[database]\npassword = \"DPAPI:AQAA\"\n";
        assert!(proteger_texto(t, Metodo::Gsr).unwrap().is_none());
    }

    #[test]
    fn dpapi_sin_tarea_no_reescribe() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        let t = "[database]\npassword = \"pw\"\n";
        std::fs::write(&p, t).unwrap();
        assert_eq!(proteger_archivo(&p, Metodo::Dpapi, false).unwrap(), Resultado::PendienteTarea);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), t);
    }

    #[cfg(not(windows))]
    #[test]
    fn dpapi_fuera_de_windows_es_error() {
        let e = revelar("DPAPI:AQAA", "database.password").unwrap_err();
        assert!(format!("{e:#}").contains("solo disponible en Windows"), "{e:#}");
        assert_eq!(revelar(&ofuscar("x"), "database.password").unwrap(), "x");
    }

    #[test]
    fn ida_y_vuelta() {
        for s in ["monitor", "p@ss\"w\\o'rd ñ", "x"] {
            let o = ofuscar(s);
            assert!(o.starts_with("GSR:") && !o.contains(s), "{o}");
            assert_eq!(desofuscar(&o).unwrap(), s);
        }
    }

    #[test]
    fn desofuscar_rechaza_basura_sin_filtrar_el_valor() {
        assert!(desofuscar("sinprefijo").is_err());
        let e = desofuscar("GSR:%%%secreto").unwrap_err();
        assert!(!format!("{e:#}").contains("secreto"));
        // Base64 válido que tras el XOR no es UTF-8
        assert!(desofuscar(&format!("GSR:{}", STANDARD.encode(xor(&[0xff, 0xfe])))).is_err());
    }

    #[test]
    fn ofuscar_texto_solo_toca_campos_en_claro() {
        let pw = ofuscar("pw");
        let t = format!(
            "# cabecera\n[database]\nkind = \"oracle\"\nuser = \"u\"  # usuario\npassword = \"{pw}\"\n[logging]\nlevel = \"info\"\n"
        );
        let n = proteger_texto(&t, Metodo::Gsr).unwrap().unwrap();
        let doc: toml::Table = toml::from_str(&n).unwrap();
        let user = doc["database"]["user"].as_str().unwrap();
        assert_eq!(desofuscar(user).unwrap(), "u");
        assert_eq!(doc["database"]["password"].as_str().unwrap(), pw);
        assert!(n.starts_with("# cabecera\n") && n.contains("  # usuario\n") && n.contains("[logging]\nlevel = \"info\"\n"), "{n}");
    }

    #[test]
    fn ofuscar_texto_none_si_ya_esta_todo_ofuscado_o_vacio() {
        let t = format!("[database]\nuser = \"{}\"\npassword = \"{}\"\n", ofuscar("u"), ofuscar("p"));
        assert!(proteger_texto(&t, Metodo::Gsr).unwrap().is_none());
        assert!(proteger_texto("[database]\nkind = \"sqlite\"\npassword = \"\"\n", Metodo::Gsr).unwrap().is_none());
    }

    #[test]
    fn ofuscar_texto_conserva_bom_y_crlf() {
        let t = "\u{feff}[database]\r\nuser = \"u\"\r\n[ntfy]\r\ntoken = \"tk\"\r\n";
        let n = proteger_texto(t, Metodo::Gsr).unwrap().unwrap();
        assert!(n.starts_with('\u{feff}') && n.contains("\r\n"), "{n:?}");
        assert!(!n.replace("\r\n", "").contains('\n'), "{n:?}");
        assert!(!n.contains("\"tk\""), "{n:?}");
    }
}
