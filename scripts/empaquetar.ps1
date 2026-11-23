# Compatibilidad: empaqueta con `build.ps1` y añade el LEEME.
#
# Uso:
#   .\scripts\empaquetar.ps1
#
# Genera `dist\Lunar Offline.exe`, su sha256 y `dist\LEEME.txt`.

$ErrorActionPreference = "Continue"
$raiz = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $raiz "dist"

& (Join-Path $raiz "build.ps1") -SkipChecks
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$destino = Join-Path $dist "Lunar Offline.exe"
if (-not (Test-Path $destino)) {
    throw "no se generó $destino"
}

$textoVersion = (Select-String -Path (Join-Path $raiz "Cargo.toml") -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$tamano = [math]::Round((Get-Item $destino).Length / 1MB, 2)
$hash = (Get-FileHash $destino -Algorithm SHA256).Hash.ToLower()

$leeme = @"
Lunar Offline Launcher $textoVersion
=====================================================================

Un solo fichero: Lunar Offline.exe ($tamano MB). No hay instalador.
Bórralo para desinstalar: no escribe en el registro ni deja servicios.
Los datos (config.json, sus copias y los logs) viven en %APPDATA%\LunarOffline.

Qué necesita
------------
- Windows 10/11 de 64 bits.
- WebView2 Runtime (viene con Windows 11 y con Windows 10 actualizado).
- Lunar Client instalado y abierto al menos una vez, con 1.8.9 o 1.7.10
  descargadas. Este launcher usa esos archivos; no descarga ni redistribuye
  nada de Lunar ni de Mojang.

Cómo se usa
-----------
1. Cierra "Lunar Client.exe" antes de pulsar Jugar: el launcher oficial
   reescribe accounts.json y se pisaría la cuenta temporal.
2. Escribe tu nick (1-16 letras, números o guion bajo), elige versión y RAM.
3. Jugar. El launcher guarda tu accounts.json, pone una cuenta offline, arranca
   el juego y restaura el fichero original al salir.

También sirve desde la consola:
  "Lunar Offline.exe" selftest
  "Lunar Offline.exe" launch --nick Steve --dry-run
  "Lunar Offline.exe" restore

Verificar la descarga
---------------------
  certutil -hashfile "Lunar Offline.exe" SHA256

Límites
-------
- No funciona en servidores premium: la sesión es offline (Invalid session).
- Sin cosméticos ni skins premium.
- Sin telemetría: nada sale del equipo.
- Licencia MIT.

sha256: $hash
"@
Set-Content -Path (Join-Path $dist "LEEME.txt") -Value $leeme -Encoding UTF8

Write-Host ""
Write-Host "listo:" -ForegroundColor Green
Write-Host "  $destino  ($tamano MB)"
Write-Host "  $hash"
