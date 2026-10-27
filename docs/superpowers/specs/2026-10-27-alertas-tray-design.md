# Diseño: alertas visibles y icono de bandeja

Fecha: 2026-10-27 · Estado: borrador para revisión

## Intención

`db-monitor` es un perro guardián: vigila que una tabla siga recibiendo filas (caso real: `iec104_historialestadoelemento` en Oracle, ventana de 2 h, ver `config.iec104.example.toml`) y avisa cuando deja de recibirlas. Hoy el aviso es un `WARN` enterrado en un log que crece con miles de líneas de `nuevo registro`, y nadie lo lee. El aviso tiene que llegar solo a la pantalla del operador.

El programa se lanza a mano en un PC con Windows 11 de una sala de control. Mientras corre, lo único visible es un icono en la bandeja del sistema. macOS (Apple Silicon) se usa para desarrollar y probar.

## Objetivos

1. Eliminar la sección `[monitor]` (consulta con `{last_id}`, marca de agua, `state.toml`, log por fila). `[freshness]` pasa a ser la sección principal y obligatoria.
2. Icono de bandeja con tres colores según el estado; menú con estado, "Abrir logs" y "Salir".
3. Toasts nativos al cambiar de estado.
4. Ventana de advertencia insistente mientras la tabla esté sin datos.
5. Sin consola: errores de arranque visibles en una ventana; una sola instancia del programa.

## No objetivos

- Linux (ni bandeja ni notificaciones). El código puede no compilar allí con las nuevas dependencias; CI y Docker solo apuntan a Windows y el desarrollo a macOS.
- Vigilar más de una tabla.
- Correr como servicio de Windows o arrancar con el sistema.
- Configurar textos, colores o sonidos de los avisos.
- Sonido propio en los avisos (el toast usa el sonido por defecto del sistema).

## Anti-objetivos (sería un fracaso aunque funcione)

- Que un problema que dura horas genere un toast por ciclo (spam que el operador termina silenciando).
- Que se apilen varias ventanas de advertencia.
- Que un doble clic accidental deje dos instancias con avisos duplicados.
- Que un `config.toml` mal escrito haga que "no pase nada" al hacer doble clic.
- Que la ventana de advertencia bloquee el icono, el menú o los chequeos.

## Estados y avisos (decidido con el usuario)

| Estado | Condición | Icono |
|---|---|---|
| `Ok` | Conectado y `COUNT` de filas en la ventana > 0 | verde |
| `SinDatos` | Conectado y `COUNT` = 0 | rojo |
| `SinConexion` | Falla la conexión o una consulta (cualquier caso en que hoy se hace `break 'session true`) **y sigue fallando 60 s después del primer fallo** | amarillo |

Estado inicial, antes del primer resultado: amarillo, con texto "Conectando…" y **sin toast**.

**Gracia de 60 s para cortes (decidido con el usuario).** Un fallo no cambia el estado visible de inmediato: se anota el momento del primer fallo y, si a los 60 s no hubo un resultado bueno (`Ok` o `SinDatos`), recién entonces se pasa a `SinConexion` (icono amarillo + toast). Si se recupera antes, no hay toast ni cambio de icono: el corte queda solo en el log. Consecuencias:

- El cronómetro corre en el hilo de la interfaz (p. ej. `ControlFlow::WaitUntil`), no en el monitor: el monitor puede estar dormido en el backoff o colgado en un `connect` cuando se cumplen los 60 s.
- El toast "conexión restablecida" solo sale si antes salió el de "sin conexión".
- Al arrancar se aplica lo mismo: si a los 60 s todavía no hubo un primer resultado bueno, toast "sin conexión".
- El valor es fijo en el código (60 s), no se configura.
- La lógica de transiciones deja de depender solo de (estado anterior, nuevo): necesita el instante del primer fallo y la hora actual, que se le pasan como parámetro para poder testearla sin esperar.

Transiciones:

