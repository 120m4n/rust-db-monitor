# Historias de usuario HU-01..HU-09 — Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** cerrar las brechas entre `docs/historias-usuario.md` y el código: endurecer HU-01..HU-05 (ya construidas), implementar HU-06 (sprint, prioridad alta) y dejar planificadas HU-07..HU-09 (backlog).

**Architecture:** cambios acotados en los módulos existentes; un único módulo nuevo `src/credenciales.rs` (ofuscación + reescritura del config). La reescritura del archivo ocurre en `main.rs` **después** de tomar el lock y **antes** de iniciar el log; `config::load` solo desofusca en memoria.

**Tech Stack:** Rust 2021, `toml` 0.8 + `toml_edit` 0.20 (ya en `Cargo.lock`), `base64` 0.22 (ya en `Cargo.lock`), `tokio-postgres` 0.7.18, `oracle` 0.6, `tracing-appender` 0.2.5, `windows-sys` 0.61.

**Spec:** `docs/historias-usuario.md`

## Global Constraints

- Comentarios, mensajes de log y textos en español (CLAUDE.md).
- Todo texto visible para el operador es una plantilla en `Textos` (`src/textos.rs`); nada de literales en otros módulos.
- Sin crates nuevas en el árbol: `toml_edit = "0.20"` y `base64 = "0.22"` como dependencias directas solo porque ya están en `Cargo.lock`. Verificar con `cargo tree -d` que no aparece ninguna versión nueva.
- Prefijo de ofuscación exacto: `GSR:`. Mensaje exacto: `Credenciales visibles detectadas, iniciando el proceso de cifrado`.
- Las credenciales en claro nunca van a logs, `Debug`, ni mensajes de error.
- `cargo test` no toca UI ni bases externas; lo de Windows se valida a mano (no hay pwsh ni Windows en el Mac).
- Atajos deliberados marcados con `ponytail:`.
- Mac: `export PATH=$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH` antes de `cargo`.

## Review Focus

1. **`config.toml` guardado con el Bloc de notas (BOM UTF-8 + CRLF)** → `load` lo lee y la reescritura de HU-06 conserva BOM, CRLF y comentarios. Test en Task 2.
2. **Contraseña con `"`, `\`, `ñ` o `'`** → ida y vuelta exacta por `GSR:` y TOML reescrito válido; token con `"` válido en la config de curl. Tests en Task 2 y Task 4.
3. **Dos arranques simultáneos en el primer arranque (tarea + doble clic)** → solo el que tiene el lock reescribe. Orden fijado en Task 3; prueba manual.
4. **`config.toml` de solo lectura o carpeta sin escritura** → ventana de error, código 1, nunca sigue en claro. Test en Task 3.
5. **Configs desplegadas con `query_timeout_secs >= interval_secs` o una clave mal escrita** → ahora no arrancan; el error nombra la clave. Test en Task 1; nota en el README.

---

### Task 0: Preparar `main`

Hoy hay trabajo sin commit (repetición del toast rojo en `src/alerts.rs`, `src/main.rs`, README y ejemplos) y la rama `docs/historias-usuario` sin integrar.

- [ ] **Step 1:** el usuario revisa y hace commit de su trabajo pendiente (no forma parte de este plan).
- [ ] **Step 2:** integrar `docs/historias-usuario` en `main` (merge o PR).
- [ ] **Step 3:** `cargo test` en `main` → PASS.
- [ ] **Step 4:** crear la rama `feat/historias-usuario` desde `main`. Cada task es un commit; al final, PR a `main`.

---

### Task 1: Validación estricta de config (HU-02)

**Files:**
- Modify: `src/config.rs` (structs `Config`, `Database`, `Freshness`, `Logging`, `Ntfy`; `Config::validate`)
- Modify: `src/main.rs:61` (el `unwrap_or_else` del `EnvFilter` pasa a `expect`, ya validado)
- Modify: `README.md` (sección de actualización)

