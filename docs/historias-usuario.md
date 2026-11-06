# Historias de usuario — db-monitor

Definición funcional para el sprint (Azure DevOps). Cada historia cubre una parte de la aplicación con sus propias validaciones; todas convergen en un único entregable.

**Entregable único del sprint:** `db-monitor.exe` para Windows 10/11, junto con `config.toml` de ejemplo, el script `install-task.ps1` + `db-monitor-task.xml` (arranque automático al iniciar sesión) y el README con la referencia de configuración.

> Notas: la configuración es `config.toml` (TOML), no `.xml`; el único `.xml` es `db-monitor-task.xml` (tarea programada). La revisión de datos nuevos usa una llave única (`key_column`) y una columna de fecha (`time_column`), no una FK.

## Resumen

| HU | Título | Prioridad | Puntos | Horas | Sprint |
|---|---|---|---|---|---|
| HU-01 | Conexión a base de datos Oracle | Normal | 3 | 18 h | Actual |
| HU-02 | Configuración por secciones desde archivo | Normal | 3 | 14 h | Actual |
| HU-03 | Detección de falta de datos nuevos en una tabla | Normal | 3 | 16 h | Actual |
| HU-04 | Alertas en la bandeja de Windows (toasts) | Normal | 3 | 22 h | Actual |
| HU-05 | Notificaciones externas a móviles con ntfy | Normal | 3 | 14 h | Actual |
| HU-06 | Ofuscación de credenciales en el archivo de configuración | **Alta** | 3 | 14 h | Actual |
| | **Total sprint** | | **18** | **98 h** | |
| HU-07 | Cifrado de credenciales con Windows DPAPI (mejora futura) | Baja | 3 | 8 h | Backlog |

---

## HU-01 · Conexión a base de datos Oracle — 3 pts · 18 h

**Como** operador de la sala de control
**quiero** que la aplicación se conecte a la base de datos Oracle de producción
**para** poder vigilar sus tablas sin intervención manual.

**Criterios de aceptación**
1. Se conecta a Oracle con host, puerto (1521 por defecto), usuario, contraseña y *service name*.
2. La contraseña puede venir de la variable de entorno `DB_PASSWORD`, que tiene prioridad sobre el archivo.
3. Si la conexión tarda más de `connect_timeout_secs`, se aborta y se reintenta.
4. Ante una caída de la base, reintenta sola con espera exponencial (1 s, 2 s, 4 s… hasta `backoff_max_secs`) sin cerrarse.
5. Si una consulta pasa de `query_timeout_secs`, se interrumpe y se registra en el log.
6. Funciona también con PostgreSQL y SQLite, que se usan para las pruebas.

**Validación:** pruebas de integración contra Oracle real (`TEST_ORA_*`), cortando la red o la base para comprobar los reintentos y la reconexión.

| Tarea | Horas |
|---|---|
| Conexión Oracle (Instant Client, service name, timeout de conexión) | 5 |
| Soporte PostgreSQL y SQLite para pruebas | 3 |
| Reconexión con espera exponencial y timeout de consulta | 4 |
| Contraseña desde la variable `DB_PASSWORD` | 1 |
| Pruebas de integración contra Oracle y PostgreSQL reales | 4 |
| Corrección de defectos | 1 |
| **Total** | **18 h** |

---

## HU-02 · Configuración por secciones desde archivo — 3 pts · 14 h

**Como** administrador
**quiero** configurar la aplicación con un archivo de texto dividido en secciones
**para** adaptarla a cada instalación sin recompilar.

**Criterios de aceptación**
1. Archivo `config.toml` junto al ejecutable, o una ruta pasada como argumento.
2. Secciones: `[database]`, `[freshness]` (obligatoria), `[logging]`, y las opcionales `[ntfy]` y `[textos]`.
3. Las rutas relativas (logs, archivo sqlite) se resuelven contra la carpeta del archivo de configuración.
4. Una configuración inválida (clave obligatoria ausente, valor fuera de rango, clave desconocida en `[textos]`) muestra una ventana de error con el motivo y la aplicación sale con código 1.
5. `[textos]` permite personalizar todos los mensajes que ve el operador, con variables como `{app}`, `{tabla}`, `{periodo}` y `{hora}`.
6. Logs diarios (`db-monitor.log.AAAA-MM-DD`) con nivel configurable.
7. Se documenta en el README cada sección y cada clave, con su valor por defecto.

