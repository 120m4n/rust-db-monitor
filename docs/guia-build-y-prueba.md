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
dist/config.iec104.example.toml   plantilla del caso IEC-104 (Oracle)
```

### Windows 10

```
cargo build --release
```

El ejecutable queda en `target\release\db-monitor.exe`.

### Windows desde macOS (Docker)

Con Docker Desktop corriendo:

```sh
docker compose run --rm build-windows
```

Resultado: `dist/windows/db-monitor.exe` (Windows x86_64, toolchain MinGW) y las dos plantillas (`config.example.toml`, `config.iec104.example.toml`). Al final lista las DLL que importa el `.exe`; todas deben ser del sistema (`kernel32.dll`, `ws2_32.dll`, `msvcrt.dll`…). El primer build tarda unos 3 minutos; los siguientes reusan la caché de los volúmenes `cargo-registry` y `target-windows`.

## 2. Configuración (`config.toml`)

La configuración es un archivo **TOML** (no XML).

1. Copia la plantilla junto al ejecutable:

   ```sh
   cp dist/config.example.toml dist/config.toml
   ```

2. Sin argumentos, el programa busca `config.toml` **en la misma carpeta que el ejecutable**. Para usar otro archivo: `db-monitor ruta/al/config.toml`.
3. Las rutas relativas (`dbname` de SQLite, `logs/`) se resuelven contra la carpeta del `config.toml`.
4. Las secciones son tres:

   | Sección | Para qué |
   |---|---|
   | `[database]` | Motor (`postgres`, `sqlite`, `oracle`) y datos de conexión |
   | `[freshness]` | Tabla vigilada, ventana de tiempo e intervalo de chequeo |
   | `[logging]` | Carpeta y nivel de los logs |

5. Un config de versiones anteriores con `[monitor]` da error: mueve `interval_secs`, `query_timeout_secs` y `backoff_max_secs` a `[freshness]` y borra el resto de esa sección.
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

[freshness]
table = "events"
key_column = "id"
time_column = "created_at"
window_secs = 10
interval_secs = 2
```

### 3.3 Ejecutar

```sh
cd ~/prueba-monitor
./db-monitor
```

No se abre ninguna ventana: sale un toast "db-monitor en ejecución" y aparece un icono en la barra de menú (macOS) o en la bandeja (Windows), primero amarillo ("Conectando…") y enseguida verde o rojo. Lanzado desde una terminal, el log también sale por pantalla:

```
INFO conectado
INFO frescura: 3 registros en `events` en los últimos 10s
```

### 3.4 Comprobaciones

Con el monitor corriendo, abre otra terminal en la raíz del repo:

| Qué probar | Cómo | Resultado esperado |
|---|---|---|
| Inserción continua | `scripts/sqlite-demo.sh ~/prueba-monitor/demo.db 1 3` (Ctrl-C para parar) | Icono verde; tooltip `OK · N registros en 10 s · chequeo HH:MM` |
| Tabla sin datos | Para el script y espera más de 10 s | Icono rojo, toast "sin datos" y ventana de advertencia |
| Ventana insistente | Cierra la ventana de advertencia | Vuelve a aparecer en el siguiente ciclo (~2 s) |
| Recuperación | `scripts/sqlite-demo.sh ~/prueba-monitor/demo.db 1` | Icono verde, toast "OK" y la ventana se cierra sola |
| Una sola instancia | Lanza `./db-monitor` otra vez | Ventana "db-monitor ya está en ejecución"; la segunda sale |
| Menú | Clic en el icono → Abrir logs | Se abre la carpeta `logs/` |
| Apagado ordenado | Clic en el icono → Salir | El icono desaparece; el log termina en `INFO apagado` |
| Sin conexión | En otra carpeta, config `postgres` a `127.0.0.1` puerto `1` | Amarillo "Conectando…"; a los 60 s toast "sin conexión" |
| Logs diarios | `ls logs/` | `db-monitor.log.AAAA-MM-DD`, con una línea `aviso: …` por cada toast |
| Error de configuración | Cambia `table = "events"` por `"eventos"` y ejecuta | Ventana de error `freshness table 'eventos' ... no such table`; sale con código 1 |

Para repetir la prueba desde cero: borra `demo.db` y `logs/`.

## 4. La misma prueba con otro motor

Solo cambia `[database]`. El resto del config y las comprobaciones de la sección 3.4 son iguales; las filas se insertan con el cliente de cada motor en lugar de `sqlite-demo.sh`.

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

```

Oracle (requiere Oracle Instant Client; en macOS, las `.dylib` junto al ejecutable):

```toml
[database]
kind = "oracle"
host = "127.0.0.1"
user = "monitor"
dbname = "ORCLPDB1"   # service name

```

Antes de ejecutar: `export DB_PASSWORD=...` (en Windows, `set DB_PASSWORD=...`).

## 5. Tests automáticos

```sh
cargo test                                                   # unitarios + SQLite, sin servidores
cargo test --test db_targets -- --ignored --test-threads=1   # Postgres y Oracle reales
```

Los tests de Postgres y Oracle leen `TEST_PG_*` y `TEST_ORA_*` (`HOST`, `PORT`, `USER`, `PASSWORD`, `NAME`).
