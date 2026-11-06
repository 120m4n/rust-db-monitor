# Historias de usuario — db-monitor

Definición funcional para el sprint (Azure DevOps). Cada historia cubre una parte de la aplicación con sus propias validaciones; todas convergen en un único entregable.

**Entregable único del sprint:** `db-monitor.exe` para Windows 10/11, firmado (ver HU-04), junto con `config.toml` de ejemplo, el script `install-task.ps1` + `db-monitor-task.xml` (arranque automático al iniciar sesión) y el README con la referencia de configuración.

> Notas: la configuración es `config.toml` (TOML), no `.xml`; el único `.xml` es `db-monitor-task.xml` (tarea programada). La revisión de datos nuevos usa una llave única (`key_column`) y una columna de fecha (`time_column`), no una FK.

## Resumen

| HU | Título | Prioridad | Puntos | Horas | Sprint |
|---|---|---|---|---|---|
| HU-01 | Conexión a base de datos Oracle | Normal | 3 | 21 h | Actual |
| HU-02 | Configuración por secciones desde archivo | Normal | 3 | 15 h | Actual |
| HU-03 | Detección de falta de datos nuevos en una tabla | Normal | 3 | 19 h | Actual |
| HU-04 | Alertas en la bandeja de Windows (toasts) | Normal | 3 | 26 h | Actual |
| HU-05 | Notificaciones externas a móviles con ntfy | Normal | 3 | 15 h | Actual |
| HU-06 | Ofuscación de credenciales en el archivo de configuración | **Alta** | 3 | 17 h | Actual |
| | **Total sprint** | | **18** | **113 h** | |
| HU-07 | Cifrado de credenciales con Windows DPAPI (mejora futura) | Baja | 3 | 9 h | Backlog |
| HU-08 | Aviso diario de funcionamiento ("sigo vivo") por ntfy | Media | 3 | 8 h | Backlog |
| HU-09 | Retención de logs | Media | 3 | 4 h | Backlog |

---

## HU-01 · Conexión a base de datos Oracle — 3 pts · 21 h

**Como** operador de la sala de control
**quiero** que la aplicación se conecte a la base de datos Oracle de producción
**para** poder vigilar sus tablas sin intervención manual.

**Criterios de aceptación**
1. Se conecta a Oracle con host, puerto (1521 por defecto), usuario, contraseña y *service name*.
2. La contraseña puede venir de la variable de entorno `DB_PASSWORD`, que tiene prioridad sobre el archivo.
3. Si la conexión tarda más de `connect_timeout_secs`, se da por fallida y se reintenta. Un intento colgado no debe acumular hilos ni conexiones en cada reintento.
4. Ante una caída de la base, reintenta sola con espera exponencial (1 s, 2 s, 4 s… hasta `backoff_max_secs`) sin cerrarse.
5. Si una consulta pasa de `query_timeout_secs`, se cancela **también en el servidor** (Oracle: interrupción de la llamada; PostgreSQL: cancelación de la consulta) y se registra en el log.
6. Una caída silenciosa de la red (sin cierre de conexión) se detecta en minutos, no en horas (keepalive TCP en PostgreSQL).
7. Funciona también con PostgreSQL y SQLite, que se usan para las pruebas.
8. **Requisito de instalación:** la aplicación usa un usuario de base de datos **de solo lectura**, con `SELECT` únicamente sobre la tabla vigilada. Lo crea el DBA/TI; el README lo documenta con el `GRANT` de ejemplo para Oracle y PostgreSQL.

**Validación:** pruebas de integración contra Oracle real (`TEST_ORA_*`), cortando la red o la base para comprobar los reintentos y la reconexión, y con una consulta lenta para comprobar que se cancela en el servidor.

| Tarea | Horas |
|---|---|
| Conexión Oracle (Instant Client, service name, timeout de conexión) | 5 |
| Soporte PostgreSQL y SQLite para pruebas | 3 |
| Reconexión con espera exponencial y timeout de consulta | 4 |
| Cancelación de consulta en el servidor y keepalive en PostgreSQL | 2 |
| Contraseña desde la variable `DB_PASSWORD` | 1 |
| Documentación del usuario de solo lectura (`GRANT`) | 1 |
| Pruebas de integración contra Oracle y PostgreSQL reales | 4 |
| Corrección de defectos | 1 |
| **Total** | **21 h** |

---

## HU-02 · Configuración por secciones desde archivo — 3 pts · 15 h