**Validación:** pruebas unitarias de carga y validación de configuración, con casos válidos e inválidos.

| Tarea | Horas |
|---|---|
| Lectura de `config.toml` y resolución de rutas relativas | 3 |
| Validaciones y ventana de error con el motivo | 3 |
| Sección `[textos]` con variables (`{app}`, `{tabla}`, `{periodo}`…) | 3 |
| Logs diarios con nivel configurable | 1 |
| Pruebas unitarias de configuración válida e inválida | 2 |
| Documentación en el README y archivos de ejemplo | 2 |
| **Total** | **14 h** |

---

## HU-03 · Detección de falta de datos nuevos en una tabla — 3 pts · 16 h

**Como** operador
**quiero** saber cuándo una tabla deja de recibir registros
**para** detectar fallas en la cadena de adquisición (p. ej. IEC-104) antes de que afecten la operación.

**Criterios de aceptación**
1. La tabla se define con `table`, una llave única (`key_column`) y una columna de fecha/timestamp (`time_column`).
2. En la primera conexión se valida que la tabla y las columnas existan. Si no existen, la aplicación termina con error.
3. Cada `interval_secs` cuenta las filas insertadas en los últimos `window_secs`, usando el reloj de la base de datos.
4. Si hay filas, el estado pasa a OK (verde). Si no hay, pasa a "sin datos" (rojo) y queda un `WARN` en el log.
5. Los nombres de tabla y columnas se validan como identificadores simples, para evitar inyección SQL.

**Validación:** pruebas de integración con SQLite, PostgreSQL y Oracle, insertando y dejando de insertar filas en la tabla de prueba.

| Tarea | Horas |
|---|---|
| Validación de tabla y columnas en la primera conexión | 3 |
| Consulta de filas recientes por motor (Oracle, PostgreSQL, SQLite) con el reloj de la BD | 5 |
| Validación de identificadores (contra inyección SQL) | 1 |
| Ciclo de chequeo cada `interval_secs` y registro en el log | 2 |
| Pruebas de integración insertando y deteniendo datos | 4 |
| Corrección de defectos | 1 |
| **Total** | **16 h** |

---

## HU-04 · Alertas en la bandeja de Windows (toasts) — 3 pts · 22 h

**Como** operador
**quiero** recibir notificaciones de Windows y ver un icono de estado en la bandeja
**para** enterarme de una caída de conexión o de la falta de datos sin revisar los logs.

**Criterios de aceptación**
1. Icono en la bandeja con tres colores: verde (OK), rojo (sin datos) y amarillo (sin conexión o conectando).
2. Toast al arrancar, al caer la conexión, al quedarse sin datos y al recuperarse.
3. Los avisos de rojo y amarillo se repiten en cada ciclo mientras dure el problema. Los de recuperación salen solo al cambiar de estado.
4. En rojo se abre además una ventana de advertencia (nunca más de una) que se cierra sola cuando vuelven los datos.
5. El menú del icono muestra el estado actual y las opciones "Abrir logs" y "Salir".
6. Solo puede correr una instancia a la vez.
7. Se ejecuta como tarea programada en la sesión del usuario y se relanza cada 5 minutos si se cerró.

**Validación:** pruebas unitarias de la máquina de estados de alertas, más una prueba manual en Windows 10/11 provocando cada estado.

| Tarea | Horas |
|---|---|
| Icono en la bandeja con 3 estados y menú (estado, Abrir logs, Salir) | 4 |
| Máquina de estados de alertas (repetición en cada ciclo, recuperación) | 4 |
| Toasts de Windows con remitente propio (acceso directo con AppUserModelID) | 4 |
| Ventana de advertencia única que se cierra sola | 3 |
| Instancia única y tarea programada (`install-task.ps1`) | 3 |
| Pruebas unitarias y prueba manual en Windows 10/11 | 4 |
| **Total** | **22 h** |

---

## HU-05 · Notificaciones externas a móviles con ntfy — 3 pts · 14 h

**Como** responsable de guardia
**quiero** recibir las alertas en mi celular
**para** enterarme aunque no esté frente al equipo de la sala de control.