**Interfaces:**
- Produces: `Config::validate` además rechaza claves desconocidas, `connect_timeout_secs == 0`, `query_timeout_secs >= interval_secs` y `level` inválido.

- [ ] **Step 1: tests que fallan** en `config.rs`:

```rust
#[test] fn rejects_unknown_key_in_any_section() {
    // "pasword" en [database], "intervalo" en [freshness], "nivel" en [logging], "topik" en [ntfy]
    // cada uno: load(..) es Err y format!("{e:#}") contiene el nombre de la clave
}
#[test] fn rejects_query_timeout_not_below_interval() {
    // interval_secs = 30, query_timeout_secs = 30 → Err; query_timeout_secs = 29 → Ok
}
#[test] fn rejects_zero_connect_timeout() { /* connect_timeout_secs = 0 → Err */ }
#[test] fn rejects_unknown_log_level() { /* level = "verbose" → Err; level = "debug" → Ok */ }
```

- [ ] **Step 2:** `cargo test config::` → los 4 FAIL.
- [ ] **Step 3:** implementar:
  - `#[serde(deny_unknown_fields)]` en los 5 structs.
  - En `validate`: los tres rangos; nivel con `tracing_subscriber::EnvFilter::try_new(&self.logging.level).is_ok()`.
  - Mensajes en el estilo actual (`"freshness.query_timeout_secs must be < interval_secs"`).
- [ ] **Step 4:** `cargo test` → PASS, incluido `example_configs_are_valid`; corregir los ejemplos si fallan.
- [ ] **Step 5:** README: párrafo "Al actualizar" indicando que una clave mal escrita o `query_timeout_secs >= interval_secs` ahora impide arrancar.
- [ ] **Step 6:** commit `feat(config): rechaza claves desconocidas y rangos inválidos (HU-02)`.

---

### Task 2: Módulo `credenciales` — ofuscación y reescritura en memoria (HU-06)

**Files:**
- Create: `src/credenciales.rs`
- Modify: `src/lib.rs` (`pub mod credenciales;`)
- Modify: `Cargo.toml` (`base64 = "0.22"`, `toml_edit = "0.20"`)

**Interfaces:**
- Produces:
  - `pub const PREFIJO: &str = "GSR:";`
  - `pub const CAMPOS: [(&str, &str); 3] = [("database","user"), ("database","password"), ("ntfy","token")];`
  - `pub fn ofuscar(claro: &str) -> String` → `"GSR:" + base64(xor(bytes, CLAVE))`.
  - `pub fn desofuscar(valor: &str) -> anyhow::Result<String>` → error si falta el prefijo, si el Base64 es inválido o si el resultado no es UTF-8. El error **no** incluye el valor.
  - `pub fn ofuscar_texto(texto: &str) -> anyhow::Result<Option<String>>` → `None` si no hay nada en claro; `Some(nuevo)` con solo esos valores cambiados.

- [ ] **Step 1: tests que fallan** en `credenciales.rs`:

```rust
#[test] fn ida_y_vuelta() {
    for s in ["monitor", "p@ss\"w\\o'rd ñ", "x"] {
        let o = ofuscar(s);
        assert!(o.starts_with("GSR:") && !o.contains(s));
        assert_eq!(desofuscar(&o).unwrap(), s);
    }
}
#[test] fn desofuscar_rechaza_basura_sin_filtrar_el_valor() {
    assert!(desofuscar("sinprefijo").is_err());
    let e = desofuscar("GSR:%%%secreto").unwrap_err();
    assert!(!format!("{e:#}").contains("secreto"));
}
#[test] fn ofuscar_texto_solo_toca_campos_en_claro() {
    let t = "# cabecera\n[database]\nkind = \"oracle\"\nuser = \"u\"  # usuario\npassword = \"GSR:...\"\n[logging]\nlevel = \"info\"\n";
    // usa ofuscar("pw") real en lugar de "GSR:..."
    let n = ofuscar_texto(t).unwrap().unwrap();
    // user ahora empieza por GSR:, password intacto, "# cabecera", "# usuario" y [logging] intactos
}
#[test] fn ofuscar_texto_none_si_ya_esta_todo_ofuscado_o_vacio() {
    // user y password con GSR: → None; password = "" y sin [ntfy] → None
}
#[test] fn ofuscar_texto_conserva_bom_y_crlf() {
    let t = "\u{feff}[database]\r\nuser = \"u\"\r\n";
    let n = ofuscar_texto(t).unwrap().unwrap();
    assert!(n.starts_with('\u{feff}') && n.contains("\r\n") && !n.replace("\r\n", "").contains('\n'));
}
```

