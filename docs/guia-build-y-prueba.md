# Guía: build, configuración y prueba

Paso a paso para compilar `db-monitor`, configurarlo y comprobar que funciona sin depender de ningún servidor de base de datos (la prueba usa SQLite, que va incluido en el binario). La referencia completa de cada opción está en el [README](../README.md).

## 1. Build

### macOS Apple Silicon (M1/M2/…)

```sh
./build.sh          # compila, deja el resultado en dist/ y borra target/
./build.sh --keep   # igual, pero conserva target/ para recompilar más rápido
```

Resultado:

```
dist/db-monitor            ejecutable arm64
dist/config.example.toml   plantilla de configuración
```

### Windows 10

```
cargo build --release
```

El ejecutable queda en `target\release\db-monitor.exe`. También lo publica el CI de GitHub en cada push (artefacto `db-monitor-windows`).

### Windows desde macOS (Docker)

Con Docker Desktop corriendo:

```sh
docker compose run --rm build-windows
```

Resultado: `dist/windows/db-monitor.exe` (Windows x86_64, toolchain MinGW) y `config.example.toml`. Al final lista las DLL que importa el `.exe`; todas deben ser del sistema (`kernel32.dll`, `ws2_32.dll`, `msvcrt.dll`…). El primer build tarda unos 3 minutos; los siguientes reusan la caché de los volúmenes `cargo-registry` y `target-windows`.

## 2. Configuración (`config.toml`)

La configuración es un archivo **TOML** (no XML).

1. Copia la plantilla junto al ejecutable:

   ```sh
   cp dist/config.example.toml dist/config.toml
   ```

2. Sin argumentos, el programa busca `config.toml` **en la misma carpeta que el ejecutable**. Para usar otro archivo: `db-monitor ruta/al/config.toml`.
3. Las rutas relativas (`dbname` de SQLite, `logs/`, `state.toml`) se resuelven contra la carpeta del `config.toml`.
4. Las secciones son cuatro:

   | Sección | Para qué |
   |---|---|
   | `[database]` | Motor (`postgres`, `sqlite`, `oracle`) y datos de conexión |
   | `[monitor]` | Consulta de registros nuevos e intervalo |
   | `[freshness]` | Opcional: avisa si una tabla deja de recibir filas |
   | `[logging]` | Carpeta y nivel de los logs |

5. La consulta de `[monitor]` debe contener `{last_id}`, devolver una columna entera `id`, ordenar por `id` y limitar las filas.
6. La contraseña conviene ponerla en la variable de entorno `DB_PASSWORD` en lugar del archivo.

## 3. Prueba con SQLite (no requiere servidor)

### 3.1 Preparar una carpeta de prueba

```sh
mkdir -p ~/prueba-monitor
cp dist/db-monitor ~/prueba-monitor/
scripts/sqlite-demo.sh ~/prueba-monitor/demo.db 3
```

`sqlite-demo.sh` crea la tabla `events (id, message, created_at)` y le inserta 3 filas. Salida esperada: `filas: 3, max id: 3`.

### 3.2 Crear `~/prueba-monitor/config.toml`

```toml
[database]
kind = "sqlite"
dbname = "demo.db"

[monitor]
query = "SELECT id, message FROM events WHERE id > {last_id} ORDER BY id LIMIT 1000"
interval_secs = 2

[freshness]
table = "events"
key_column = "id"
time_column = "created_at"
window_secs = 10
```

### 3.3 Ejecutar

```sh
cd ~/prueba-monitor
./db-monitor
```

Deberías ver:

```
INFO conectado
INFO nuevo registro row={"id": Some("1"), "message": Some("evento ...")}
INFO nuevo registro row={"id": Some("2"), ...}
INFO nuevo registro row={"id": Some("3"), ...}
INFO ciclo: 3 filas, last_id=3
INFO frescura: 3 registros en `events` en los últimos 10s
```

### 3.4 Comprobaciones

Con el monitor corriendo, abre otra terminal en la raíz del repo:

| Qué probar | Cómo | Resultado esperado |
|---|---|---|
| Detecta filas nuevas | `scripts/sqlite-demo.sh ~/prueba-monitor/demo.db 2` | Aparecen `nuevo registro` para los ids 4 y 5 |
| Inserción continua | `scripts/sqlite-demo.sh ~/prueba-monitor/demo.db 1 3` (Ctrl-C para parar) | Una fila nueva cada 3 s |
| Aviso de frescura | No insertes nada durante más de 10 s | `WARN sin registros nuevos en 'events' durante los últimos 10s` |
| Apagado ordenado | Ctrl-C en la terminal del monitor | `INFO apagado` y sale con código 0 |
| Marca de agua persistente | `cat state.toml`, luego vuelve a ejecutar `./db-monitor` | `last_id = N`; al reiniciar no repite filas ya vistas |
| Logs diarios | `ls logs/` | `db-monitor.log.AAAA-MM-DD` |
| Error de configuración | Cambia `table = "events"` por `"eventos"` y ejecuta | `ERROR freshness table 'eventos' ... no such table` y sale con código 1 |

Para repetir la prueba desde cero: borra `demo.db`, `state.toml` y `logs/`.

## 4. La misma prueba con otro motor

Solo cambia `[database]` y la forma de limitar filas en `query`. El resto del config y las comprobaciones de la sección 3.4 son iguales; las filas se insertan con el cliente de cada motor en lugar de `sqlite-demo.sh`.

Tabla de prueba equivalente:

```sql
-- PostgreSQL
CREATE TABLE events (id bigserial PRIMARY KEY, message text, created_at timestamp DEFAULT now());
INSERT INTO events (message) VALUES ('evento 1'), ('evento 2');

-- Oracle 12c+
CREATE TABLE events (id NUMBER GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                     message VARCHAR2(100), created_at TIMESTAMP DEFAULT SYSTIMESTAMP);
INSERT INTO events (message) VALUES ('evento 1');
COMMIT;
```

PostgreSQL:

```toml
[database]
kind = "postgres"
host = "127.0.0.1"
user = "monitor"
dbname = "mydb"

[monitor]
query = "SELECT id, message FROM events WHERE id > {last_id} ORDER BY id LIMIT 1000"
```

Oracle (requiere Oracle Instant Client; en macOS, las `.dylib` junto al ejecutable):

```toml
[database]
kind = "oracle"
host = "127.0.0.1"
user = "monitor"
dbname = "ORCLPDB1"   # service name

[monitor]
query = "SELECT id, message FROM events WHERE id > {last_id} ORDER BY id FETCH FIRST 1000 ROWS ONLY"
```

Antes de ejecutar: `export DB_PASSWORD=...` (en Windows, `set DB_PASSWORD=...`).

## 5. Tests automáticos

```sh
cargo test                                                   # unitarios + SQLite, sin servidores
cargo test --test db_targets -- --ignored --test-threads=1   # Postgres y Oracle reales
```

Los tests de Postgres y Oracle leen `TEST_PG_*` y `TEST_ORA_*` (`HOST`, `PORT`, `USER`, `PASSWORD`, `NAME`).