| De → a | Toast | Ventana de advertencia |
|---|---|---|
| (inicio) → `Ok` | no | — |
| (inicio) → `SinDatos` | sí, "sin datos" | se abre |
| (inicio) → `SinConexion` | sí, "sin conexión" | — |
| `Ok` → `SinDatos` | sí, "sin datos" | se abre |
| `SinDatos` → `Ok` | sí, "recuperado" | se cierra sola |
| cualquiera → `SinConexion` | sí, "sin conexión" | no se toca: si estaba abierta, sigue abierta |
| `SinConexion` → `Ok` / `SinDatos` | sí, "conexión restablecida" (+ "recuperado" o "sin datos" según el caso; ver nota) | se cierra si queda `Ok`; se abre si queda `SinDatos` |
| mismo estado, nuevo ciclo | no | si es `SinDatos` y la ventana está cerrada, se vuelve a abrir |

Nota de mi criterio: al reconectar sale **un solo** toast que combina ambas cosas ("Conexión restablecida · iec104_… sin registros en 2 h" o "Conexión restablecida · iec104_… OK"), para no lanzar dos toasts seguidos.

Un fallo de conexión en el arranque (Oracle caído) **no** es fatal: reintenta con backoff como hoy, en amarillo.

Textos (de mi criterio, en español):

- Sin datos: título `db-monitor: sin datos`, cuerpo `Sin registros nuevos en <tabla> en los últimos <ventana legible>`.
- Recuperado: `db-monitor: OK`, `<tabla> volvió a recibir registros`.
- Sin conexión: `db-monitor: sin conexión`, `No se puede conectar a la base de datos; reintentando`.
- Ventana de advertencia: título fijo `db-monitor: sin datos` (sirve para encontrarla y cerrarla), texto `Sin registros nuevos en <tabla> en los últimos <ventana>.\nÚltimo chequeo: HH:MM`, botón Aceptar. En Windows: siempre encima (`MB_TOPMOST | MB_SETFOREGROUND | MB_ICONWARNING`).
- `<ventana legible>`: `2 h`, `90 min`, `45 s`, según `window_secs`.

## Menú del icono (decidido con el usuario)

1. Línea de estado, deshabilitada (solo texto). También es el tooltip del icono:
   - verde: `OK · <n> registros en <ventana> · chequeo HH:MM`
   - rojo: `Sin registros en <ventana> · chequeo HH:MM`
   - amarillo: `Sin conexión · reintentando · desde HH:MM` (o `Conectando…` al inicio)
2. **Abrir logs**: abre la carpeta `logging.dir` en el Explorador (Windows: `explorer <dir>`) o en Finder (macOS: `open <dir>`). La carpeta y no el archivo, porque el archivo puede ser enorme.
3. **Salir**: apagado ordenado sin confirmación (interrumpe la consulta en curso, cierra la conexión, vacía el log, sale con código 0).

## Arranque sin consola

- Windows: `#![cfg_attr(windows, windows_subsystem = "windows")]` en `main.rs`. macOS: la app no aparece en el Dock (política de activación *accessory*).
- **Errores fatales** (config ilegible o inválida, tabla/columnas de `[freshness]` inexistentes): ventana de error nativa con el mensaje completo (`{e:#}`) y salida con código 1. Además se loguean si el log ya está inicializado.
- La validación de la tabla sigue como hoy (mi criterio): se hace en la primera conexión exitosa, aunque eso ocurra horas después del arranque si la base estaba caída, y cualquier error que no cierre la conexión (tabla o columna inexistente, falta de permisos) es fatal. Es un error de configuración que alguien tiene que corregir; la ventana de error muestra el motivo. El hilo del monitor devuelve ese error al hilo principal, que muestra la ventana y termina el bucle de eventos con código 1.
- **Una sola instancia**: archivo `db-monitor.lock` junto al `config.toml`, bloqueado con `std::fs::File::try_lock` (Rust ≥ 1.89) durante toda la vida del proceso. Si ya está bloqueado: ventana informativa "db-monitor ya está en ejecución" y salida con código 0. El bloqueo lo libera el sistema operativo si el proceso muere.
- Ctrl-C sigue funcionando como apagado cuando hay terminal (útil en macOS para desarrollar).

## Configuración nueva

```toml
[database]          # sin cambios

[freshness]         # obligatoria
table = "iec104_historialestadoelemento"
key_column = "codigohistorialestado"
time_column = "fecharegistro"
window_secs = 7200
interval_secs = 300        # antes en [monitor]; default 60, >= 1
query_timeout_secs = 60    # antes en [monitor]; default 30
backoff_max_secs = 300     # antes en [monitor]; default 300

[logging]           # sin cambios
```