- [ ] **Step 2:** `cargo test credenciales::` → FAIL (no compila).
- [ ] **Step 3: implementar.**
  - `CLAVE`: `const CLAVE: &[u8; 32]` con bytes fijos. Marcarla con `// ponytail: clave fija en el binario = ofuscación, no cifrado; HU-07 (DPAPI) si TI pide cifrado real`.
  - `ofuscar_texto`:
    1. Quitar y recordar el BOM.
    2. Parsear con `toml_edit::Document` (en 0.20 el tipo se llama `Document`, no `DocumentMut`).
    3. Para cada uno de los `CAMPOS` que sea string no vacío y no empiece por `PREFIJO`, sustituir el valor copiando el `decor()` del original para no perder el comentario de fin de línea.
    4. Si el original tenía `\r\n` y la salida no, normalizar a `\r\n`.
    5. Volver a poner el BOM.
- [ ] **Step 4:** `cargo test credenciales::` → PASS; `cargo tree -d` sin versiones nuevas.
- [ ] **Step 5:** commit `feat(credenciales): ofuscación GSR: y reescritura que conserva formato (HU-06)`.

---

### Task 3: Integrar HU-06 en el arranque

**Files:**
- Modify: `src/config.rs` (`load`, `impl Debug for Database`)
- Modify: `src/credenciales.rs` (escritura del archivo)
- Modify: `src/main.rs` (orden de arranque)
- Modify: `src/textos.rs` (claves `credenciales.titulo` / `credenciales.cuerpo`)
- Modify: `Cargo.toml` (`windows-sys`: feature `Win32_Storage_FileSystem`)
- Modify: `README.md`, `config.example.toml`, `config.iec104.example.toml`

**Interfaces:**
- Consumes: `credenciales::{desofuscar, ofuscar_texto, CAMPOS, PREFIJO}` (Task 2).
- Produces:
  - `pub fn proteger_archivo(path: &Path) -> anyhow::Result<bool>` en `credenciales.rs`:
    1. Relee el archivo y llama a `ofuscar_texto`.
    2. Si hay cambios, escribe `config.toml.tmp` con `sync_all`.
    3. Reemplaza el original: en Windows con `ReplaceFileW`, que conserva ACL y atributos; en el resto con `fs::rename` copiando los `permissions` del original.
    4. Devuelve `true` si reescribió.
  - `Textos.credenciales_titulo` (por defecto `"{app}: credenciales protegidas"`) y `Textos.credenciales_cuerpo` (por defecto `"Credenciales visibles detectadas, iniciando el proceso de cifrado"`).

- [ ] **Step 1: tests que fallan:**

