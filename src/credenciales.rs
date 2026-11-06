//! Credenciales ofuscadas en `config.toml` (HU-06): `user`/`password` de `[database]` y `token` de
//! `[ntfy]` se guardan como `GSR:` + Base64(XOR). Es ofuscación, no cifrado: evita leerlas a simple vista.

use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, bail, Context};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;

pub const PREFIJO: &str = "GSR:";
/// Campos `(sección, clave)` que se ofuscan.
pub const CAMPOS: [(&str, &str); 3] = [("database", "user"), ("database", "password"), ("ntfy", "token")];

// ponytail: clave fija en el binario = ofuscación, no cifrado; HU-07 (DPAPI) si TI pide cifrado real
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

/// Ofusca los `CAMPOS` en claro (no vacíos y sin prefijo) conservando comentarios, orden, BOM y CRLF.
/// `None` si no había nada que ofuscar.
pub fn ofuscar_texto(texto: &str) -> anyhow::Result<Option<String>> {
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
        if s.is_empty() || s.starts_with(PREFIJO) {
            continue;
        }
        let decor = v.decor().clone();
        let mut nuevo = toml_edit::Value::from(ofuscar(s));
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

/// Relee `path` y, si tiene credenciales en claro, lo reescribe ofuscadas: escribe `<path>.tmp` y
/// reemplaza el original (en Windows con `ReplaceFileW`, que conserva ACL y atributos).
/// `true` si lo reescribió. Un archivo de solo lectura o sin permiso de escritura es error.
/// Llamar con el lock de instancia tomado.
pub fn proteger_archivo(path: &Path) -> anyhow::Result<bool> {
    let texto = std::fs::read_to_string(path).with_context(|| format!("no se pudo leer {}", path.display()))?;
    let Some(nuevo) = ofuscar_texto(&texto)? else { return Ok(false) };
    let ctx = || format!("no se pudo ofuscar las credenciales de {}: el archivo debe ser escribible", path.display());
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
    Ok(true)
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
        assert!(proteger_archivo(&p).unwrap());
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(!t.contains("\"pw\"") && t.contains("password = \"GSR:") && t.contains("# quien"), "{t}");
        assert!(!proteger_archivo(&p).unwrap());
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
        assert!(proteger_archivo(&p).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), t);
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
        let n = ofuscar_texto(&t).unwrap().unwrap();
        let doc: toml::Table = toml::from_str(&n).unwrap();
        let user = doc["database"]["user"].as_str().unwrap();
        assert_eq!(desofuscar(user).unwrap(), "u");
        assert_eq!(doc["database"]["password"].as_str().unwrap(), pw);
        assert!(n.starts_with("# cabecera\n") && n.contains("  # usuario\n") && n.contains("[logging]\nlevel = \"info\"\n"), "{n}");
    }

    #[test]
    fn ofuscar_texto_none_si_ya_esta_todo_ofuscado_o_vacio() {
        let t = format!("[database]\nuser = \"{}\"\npassword = \"{}\"\n", ofuscar("u"), ofuscar("p"));
        assert!(ofuscar_texto(&t).unwrap().is_none());
        assert!(ofuscar_texto("[database]\nkind = \"sqlite\"\npassword = \"\"\n").unwrap().is_none());
    }

    #[test]
    fn ofuscar_texto_conserva_bom_y_crlf() {
        let t = "\u{feff}[database]\r\nuser = \"u\"\r\n[ntfy]\r\ntoken = \"tk\"\r\n";
        let n = ofuscar_texto(t).unwrap().unwrap();
        assert!(n.starts_with('\u{feff}') && n.contains("\r\n"), "{n:?}");
        assert!(!n.replace("\r\n", "").contains('\n'), "{n:?}");
        assert!(!n.contains("\"tk\""), "{n:?}");
    }
}
