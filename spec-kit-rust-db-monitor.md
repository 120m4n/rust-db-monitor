# Spec Kit - Aplicación Consola Rust para Monitoreo de Base de Datos en Windows 10

## 1. Resumen
Desarrollar una aplicación de consola en Rust para Windows 10 que ejecute consultas periódicas contra una base de datos, procese los resultados y registre la actividad del sistema.

## 2. Objetivo
- Conectarse a una base de datos configurada.
- Ejecutar consultas SQL cada N minutos.
- Detectar cambios o nuevos registros.
- Registrar eventos en archivos de log.
- Funcionar continuamente con bajo consumo de recursos.
- Recuperarse automáticamente ante fallos temporales de conectividad.

## 3. Requisitos Funcionales
### RF-001: Inicialización
- Leer configuración desde archivo.
- Validar parámetros al iniciar.
- Mostrar versión y estado inicial.

### RF-002: Conexión a Base de Datos
Soporte para PostgreSQL, SQL Server y MySQL.

### RF-003: Consulta Periódica
Ejecutar consulta configurable cada minuto.

### RF-004: Procesamiento
Procesar resultados y registrar eventos.

### RF-005: Reconexión Automática
Reintentos automáticos tras pérdida de conexión.

### RF-006: Apagado Controlado
Cerrar conexiones, logs y tareas activas.

## 4. Requisitos No Funcionales
- Uso RAM < 100 MB
- Uso CPU < 5%
- Operación 24/7
- Configuración externa mediante config.toml

## 5. Arquitectura
```text
Rust Application
 |- Scheduler
 |- Logger
 |- Database Client
```

## 6. Dependencias
```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
tokio-postgres = "0.7"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
chrono = "0.4"
tracing = "0.1"
tracing-subscriber = "0.3"
anyhow = "1.0"
```

## 7. Estructura Proyecto
```text
db-monitor/
├── Cargo.toml
├── config.toml
├── src/
└── logs/
```

## 8. Criterios de Aceptación
- Inicia correctamente en Windows 10.
- Lee configuración desde config.toml.
- Conecta a PostgreSQL.
- Ejecuta consultas cada 60 segundos.
- Registra logs diarios.
- Se recupera automáticamente ante fallos.
- Preparada para Windows Service o Tray Icon.

## 9. Uso por Agente de Código
Este documento está diseñado para servir como especificación funcional de entrada para agentes de generación de código (Claude Code, Copilot Coding Agent, OpenHands, Aider, Cursor Agent, etc.).