**Criterios de aceptación**
1. La sección opcional `[ntfy]` define `topic`, `server` (`https://ntfy.sh` por defecto o un servidor propio) y `token` (o la variable de entorno `NTFY_TOKEN`).
2. Cada toast se reenvía a ntfy con el mismo título y texto.
3. Prioridad alta para "sin datos" y "sin conexión", normal para la recuperación y el arranque.
4. Si el envío falla (sin internet, servidor caído), queda un `WARN` en el log y la aplicación sigue funcionando.
5. **Requisito:** el equipo necesita salida a internet hacia el servidor ntfy.

**Validación:** suscribirse al `topic` desde la app móvil ntfy (Android/iOS), provocar cada estado y confirmar que llegan las notificaciones con la prioridad correcta. Probar también con la red cortada.

| Tarea | Horas |
|---|---|
| Sección `[ntfy]` (topic, server, token, variable `NTFY_TOKEN`) | 2 |
| Envío de cada toast a ntfy con prioridad según el estado | 4 |
| Manejo de fallos de red sin detener la aplicación | 2 |
| Configuración de la app móvil ntfy y suscripción al topic | 2 |
| Pruebas en el móvil (cada estado, prioridad, red cortada) | 3 |
| Documentación | 1 |
| **Total** | **14 h** |

---

## HU-06 · Ofuscación de credenciales en el archivo de configuración — 3 pts · 14 h · Prioridad: Alta

> **Origen:** solicitud expresa del jefe del área de TI: no se permiten credenciales legibles en texto plano.

**Como** jefe del área de TI
**quiero** que el usuario y la contraseña no queden en texto plano en el archivo de configuración
**para** que nadie pueda leer las credenciales abriendo el archivo.

**Criterios de aceptación**
1. Los campos `user` y `password` de `[database]` se guardan ofuscados con XOR + Base64 y el prefijo `GSR:`, por ejemplo `password = "GSR:Kx8aBQ..."`. Por ser el campo equivalente, lo mismo aplica a `token` de `[ntfy]`.
2. Al iniciar, la aplicación lee la configuración y revisa esos campos:
   - Si **todos** llevan el prefijo `GSR:`, los desofusca en memoria y arranca de la forma habitual.
   - Si **alguno** está en texto plano (sin `GSR:`), muestra el mensaje *"Credenciales no visibles detectadas, iniciando el proceso de cifrado"*, ofusca esos campos, los reescribe en el archivo y luego continúa el arranque normal.
3. Compatibilidad: un archivo actual sin ofuscar funciona sin cambios manuales. Queda ofuscado en el primer arranque.
4. Al reescribir, solo cambian los valores de esos campos: se conservan los comentarios, el orden y el resto de las secciones. La escritura es segura (se escribe a un archivo temporal y luego se reemplaza), de modo que el archivo nunca queda a medias.
5. Los campos vacíos o ausentes (p. ej. SQLite, que no usa usuario ni contraseña) no se tocan.
6. Si un valor con `GSR:` no se puede desofuscar (Base64 inválido o dato corrupto), se muestra una ventana de error que indica el campo y la aplicación sale con código 1, sin intentar conectarse.
7. Si no se puede escribir el archivo (permisos, solo lectura), se muestra una ventana de error con el motivo y la aplicación no arranca. Así nunca sigue funcionando con las credenciales en claro sin que nadie lo sepa.
8. Las credenciales en claro nunca aparecen en los logs. El log registra solo *"credenciales ofuscadas en <ruta>"*.
9. Las variables de entorno `DB_PASSWORD` y `NTFY_TOKEN` siguen teniendo prioridad y se aceptan en claro, porque no se guardan en el archivo.
10. El README documenta el prefijo `GSR:` y cómo cambiar una credencial: se escribe en claro y la aplicación la ofusca en el siguiente arranque.

**Limitación aceptada:** XOR + Base64 es ofuscación, no cifrado. Evita la lectura a simple vista, pero quien tenga el ejecutable puede revertirla, porque la clave viaja dentro del `.exe`. Ver HU-07 como mejora futura.

**Validación:** pruebas unitarias (ofuscar → desofuscar devuelve el valor original, prefijo, valor corrupto, campos vacíos) y prueba de integración con un archivo en claro: el primer arranque muestra el mensaje y reescribe el archivo, y el segundo arranca sin mensaje y se conecta.