- Si el archivo trae `[monitor]`, es error de configuración con un mensaje claro: `la sección [monitor] ya no existe: mueve interval_secs, query_timeout_secs y backoff_max_secs a [freshness] y borra query/state_file`. (Hoy serde ignora secciones desconocidas; esto se detecta explícitamente, sin activar `deny_unknown_fields` global.)
- `state.toml` deja de usarse; un archivo viejo simplemente se ignora.

## Log

Se mantiene el log diario en `logging.dir`, sin las líneas `nuevo registro` ni `ciclo: N filas`. Quedan: arranque, `conectado`, una línea por ciclo de frescura (`INFO frescura: N…` o `WARN sin registros…`), cambios de estado y errores. Con `interval_secs = 300` son unas 300 líneas por día.

## Arquitectura (mi criterio)

- **Hilo principal**: bucle de eventos de la interfaz (`tao`) con el icono (`tray-icon`, menú vía `muda`, que reexporta `tray-icon`). En macOS el icono y el menú solo pueden vivir en el hilo principal, por eso el monitor se mueve a otro hilo.
- **Hilo del monitor**: el runtime tokio `current_thread` actual con `monitor::run(cfg, shutdown, on_status)`. `on_status` recibe `Ok { n }`, `SinDatos` o `SinConexion` en cada resultado (cada ciclo y cada fallo) y los envía al hilo principal con el `EventLoopProxy` de `tao`.
- **Lógica de transiciones**: función pura (sin UI) que, dado el estado anterior, el nuevo resultado, el instante del primer fallo y la hora actual, devuelve qué hacer: toast sí/no y con qué texto, color del icono, abrir/cerrar/reabrir la ventana. Es lo que lleva tests unitarios; la UI solo ejecuta las acciones.
- **Toasts**: `notify-rust` (WinRT en Windows, Centro de notificaciones en macOS). En Windows, sin instalador no hay AppUserModelID propio, así que el remitente que se ve puede ser genérico (p. ej. PowerShell): aceptado.
- **Ventanas de advertencia, error e información**: cuadro de mensaje nativo. Windows: `MessageBoxW` en un hilo propio, para no bloquear el bucle de eventos; "una sola" = bandera atómica de "abierta"; "cerrar sola" = buscar la ventana por su título fijo y enviarle `WM_CLOSE`. macOS: cualquier diálogo nativo que no bloquee el bucle de eventos; ahí cerrarla sola al recuperarse es deseable pero no obligatorio.
- **Iconos**: tres círculos de color generados en código como RGBA (sin archivos de imagen).
- **Salir**: el menú resuelve el futuro `shutdown` que ya recibe `monitor::run`; el hilo principal espera al hilo del monitor (el `shutdown_timeout(2s)` actual sigue acotando la espera) y termina el bucle de eventos.

## Qué se elimina del código

- `monitor.rs`: `render_query`, `next_watermark`, `parse_id`, `load_state`, `save_state`, `State` y sus tests. Se quedan `backoff` y el bucle de reconexión.
- `config.rs`: `Monitor` y `state_file`.
- `freshness` deja de ser `Option` en `Config` y en `run`.
- Tests de integración que dependen de la consulta o de la marca de agua (`tests/db_targets.rs`): se borran o se pasan a frescura. `connect`, `fetch`, `interrupt` y los tests de frescura se quedan.

## Restricciones

- Debe seguir compilando con `docker compose run --rm build-windows` (`x86_64-pc-windows-gnu`, MinGW) y en el CI con MSVC (`windows-latest`).
- `cargo test` no debe lanzar toasts, ventanas ni iconos (la UI vive solo en `main.rs`/módulo de UI; `monitor::run` solo llama a `on_status`).
- Se mantiene: `config.toml` junto al exe, rutas relativas al config, `DB_PASSWORD`, `shutdown_timeout(2s)`, interrupción de consultas lentas.

## Documentación a actualizar

README, `docs/guia-build-y-prueba.md`, `config.example.toml`, `config.iec104.example.toml`, `CLAUDE.md`, `scripts/sqlite-demo.sh` (si hace falta).

## Lo que queda a criterio del implementador

Organización en módulos, nombres de tipos, versiones exactas de las librerías, tamaño y tono exacto de los colores del icono, y cómo resolver el diálogo no bloqueante en macOS.