**Como** administrador
**quiero** configurar la aplicación con un archivo de texto dividido en secciones
**para** adaptarla a cada instalación sin recompilar.

**Criterios de aceptación**
1. Archivo `config.toml` junto al ejecutable, o una ruta pasada como argumento.
2. Secciones: `[database]`, `[freshness]` (obligatoria), `[logging]`, y las opcionales `[ntfy]` y `[textos]`.
3. Las rutas relativas (logs, archivo sqlite) se resuelven contra la carpeta del archivo de configuración.
4. Una configuración inválida muestra una ventana de error con el motivo y la aplicación sale con código 1. Es inválida si tiene:
   - una clave obligatoria ausente;
   - una clave desconocida en **cualquier** sección (p. ej. `pasword` mal escrito);
   - un valor fuera de rango: `connect_timeout_secs`, `interval_secs` y `window_secs` deben ser ≥ 1, y `query_timeout_secs` < `interval_secs`;
   - un `level` de log desconocido.
5. `[textos]` permite personalizar todos los mensajes que ve el operador, con variables como `{app}`, `{tabla}`, `{periodo}` y `{hora}`.
6. Logs diarios (`db-monitor.log.AAAA-MM-DD`) con nivel configurable.
7. Se documenta en el README cada sección y cada clave, con su valor por defecto.

**Validación:** pruebas unitarias de carga y validación de configuración, con casos válidos e inválidos (incluidas claves mal escritas y rangos).

| Tarea | Horas |
|---|---|
| Lectura de `config.toml` y resolución de rutas relativas | 3 |
| Validaciones (claves desconocidas, rangos, nivel de log) y ventana de error con el motivo | 4 |
| Sección `[textos]` con variables (`{app}`, `{tabla}`, `{periodo}`…) | 3 |
| Logs diarios con nivel configurable | 1 |
| Pruebas unitarias de configuración válida e inválida | 2 |
| Documentación en el README y archivos de ejemplo | 2 |
| **Total** | **15 h** |

---

## HU-03 · Detección de falta de datos nuevos en una tabla — 3 pts · 19 h

**Como** operador
**quiero** saber cuándo una tabla deja de recibir registros
**para** detectar fallas en la cadena de adquisición (p. ej. IEC-104) antes de que afecten la operación.

**Criterios de aceptación**
1. La tabla se define con `table`, una llave única (`key_column`) y una columna de fecha/timestamp (`time_column`).
2. En la primera conexión se valida que la tabla y las columnas existan. Si no existen, la aplicación termina con error.
3. Cada `interval_secs` cuenta las filas insertadas en los últimos `window_secs`, usando el reloj de la base de datos.
4. **Zona horaria:** se puede indicar si `time_column` guarda hora UTC o local del servidor (p. ej. `clock = "utc" | "local"`), y la ventana se calcula con el reloj correspondiente. Así no hay falsos rojos ni falsos verdes cuando el adquisidor guarda UTC y el servidor está en hora local.
5. Si hay filas, el estado pasa a OK (verde). Si no hay, pasa a "sin datos" (rojo) y queda un `WARN` en el log.
6. Si la tabla o una columna desaparece **después** de arrancar, el operador ve un error que lo indica (no "sin conexión") y queda registrado en el log.
7. Las filas con fecha futura (reloj del adquisidor adelantado) no cuentan como datos recientes, o generan un `WARN` en el log.
8. Los nombres de tabla y columnas se validan como identificadores simples, para evitar inyección SQL.
9. **Requisito de instalación:** `time_column` debe tener un índice, para que el conteo no recorra la tabla completa en cada ciclo. El README lo documenta.

**Validación:** pruebas de integración con SQLite, PostgreSQL y Oracle, insertando y dejando de insertar filas en la tabla de prueba, con datos en UTC y en hora local, con filas con fecha futura y borrando la tabla con la aplicación en marcha.

| Tarea | Horas |
|---|---|
| Validación de tabla y columnas en la primera conexión | 3 |
| Consulta de filas recientes por motor (Oracle, PostgreSQL, SQLite) con el reloj de la BD | 5 |
| Opción de zona horaria (UTC / local) y exclusión de fechas futuras | 2 |
| Error específico cuando la tabla desaparece en marcha | 1 |
| Validación de identificadores (contra inyección SQL) | 1 |
| Ciclo de chequeo cada `interval_secs` y registro en el log | 2 |
| Pruebas de integración insertando y deteniendo datos | 4 |
| Corrección de defectos | 1 |
| **Total** | **19 h** |