```rust
// config.rs
#[test] fn load_desofusca_gsr_antes_de_db_password() {
    // password = ofuscar("filepw") → load().database.password == "filepw"
    // con DB_PASSWORD=env → "env"
}
#[test] fn load_error_gsr_corrupto_nombra_campo() {
    // password = "GSR:%%" → Err que contiene "database.password"
}
#[test] fn load_acepta_bom() { /* archivo con \u{feff} al inicio → Ok */ }
#[test] fn error_de_sintaxis_no_muestra_el_secreto() {
    // 'password = "s3cr3t' (sin cerrar) → Err; el mensaje no contiene "s3cr3t" y sí "línea"
}
#[test] fn debug_redacta_user() { /* format!("{:?}", db) no contiene el user */ }
// credenciales.rs
#[test] fn proteger_archivo_reescribe_una_vez() {
    // 1.ª llamada → true y el archivo tiene GSR:; 2.ª → false y el archivo queda igual byte a byte
}
#[cfg(unix)] #[test] fn proteger_archivo_solo_lectura_es_error() {
    // permisos 0o444 → Err y el contenido no cambia
}
#[test] fn proteger_archivo_nunca_escribe_db_password() {
    // DB_PASSWORD=envpw, archivo con password = "filepw" → el archivo no contiene "envpw"
}
```

- [ ] **Step 2:** `cargo test` → FAIL.
- [ ] **Step 3: implementar.**
  - `load`:
    1. Quitar el BOM.
    2. Ante un error de parseo, reportar solo `línea N, columna M`, calculadas desde `toml::de::Error::span()`, sin el fragmento de la línea.
    3. Sobre el `toml::Table` crudo, **antes** de `DB_PASSWORD`/`NTFY_TOKEN`, desofuscar cada campo de `CAMPOS` que empiece por `PREFIJO`. El error lleva el nombre `sección.clave`.
  - `Debug` de `Database`: `user` pasa a `"***"`.
  - `main.rs`, orden de arranque:
    1. `load`.
    2. Lock (sin cambios).
    3. `let protegido = credenciales::proteger_archivo(&path).unwrap_or_else(|e| fatal(&tx.app, &format!("{e:#}")))`.
    4. Iniciar el log.
    5. Si `protegido`: `tracing::info!("credenciales ofuscadas en {}", path.display())` y, tras `setup_toasts`, `desktop::toast(&tx.credenciales_titulo, &tx.credenciales_cuerpo)`. Toast, no ventana modal.
  - `textos.rs`: agregar las 2 claves a `campos()` (el array pasa de 19 a 21) y a la tabla del README.
- [ ] **Step 4:** `cargo test` → PASS.
- [ ] **Step 5:** README: sección "Credenciales ofuscadas (`GSR:`)" con el criterio 13 de HU-06 y la limitación (no es cifrado). Ejemplos de config: comentario de que `user`/`password` se ofuscan solos al primer arranque.
- [ ] **Step 6: manual en Windows.**
  1. Copiar un `config.toml` en claro, guardado con el Bloc de notas.
  2. Arranque 1: toast y el archivo pasa a `GSR:`, conservando comentarios y ACL (comprobar con `icacls`).
  3. Arranque 2: sin toast, conecta.
  4. Abrir doble clic + tarea a la vez: solo una instancia reescribe.
- [ ] **Step 7:** commit `feat: credenciales ofuscadas con GSR: al primer arranque (HU-06)`.

---

### Task 4: Token ntfy por stdin (HU-05)

**Files:**
- Modify: `src/ntfy.rs`

**Interfaces:**
- Produces:
  - `pub fn curl_args(n: &Ntfy, title: &str, body: &str, priority: u8) -> Vec<String>`, que **nunca** contiene el token. Si hay token, incluye `--config -`.
  - `pub fn curl_config(n: &Ntfy) -> Option<String>` → `header = "Authorization: Bearer <token>"` con `\` y `"` escapados.
  - `pub fn send(n: &Ntfy, title: &str, body: &str, priority: u8)`.
- `urgent: bool` pasa a `priority: u8` (4 = rojo/amarillo, 3 = normal; HU-08 usará 2). El tag: `warning` si `priority >= 4`; si no, `white_check_mark`. Los llamadores en `main.rs` pasan `if urgent {4} else {3}`.

- [ ] **Step 1: tests:** actualizar los 2 existentes a `priority` y agregar:

