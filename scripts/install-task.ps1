# Registra db-monitor como tarea programada que arranca al iniciar sesion (ver db-monitor-task.xml).
# Uso, desde la carpeta con db-monitor.exe, config.toml y estos dos archivos:
#   powershell -ExecutionPolicy Bypass -File install-task.ps1               # usuario actual
#   powershell -ExecutionPolicy Bypass -File install-task.ps1 -Usuario SALA\operador   # otro usuario: como administrador
#   powershell -ExecutionPolicy Bypass -File install-task.ps1 -Desinstalar
# Textos sin tildes: PowerShell 5.1 lee los .ps1 sin BOM como ANSI.
param(
    [string]$Exe = (Join-Path $PSScriptRoot 'db-monitor.exe'),
    [string]$Usuario = "$env:USERDOMAIN\$env:USERNAME",
    [switch]$Desinstalar
)
$ErrorActionPreference = 'Stop'
$Nombre = 'db-monitor'

if ($Desinstalar) {
    Stop-ScheduledTask -TaskName $Nombre -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $Nombre -Confirm:$false
    # remitente de los toasts que registraba db-monitor (versiones eabae29..f9be713)
    Remove-Item 'HKCU:\Software\Classes\AppUserModelId\DbMonitor.Alertas' -Recurse -ErrorAction SilentlyContinue
    # accesos directos del menu Inicio que crea db-monitor para firmar los toasts
    $Shell = New-Object -ComObject WScript.Shell
    Get-ChildItem "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\*.lnk" |
        Where-Object { $Shell.CreateShortcut($_.FullName).TargetPath -eq $Exe } |
        ForEach-Object { Remove-Item $_.FullName; Write-Host "Acceso directo eliminado: $($_.Name)" }
    Write-Host "Tarea '$Nombre' eliminada."
    return
}

$Exe = (Resolve-Path $Exe).Path
$Dir = Split-Path $Exe
if (-not (Test-Path (Join-Path $Dir 'config.toml'))) {
    throw "Falta config.toml en $Dir (copia una plantilla config.*.example.toml)."
}
# logs/ y db-monitor.lock se escriben junto a config.toml: la carpeta debe ser escribible por el usuario
# (no uses Program Files).

[xml]$Tarea = Get-Content -Raw -Encoding UTF8 (Join-Path $PSScriptRoot 'db-monitor-task.xml')
$Tarea.Task.Triggers.LogonTrigger.UserId = $Usuario
$Tarea.Task.Principals.Principal.UserId = $Usuario
$Tarea.Task.Actions.Exec.Command = $Exe
$Tarea.Task.Actions.Exec.WorkingDirectory = $Dir

Register-ScheduledTask -TaskName $Nombre -Xml $Tarea.OuterXml -Force | Out-Null
Write-Host "Tarea '$Nombre' registrada para $Usuario -> $Exe"
Write-Host "Arranca al iniciar sesion y se relanza cada 5 min si no esta corriendo."
Write-Host "Para pararla del todo: deshabilita la tarea o usa -Desinstalar ('Salir' solo la cierra hasta el siguiente relanzamiento)."

if ($Usuario -eq "$env:USERDOMAIN\$env:USERNAME") {
    # si ya hay uno abierto a mano, esta instancia sale sin avisar (--tarea)
    Start-ScheduledTask -TaskName $Nombre
    Write-Host "Lanzada ahora."
}