| Tarea | Horas |
|---|---|
| Rutina de ofuscación y desofuscación XOR + Base64 con prefijo `GSR:` | 2 |
| Detección de campos en claro al iniciar y mensaje al operador | 2 |
| Reescritura del archivo conservando comentarios y formato, con escritura segura | 4 |
| Manejo de errores (valor corrupto, archivo sin permiso de escritura) | 2 |
| Pruebas unitarias y de integración | 3 |
| Documentación en el README | 1 |
| **Total** | **14 h** |

---

## HU-07 · Cifrado de credenciales con Windows DPAPI (mejora futura) — 3 pts · 8 h · Prioridad: Baja

> **Origen:** propuesta del equipo de desarrollo, **no solicitada por TI**. Va al backlog fuera del sprint actual y depende de HU-06. Requiere la aprobación del jefe de TI antes de planificarla.

**Como** jefe del área de TI
**quiero** que las credenciales del archivo de configuración queden cifradas de verdad, y no solo ofuscadas
**para** que no se puedan recuperar aunque alguien tenga el archivo y el ejecutable.

**Contexto:** HU-06 ofusca con XOR + Base64 (`GSR:`). Eso evita que las credenciales se lean a simple vista, pero quien tenga el `.exe` puede revertirlo, porque la clave va dentro. DPAPI cifra con una clave que gestiona Windows, ligada al usuario y al equipo, sin guardar claves en la aplicación.

**Criterios de aceptación**
1. Los campos `user` y `password` de `[database]` y `token` de `[ntfy]` se guardan cifrados con DPAPI (ámbito usuario actual) y el prefijo `DPAPI:`, por ejemplo `password = "DPAPI:AQAAANCMnd8B..."`.
2. Al iniciar, la aplicación revisa cada campo:
   - Si lleva `DPAPI:`, lo descifra en memoria y arranca de la forma habitual.
   - Si está en texto plano o lleva `GSR:` (formato de HU-06), muestra el mensaje *"Credenciales no visibles detectadas, iniciando el proceso de cifrado"*, lo cifra con DPAPI, reescribe el archivo y continúa.
3. Migración: un archivo ya ofuscado con `GSR:` pasa a `DPAPI:` en el primer arranque, sin intervención manual.
4. Al reescribir, solo cambian esos valores y se conserva el resto del archivo. Se aplica la misma escritura segura de HU-06.
5. Si un valor `DPAPI:` no se puede descifrar (archivo copiado de otro equipo o usuario, perfil recreado, contraseña reseteada por un administrador), se muestra una ventana de error: *"La credencial de <campo> fue cifrada en otro equipo o usuario; escríbala de nuevo en claro en config.toml"*, y la aplicación sale con código 1. En el siguiente arranque se vuelve a cifrar sola.
6. Las credenciales en claro nunca aparecen en los logs.
7. Las variables de entorno `DB_PASSWORD` y `NTFY_TOKEN` siguen teniendo prioridad y no se cifran.
8. En sistemas que no son Windows (desarrollo en macOS), un valor `DPAPI:` produce un error claro de "solo disponible en Windows". El resto de la aplicación no cambia.
9. El README documenta:
   - el prefijo `DPAPI:`;
   - que el archivo queda atado al equipo y al usuario que ejecuta la tarea programada;
   - que no se debe copiar un `config.toml` cifrado a otro equipo;
   - cómo recuperar una credencial: escribirla en claro.

**Limitaciones aceptadas**
- No protege contra un proceso que se ejecute como el mismo usuario, ni contra un administrador del equipo.
- Con cuentas locales, si un administrador resetea la contraseña del usuario, se pierde el acceso a la credencial cifrada, y hay que volver a escribirla en claro.

**Validación:** prueba manual en Windows 10/11 de los cuatro escenarios:
- archivo en claro → cifrado;
- archivo con `GSR:` → migrado a `DPAPI:`;
- segundo arranque sin mensaje y conexión correcta;
- `config.toml` copiado a otro usuario o equipo → ventana de error, y al reescribir en claro se recupera.

Además, pruebas unitarias de detección de prefijos y del error en sistemas que no son Windows.

| Tarea | Horas |
|---|---|
| Cifrado y descifrado con DPAPI usando la crate `windows` ya existente | 2 |
| Detección de prefijos y migración de `GSR:` a `DPAPI:` | 2 |
| Mensajes de error y recuperación (otro equipo o usuario, otro sistema operativo) | 1 |
| Pruebas unitarias y manuales en Windows | 2 |
| Documentación en el README | 1 |
| **Total** | **8 h** |