```rust
#[test] fn token_nunca_en_args() {
    let a = curl_args(&n("tk_\"x"), "t", "b", 4);
    assert!(!a.iter().any(|x| x.contains("tk_")));
    assert!(a.windows(2).any(|w| w == ["--config", "-"]));
    assert_eq!(curl_config(&n("tk_\"x")).unwrap(), "header = \"Authorization: Bearer tk_\\\"x\"\n");
    assert!(curl_config(&n("")).is_none());
}
```

- [ ] **Step 2:** FAIL → **Step 3:** implementar. `send` usa `Stdio::piped()` y escribe `curl_config` en stdin antes de `wait_with_output`.
- [ ] **Step 4:** `cargo test ntfy::` → PASS.
- [ ] **Step 5:** README `[ntfy]`: el token no aparece en la línea de comandos; los avisos fallidos no se reintentan.
- [ ] **Step 6: manual en Windows:** Administrador de tareas → columna "Línea de comandos" de `curl.exe` sin el token; el aviso llega al móvil.
- [ ] **Step 7:** commit `fix(ntfy): el token viaja por stdin, no en la línea de comandos (HU-05)`.

---

### Task 5: Conexión robusta (HU-01)

**Files:**
- Modify: `src/db.rs` (`connect`, `Client::interrupt`)
- Modify: `tests/db_targets.rs`
- Modify: `README.md`

**Interfaces:**
- Produces: `Client::interrupt()` también cancela en Postgres. Como mucho **un** intento de conexión Oracle bloqueado a la vez.

- [ ] **Step 1: tests:**
  - `db.rs`:
    ```rust
    #[test] fn un_solo_connect_oracle_en_vuelo() {
        let g = OracleEnVuelo::tomar().unwrap();
        assert!(OracleEnVuelo::tomar().is_none());
        drop(g);
        assert!(OracleEnVuelo::tomar().is_some());
    }
    ```
  - `tests/db_targets.rs` (`#[ignore]`, Postgres real): `pg_interrupt_cancela_en_el_servidor`. Lanzar `SELECT pg_sleep(30)` con un timeout de 1 s, llamar a `interrupt()` y comprobar en `pg_stat_activity` que no queda la consulta.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: implementar.**
  - **Postgres:**
    - En `connect`: `.keepalives(true).keepalives_idle(Duration::from_secs(60))`.
    - En `interrupt`: `let t = c.cancel_token(); tokio::spawn(async move { let _ = t.cancel_query(NoTls).await; });`
  - **Oracle:** `struct OracleEnVuelo` sobre un `static AtomicBool`.
    - `tomar() -> Option<OracleEnVuelo>`; `Drop` lo libera.
    - El guard se **mueve dentro** del `spawn_blocking`, así se libera cuando el hilo termina, aunque el timeout ya haya expirado.
    - Si `tomar()` da `None`: `bail!("oracle connect anterior aún en curso")`, que cuenta como intento fallido.
    - Comentario: `// ponytail: global; un solo Client por proceso`.
- [ ] **Step 4:** `cargo test` y `cargo test --test db_targets -- --ignored --test-threads=1` → PASS.
- [ ] **Step 5:** README: sección "Usuario de solo lectura", con los `GRANT` para Oracle (`CREATE USER monitor …; GRANT CREATE SESSION TO monitor; GRANT SELECT ON esquema.tabla TO monitor;`) y para PostgreSQL (`GRANT CONNECT …; GRANT USAGE ON SCHEMA …; GRANT SELECT ON tabla TO monitor;`). Quitar del TODO el keepalive de PG.
- [ ] **Step 6:** commit `fix(db): cancela consultas en PG, keepalive y un solo connect Oracle colgado (HU-01)`.

---

### Task 6: Frescura correcta (HU-03)

**Files:**
- Modify: `src/config.rs` (`Freshness.clock`)
- Modify: `src/freshness.rs` (`recent_sql`)
- Modify: `src/monitor.rs` (error de tabla en marcha)
- Modify: `src/main.rs` (ntfy en el error fatal)
- Modify: `src/textos.rs` (`detenido.titulo`)
- Modify: `tests/db_targets.rs`, `README.md`, ejemplos

