//! Credenciales ofuscadas en `config.toml` (HU-06): `user`/`password` de `[database]` y `token` de
//! `[ntfy]` se guardan como `GSR:` + Base64(XOR). Es ofuscación, no cifrado: evita leerlas a simple vista.

use anyhow::{anyhow, bail};
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

#[cfg(test)]
mod tests {
    use super::*;

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