---

## HU-04 · Alertas en la bandeja de Windows (toasts) — 3 pts · 26 h

**Como** operador
**quiero** recibir notificaciones de Windows y ver un icono de estado en la bandeja
**para** enterarme de una caída de conexión o de la falta de datos sin revisar los logs.

**Criterios de aceptación**
1. Icono en la bandeja con tres colores: verde (OK), rojo (sin datos) y amarillo (sin conexión o conectando).
2. Toast al arrancar, al caer la conexión, al quedarse sin datos y al recuperarse.
3. Los avisos de rojo y amarillo se repiten en cada ciclo mientras dure el problema. Los de recuperación salen solo al cambiar de estado.
4. En rojo se abre además una ventana de advertencia (nunca más de una) que se cierra sola cuando vuelven los datos.
5. Si el operador cierra la ventana y sigue en rojo, vuelve a aparecer en el siguiente ciclo **sin robar el foco** a la aplicación que esté usando.
6. El menú del icono muestra el estado actual y las opciones "Abrir logs" y "Salir".
7. Solo puede correr una instancia a la vez.
8. Se ejecuta como tarea programada en la sesión del usuario y se relanza cada 5 minutos si se cerró. Por eso "Salir" no detiene la vigilancia de forma permanente: para eso hay que deshabilitar la tarea. El README lo documenta.
9. La carpeta de instalación debe ser escribible por el usuario de la tarea (lock, logs y, con HU-06, reescritura de `config.toml`). El script de instalación lo comprueba.
10. **Firma del ejecutable:** `db-monitor.exe` se entrega firmado con Authenticode, para que SmartScreen y el antivirus no lo bloqueen. **Dependencia:** certificado de firma de código provisto por TI.

**Validación:** pruebas unitarias de la máquina de estados de alertas, más una prueba manual en Windows 10/11 provocando cada estado, comprobando que la ventana no roba el foco y que el `.exe` firmado se ejecuta sin advertencia de SmartScreen.

| Tarea | Horas |
|---|---|
| Icono en la bandeja con 3 estados y menú (estado, Abrir logs, Salir) | 4 |
| Máquina de estados de alertas (repetición en cada ciclo, recuperación) | 4 |
| Toasts de Windows con remitente propio (acceso directo con AppUserModelID) | 4 |
| Ventana de advertencia única que se cierra sola y no roba el foco | 5 |
| Instancia única y tarea programada (`install-task.ps1`) | 3 |
| Firma Authenticode del ejecutable en el proceso de build | 2 |
| Pruebas unitarias y prueba manual en Windows 10/11 | 4 |
| **Total** | **26 h** |

---

## HU-05 · Notificaciones externas a móviles con ntfy — 3 pts · 15 h

**Como** responsable de guardia
**quiero** recibir las alertas en mi celular
**para** enterarme aunque no esté frente al equipo de la sala de control.

**Criterios de aceptación**
1. La sección opcional `[ntfy]` define `topic`, `server` (`https://ntfy.sh` por defecto o un servidor propio) y `token` (o la variable de entorno `NTFY_TOKEN`).
2. Cada toast, incluido el de arranque, se reenvía a ntfy con el mismo título y texto.
3. Prioridad alta para "sin datos" y "sin conexión", normal para la recuperación y el arranque.
4. El token **no** aparece en la línea de comandos del proceso de envío (se pasa por la entrada estándar), para que otros procesos del equipo no lo puedan leer.
5. Si el envío falla (sin internet, servidor caído), queda un `WARN` en el log y la aplicación sigue funcionando. No hay reintento: ese aviso se pierde, y el siguiente ciclo envía el aviso que corresponda.
6. **Requisito:** el equipo necesita salida a internet hacia el servidor ntfy.

**Validación:** suscribirse al `topic` desde la app móvil ntfy (Android/iOS), provocar cada estado y confirmar que llegan las notificaciones con la prioridad correcta. Probar también con la red cortada y comprobar que el token no aparece en la línea de comandos (Administrador de tareas → columna "Línea de comandos").