**Interfaces:**
- Produces:
  - `pub enum Clock { Utc, Local }` con serde lowercase. `Freshness.clock: Option<Clock>`; si falta, `Utc` para sqlite y `Local` para pg/oracle (el comportamiento actual).
  - `fn recent_sql(kind: Kind, clock: Clock, f: &Freshness) -> String`.
  - `Textos.detenido_titulo`, por defecto `"{app}: detenido"`; el cuerpo es el mensaje del error.

- [ ] **Step 1: tests** en `freshness.rs` (reemplaza `recent_sql_uses_each_engines_clock`):

```rust
#[test] fn recent_sql_por_motor_y_reloj() {
    // (Postgres, Local): "now() - INTERVAL '90 seconds'" y "<= now() + INTERVAL '300 seconds'"
    // (Postgres, Utc):   "(now() AT TIME ZONE 'UTC')"
    // (Oracle, Local):   "SYSDATE - 90/86400" y "<= SYSDATE + 300/86400"
    // (Oracle, Utc):     "SYS_EXTRACT_UTC(SYSTIMESTAMP)"
    // (Sqlite, Utc):     "datetime('now', '-90 seconds')"
    // (Sqlite, Local):   "datetime('now', 'localtime', '-90 seconds')"
}
```

  Integración SQLite (`tests/`): `sqlite_fila_futura_no_cuenta` (fila a +1 h → `recent_count == 0`) y `sqlite_tabla_borrada_en_marcha_es_fatal` (`DROP TABLE` tras el primer `Fresh` → `run` devuelve `Err`, no un bucle de `Failed`).

- [ ] **Step 2:** FAIL.
- [ ] **Step 3: implementar.**
  - **SQL:** `WHERE time >= since AND time <= ahora + 300 s`. Comentario `// ponytail: tolerancia fija de 5 min al reloj del adquisidor`.
  - **monitor:** en `Ok(Err(e))` del tick, igual que en la validación inicial: si `!client.is_closed()` y `probe` responde → `return Err(e.context("la tabla dejó de estar disponible"))`. Extraer el helper `async fn sigue_viva(client: &Client, t: Duration) -> bool` y usarlo en ambos sitios.
  - **main, en `Stopped(Err)`:** además de `error_box`, `ntfy::send(n, &tx.detenido_titulo, &msg, 4)`, **antes** de la ventana, porque bloquea.
- [ ] **Step 4:** `cargo test` y las pruebas ignoradas → PASS.
- [ ] **Step 5:** README `[freshness]`: clave `clock`, requisito de índice (`CREATE INDEX … ON tabla(time_column)`), y que una tabla borrada en marcha detiene la app con error.
- [ ] **Step 6:** commit `feat(freshness): reloj UTC/local, ignora fechas futuras y detecta tabla borrada (HU-03)`.

---

### Task 7: Bandeja y despliegue (HU-04)

**Files:**
- Modify: `src/desktop.rs` (`Warning::show`, Windows)
- Modify: `scripts/install-task.ps1` (comprobar escritura)
- Modify: `Dockerfile.windows`, `docker-compose.yml` (firma)
- Modify: `README.md`

**Interfaces:**
- Produces: `Warning` recuerda si ya se mostró; las reapariciones no usan `MB_SETFOREGROUND`.