| Tarea | Horas |
|---|---|
| Sección `[ntfy]` (topic, server, token, variable `NTFY_TOKEN`) | 2 |
| Envío de cada toast a ntfy con prioridad según el estado | 4 |
| Token por la entrada estándar | 1 |
| Manejo de fallos de red sin detener la aplicación | 2 |
| Configuración de la app móvil ntfy y suscripción al topic | 2 |
| Pruebas en el móvil (cada estado, prioridad, red cortada) | 3 |
| Documentación | 1 |
| **Total** | **15 h** |

---

## HU-06 · Ofuscación de credenciales en el archivo de configuración — 3 pts · 17 h · Prioridad: Alta

> **Origen:** solicitud expresa del jefe del área de TI: no se permiten credenciales legibles en texto plano.

**Como** jefe del área de TI
**quiero** que el usuario y la contraseña no queden en texto plano en el archivo de configuración
**para** que nadie pueda leer las credenciales abriendo el archivo.

**Criterios de aceptación**
1. Los campos `user` y `password` de `[database]` se guardan ofuscados con XOR + Base64 y el prefijo `GSR:`, por ejemplo `password = "GSR:Kx8aBQ..."`. Por ser el campo equivalente, lo mismo aplica a `token` de `[ntfy]`.
2. Al iniciar, la aplicación lee la configuración y revisa esos campos:
   - Si **todos** llevan el prefijo `GSR:`, los desofusca en memoria y arranca de la forma habitual.
   - Si **alguno** está en texto plano (sin `GSR:`), avisa con un toast y en el log *"Credenciales visibles detectadas, iniciando el proceso de cifrado"*, ofusca esos campos, los reescribe en el archivo y luego continúa el arranque normal. El aviso no es una ventana modal, para que un arranque desatendido por la tarea programada no quede bloqueado.
3. Compatibilidad: un archivo actual sin ofuscar funciona sin cambios manuales. Queda ofuscado en el primer arranque.
4. Al reescribir, solo cambian los valores de esos campos: se conservan los comentarios (incluidos los de fin de línea), el orden y el resto de las secciones. La escritura es segura (se escribe a un archivo temporal y luego se reemplaza), de modo que el archivo nunca queda a medias.
5. El archivo reescrito conserva los permisos (ACL) del original. Funciona con archivos guardados con el Bloc de notas (BOM UTF-8 y saltos de línea CRLF).
6. La reescritura ocurre solo después de obtener el bloqueo de instancia única, para que dos instancias no escriban el archivo a la vez.
7. Los campos vacíos o ausentes (p. ej. SQLite, que no usa usuario ni contraseña) no se tocan.
8. Las variables de entorno `DB_PASSWORD` y `NTFY_TOKEN` siguen teniendo prioridad y se aceptan en claro. Su valor **nunca** se escribe en el archivo.
9. Si un valor con `GSR:` no se puede desofuscar (Base64 inválido o dato corrupto), se muestra una ventana de error que indica el campo y la aplicación sale con código 1, sin intentar conectarse.
10. Si no se puede escribir el archivo (permisos, solo lectura), se muestra una ventana de error con el motivo y la aplicación no arranca. Así nunca sigue funcionando con las credenciales en claro sin que nadie lo sepa.
11. Las credenciales en claro nunca aparecen en los logs, en la salida de depuración ni en los mensajes de error de sintaxis del archivo. El log registra solo *"credenciales ofuscadas en <ruta>"*.
12. El texto del aviso es una plantilla de `[textos]`, como el resto de los mensajes al operador.
13. El README documenta el prefijo `GSR:` y cómo cambiar una credencial: se escribe en claro y la aplicación la ofusca en el siguiente arranque.

**Limitación aceptada:** XOR + Base64 es ofuscación, no cifrado. Evita la lectura a simple vista, pero quien tenga el ejecutable puede revertirla, porque la clave viaja dentro del `.exe`. La mitigación real es el usuario de base de datos de solo lectura (HU-01, criterio 8). Ver HU-07 como mejora futura.

**Validación:** pruebas unitarias (ofuscar → desofuscar devuelve el valor original, prefijo, valor corrupto, campos vacíos) y prueba de integración con un archivo en claro: el primer arranque avisa y reescribe el archivo conservando comentarios, permisos y CRLF, y el segundo arranca sin aviso y se conecta. Probar también con `DB_PASSWORD` definida, comprobando que no se escribe en el archivo.