- [ ] **Step 1: implementar.**
  - **`Warning`:**
    - Agregar `mostrada: bool`.
    - 1.ª vez: estilo actual.
    - Siguientes: `MB_OK | MB_ICONWARNING | MB_TOPMOST`, visible pero sin pedir el foco.
    - Sin test automático: es UI.
  - **`install-task.ps1`:** antes de registrar la tarea, crear y borrar `$Dir\.escritura-test`; si falla, `throw` con "la carpeta debe ser escribible por el usuario (no uses Program Files)".
  - **Firma:**
    1. `Dockerfile.windows` instala `osslsigncode`.
    2. El `command` del compose firma `dist/windows/db-monitor.exe` solo si existe `$SIGN_PFX`, usando `osslsigncode sign -pkcs12 $SIGN_PFX -pass $SIGN_PASS -t http://timestamp.digicert.com`.
    3. Si no hay certificado, imprime `AVISO: exe sin firmar`.
- [ ] **Step 2:** `cargo test` → PASS. `docker compose run --rm build-windows` → sin `SIGN_PFX` imprime el aviso; con el PFX de prueba, `osslsigncode verify` da OK.
- [ ] **Step 3:** README:
  - "Salir" no detiene la vigilancia de forma permanente; para eso hay que deshabilitar la tarea.
  - Cómo firmar (`SIGN_PFX`, `SIGN_PASS`); el certificado lo provee TI.
- [ ] **Step 4: manual en Windows:**
  - En rojo, cerrar la ventana y escribir en el Bloc de notas: al reaparecer, no roba el foco.
  - El `.exe` firmado no muestra SmartScreen.
- [ ] **Step 5:** commit `feat: advertencia sin robar foco, chequeo de carpeta y firma del exe (HU-04)`.

---

## Backlog (no ejecutar sin aprobación)

### Task 8: Retención de logs (HU-09)

**Files:**
- Modify: `src/config.rs` (`Logging.max_files: usize`, por defecto 30)
- Modify: `src/main.rs:58-59`
- Modify: `README.md`, ejemplos

- [ ] **Step 1: test** de integración en `tests/`, `retencion_borra_solo_logs_propios`:
  1. Carpeta temporal con 35 archivos `db-monitor.log.2026-01-01`.. y un `otro.txt`.
  2. Llamar a `logs::podar(dir, 30)`.
  3. Quedan los 30 más nuevos y `otro.txt`.
- [ ] **Step 2:** FAIL → **Step 3:** implementar.
  - `pub fn podar(dir: &Path, max: usize) -> std::io::Result<()>`, al final de `config.rs`: ordena por nombre y borra con `WARN` ante un fallo.
  - En `main`: llamarla al arrancar; rotación con `tracing_appender::rolling::Builder::new().rotation(Rotation::DAILY).filename_prefix("db-monitor.log").max_log_files(n).build(dir)`, que poda en cada cambio de día. `max_files = 0` → no se llama a ninguna de las dos.
- [ ] **Step 4:** PASS → **Step 5:** commit `feat(logging): retención de logs con max_files (HU-09)`.

### Task 9: Aviso diario "sigo vivo" (HU-08)

**Files:**
- Modify: `src/config.rs` (`Ntfy.heartbeat: Option<String>`, validado como `HH:MM`)
- Create: `src/heartbeat.rs`
- Modify: `src/alerts.rs` (`pub fn ultimo(&self) -> (Option<u64>, &str)` con el último `n` y el último `HH:MM`)
- Modify: `src/main.rs` (`ControlFlow::WaitUntil`)
- Modify: `src/textos.rs` (`heartbeat.titulo` / `heartbeat.cuerpo`, variables `{estado}`, `{hora}`, `{n}`)

**Interfaces:**
- Produces:
  - `pub fn parse_hora(s: &str) -> anyhow::Result<(u32, u32)>`.
  - `pub fn toca(ahora: chrono::NaiveDateTime, hora: (u32, u32), ultimo: Option<chrono::NaiveDate>) -> bool` → `true` si ya pasó la hora de hoy y no se envió hoy.
  - `pub fn proximo(ahora: NaiveDateTime, hora: (u32, u32)) -> NaiveDateTime`.
  - La fecha del último envío se guarda en `db-monitor.heartbeat`, junto al lock, para no repetir el aviso cuando la tarea relanza la app.

- [ ] **Step 1: tests** en `heartbeat.rs`:
  - `toca_despues_de_la_hora_solo_una_vez_por_dia`: 07:59 → false; 08:00 sin envío previo → true; 08:00 con `ultimo` = hoy → false; al día siguiente, 08:00 → true.
  - `proximo_salta_al_dia_siguiente`.
  - `parse_hora_rechaza_25_00_y_texto`.
- [ ] **Step 2:** FAIL → **Step 3: implementar.**
  - En `main`: al terminar cada evento, `ControlFlow::WaitUntil(instante de proximo(..))`.
  - En `StartCause::ResumeTimeReached` y en `Init`: si `toca`, `ntfy::send(.., prioridad)` con prioridad 2 en verde y 4 en rojo/amarillo, y escribir la fecha en el archivo.
  - Sin toast.
- [ ] **Step 4:** PASS. Manual: con `heartbeat` a +2 min llega 1 aviso; reiniciar la app → no se repite.
- [ ] **Step 5:** commit `feat(ntfy): aviso diario de funcionamiento (HU-08)`.

### Task 10: DPAPI (HU-07, requiere aprobación de TI)

**Files:**
- Modify: `src/credenciales.rs`
- Modify: `Cargo.toml` (`windows`: feature `Win32_Security_Cryptography`)
- Modify: `src/main.rs`, `README.md`

**Interfaces:**
- Consumes: `ofuscar_texto` / `proteger_archivo` (Tasks 2-3), parametrizados por `fn(&str) -> String`.
- Produces:
  - `pub const PREFIJO_DPAPI: &str = "DPAPI:";`
  - `#[cfg(windows)] fn dpapi_cifrar(&str) -> Result<String>` y `fn dpapi_descifrar(&str) -> Result<String>`, con `CryptProtectData`/`CryptUnprotectData`, `CRYPTPROTECT_UI_FORBIDDEN`, `LocalFree` y Base64.
  - Fuera de Windows: stub que devuelve `"solo disponible en Windows"`.

- [ ] **Step 1: tests:**
  - `dpapi_fuera_de_windows_es_error` (`#[cfg(not(windows))]`).
  - `migra_gsr_a_dpapi_en_el_texto`, usando un cifrador de prueba inyectado.
- [ ] **Step 2:** FAIL → **Step 3: implementar.**
  - `load` acepta `GSR:` y `DPAPI:`.
  - `proteger_archivo` cifra a `DPAPI:` **solo si `tarea == true`**. En un arranque manual con valores en claro o `GSR:` no reescribe y registra un `WARN`.
  - Error de descifrado: *"La credencial de <campo> fue cifrada en otro equipo o usuario; escríbala de nuevo en claro en config.toml"*.
- [ ] **Step 4:** PASS + los 5 escenarios manuales de HU-07.
- [ ] **Step 5:** commit `feat(credenciales): cifrado DPAPI desde la tarea programada (HU-07)`.

---

## Cobertura HU → Task

| HU | Criterios | Task |
|---|---|---|
| HU-01 | 3 (Oracle colgado), 5 (cancelación), 6 (keepalive), 8 (solo lectura) | 5 |
| HU-02 | 4 (claves, rangos, nivel) | 1 |
| HU-03 | 4 (reloj), 6 (tabla borrada), 7 (futuras), 9 (índice) | 6 |
| HU-04 | 5 (foco), 8 (Salir), 9 (carpeta), 10 (firma) | 7 |
| HU-05 | 2 (arranque: ya cumplido, `main.rs:147`), 4 (stdin), 5 (sin reintento) | 4 |
| HU-06 | 1-13 | 2, 3 |
| HU-07 | 1-10 | 10 |
| HU-08 | 1-7 | 9 |
| HU-09 | 1-5 | 8 |

Los criterios que no aparecen aquí ya están cumplidos por el código actual, contando con el trabajo sin commit de la Task 0.