| Tarea | Horas |
|---|---|
| Rutina de ofuscación y desofuscación XOR + Base64 con prefijo `GSR:` | 2 |
| Detección de campos en claro al iniciar, aviso por toast y log | 2 |
| Reescritura del archivo conservando comentarios, formato, ACL, BOM y CRLF, con escritura segura | 5 |
| Bloqueo de instancia antes de reescribir y exclusión de variables de entorno | 1 |
| Manejo de errores (valor corrupto, sin permiso de escritura) y redacción de credenciales en logs y errores | 3 |
| Pruebas unitarias y de integración | 3 |
| Documentación en el README | 1 |
| **Total** | **17 h** |

---

## HU-07 · Cifrado de credenciales con Windows DPAPI (mejora futura) — 3 pts · 9 h · Prioridad: Baja

> **Origen:** propuesta del equipo de desarrollo, **no solicitada por TI**. Va al backlog fuera del sprint actual y depende de HU-06. Requiere la aprobación del jefe de TI antes de planificarla.

**Como** jefe del área de TI
**quiero** que las credenciales del archivo de configuración queden cifradas de verdad, y no solo ofuscadas
**para** que no se puedan recuperar aunque alguien tenga el archivo y el ejecutable.

**Contexto:** HU-06 ofusca con XOR + Base64 (`GSR:`). Eso evita que las credenciales se lean a simple vista, pero quien tenga el `.exe` puede revertirlo, porque la clave va dentro. DPAPI cifra con una clave que gestiona Windows, ligada al usuario y al equipo, sin guardar claves en la aplicación.

**Criterios de aceptación**
1. El campo `password` de `[database]` y `token` de `[ntfy]` se guardan cifrados con DPAPI (ámbito usuario actual) y el prefijo `DPAPI:`, por ejemplo `password = "DPAPI:AQAAANCMnd8B..."`. Cifrar también `user` queda a decisión de TI: aporta poco y complica el diagnóstico.
2. Al iniciar, la aplicación revisa cada campo:
   - Si lleva `DPAPI:`, lo descifra en memoria y arranca de la forma habitual.
   - Si está en texto plano o lleva `GSR:` (formato de HU-06), avisa *"Credenciales visibles detectadas, iniciando el proceso de cifrado"*, lo cifra con DPAPI, reescribe el archivo y continúa.
3. **Solo cifra el usuario de la tarea:** el cifrado ocurre únicamente cuando la aplicación la lanza la tarea programada (`--tarea`). Si un administrador la ejecuta a mano con su usuario y hay credenciales sin cifrar, se usa el valor en memoria sin reescribir el archivo y queda un `WARN` en el log. Así el archivo no queda cifrado para un usuario distinto del de la tarea.
4. Migración: un archivo ya ofuscado con `GSR:` pasa a `DPAPI:` en el primer arranque de la tarea, sin intervención manual.
5. Al reescribir, solo cambian esos valores y se conserva el resto del archivo. Se aplican la misma escritura segura y las mismas reglas de HU-06 (ACL, BOM/CRLF, bloqueo de instancia, variables de entorno).
6. Si un valor `DPAPI:` no se puede descifrar (archivo copiado de otro equipo o usuario, perfil recreado, contraseña reseteada por un administrador), se muestra una ventana de error: *"La credencial de <campo> fue cifrada en otro equipo o usuario; escríbala de nuevo en claro en config.toml"*, y la aplicación sale con código 1. En el siguiente arranque se vuelve a cifrar sola.
7. Las credenciales en claro nunca aparecen en los logs.
8. Las variables de entorno `DB_PASSWORD` y `NTFY_TOKEN` siguen teniendo prioridad y no se cifran.
9. En sistemas que no son Windows (desarrollo en macOS), un valor `DPAPI:` produce un error claro de "solo disponible en Windows". El resto de la aplicación no cambia.
10. El README documenta:
    - el prefijo `DPAPI:`;
    - que el archivo queda atado al equipo y al usuario que ejecuta la tarea programada;
    - que no se debe copiar un `config.toml` cifrado a otro equipo;
    - cómo recuperar una credencial: escribirla en claro.

**Limitaciones aceptadas**
- No protege contra un proceso que se ejecute como el mismo usuario, ni contra un administrador del equipo.
- Con cuentas locales, si un administrador resetea la contraseña del usuario, se pierde el acceso a la credencial cifrada, y hay que volver a escribirla en claro.

**Validación:** prueba manual en Windows 10/11 de los cinco escenarios:
- archivo en claro → cifrado al arrancar desde la tarea;
- archivo con `GSR:` → migrado a `DPAPI:`;
- segundo arranque sin aviso y conexión correcta;
- arranque manual con otro usuario → no reescribe el archivo;
- `config.toml` copiado a otro usuario o equipo → ventana de error, y al reescribir en claro se recupera.

Además, pruebas unitarias de detección de prefijos y del error en sistemas que no son Windows.

| Tarea | Horas |
|---|---|
| Cifrado y descifrado con DPAPI usando la crate `windows` ya existente | 2 |
| Detección de prefijos y migración de `GSR:` a `DPAPI:` | 2 |
| Cifrado solo desde la tarea programada | 1 |
| Mensajes de error y recuperación (otro equipo o usuario, otro sistema operativo) | 1 |
| Pruebas unitarias y manuales en Windows | 2 |
| Documentación en el README | 1 |
| **Total** | **9 h** |

---

## HU-08 · Aviso diario de funcionamiento ("sigo vivo") por ntfy — 3 pts · 8 h · Prioridad: Media

> **Origen:** propuesta del equipo de desarrollo a partir de la revisión de las HU. Backlog; depende de HU-05.

**Como** responsable de guardia
**quiero** recibir cada día un aviso de que la aplicación sigue funcionando
**para** darme cuenta si el equipo de la sala se apagó o la aplicación dejó de correr, algo que hoy nadie notifica.

**Criterios de aceptación**
1. Nueva clave opcional en `[ntfy]`: `heartbeat = "08:00"` (hora local del equipo). Sin la clave no se envía nada.
2. Cada día a esa hora se envía a ntfy un aviso con el estado actual (OK, sin datos o sin conexión), la hora del último chequeo y, en verde, el número de registros en la ventana.
3. Prioridad baja (2) cuando el estado es OK, para que no suene como una alerta; si el estado es rojo o amarillo, usa la prioridad de ese estado.
4. Si el equipo arranca después de la hora configurada, el aviso de ese día sale al arrancar. Nunca se envía más de un aviso por día.
5. El título y el cuerpo son plantillas de `[textos]` (`heartbeat.titulo`, `heartbeat.cuerpo`), con las variables `{estado}`, `{hora}` y `{n}`.
6. No genera toast en el equipo: es solo para el móvil.
7. Si el envío falla, queda un `WARN` en el log, igual que el resto de los avisos ntfy.

**Limitación aceptada:** la falla se detecta por **ausencia** del aviso, así que alguien tiene que notar que no llegó. Una detección automática requiere un servicio externo de tipo "dead man's switch" (p. ej. healthchecks.io), que queda fuera de esta HU.

**Validación:** pruebas unitarias del cálculo de la próxima hora de envío (antes y después de la hora, cambio de día, un solo envío por día) y prueba con la app móvil ntfy en los tres estados.

| Tarea | Horas |
|---|---|
| Clave `heartbeat` en `[ntfy]` y su validación | 1 |
| Programación del envío diario en el ciclo de eventos | 3 |
| Plantillas en `[textos]` | 1 |
| Pruebas unitarias y prueba en el móvil | 2 |
| Documentación en el README | 1 |
| **Total** | **8 h** |

---

## HU-09 · Retención de logs — 3 pts · 4 h · Prioridad: Media

> **Origen:** propuesta del equipo de desarrollo a partir de la revisión de las HU. Backlog.

**Como** administrador
**quiero** que la aplicación borre sola los logs antiguos
**para** que el disco del equipo de la sala no se llene con los años.

**Criterios de aceptación**
1. Nueva clave opcional en `[logging]`: `max_files` (días de logs que se conservan), con valor por defecto `30`. `0` desactiva el borrado.
2. Al arrancar y en cada cambio de día se borran los archivos `db-monitor.log.AAAA-MM-DD` más antiguos que exceden `max_files`.
3. Solo se borran archivos con ese patrón de nombre dentro de `logging.dir`; nunca otros archivos de la carpeta.
4. Si no se puede borrar un archivo (en uso, sin permisos), queda un `WARN` en el log y la aplicación sigue.
5. El README documenta la clave y su valor por defecto.

**Validación:** prueba unitaria o de integración con una carpeta temporal con más archivos de los permitidos, más otros archivos ajenos que no deben borrarse.

| Tarea | Horas |
|---|---|
| Clave `max_files` en `[logging]` y su validación | 1 |
| Borrado de logs antiguos al arrancar y al cambiar de día | 1 |
| Pruebas | 1 |
| Documentación en el README | 1 |
| **Total** | **4 h** |
