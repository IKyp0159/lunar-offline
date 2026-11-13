# Compila "Lunar Offline.exe" (portable, sin dependencias en la maquina destino).
#
#   pip install pyinstaller
#   .\build.ps1
#
# Resultado: dist\Lunar Offline.exe

[CmdletBinding()]
param(
    [string]$Name = "Lunar Offline",
    [string]$Entry = "lunar_offline.py"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
Push-Location $root
try {
    if (-not (Test-Path $Entry)) {
        throw "No encuentro $Entry en $root"
    }

    $pyi = $null
    try {
        $pyi = (python -m PyInstaller --version 2>$null)
    } catch {
        $pyi = $null
    }

    if (-not $pyi) {
        Write-Host "PyInstaller no esta instalado. Instalando..." -ForegroundColor Yellow
        python -m pip install --upgrade pyinstaller
        if ($LASTEXITCODE -ne 0) { throw "Fallo la instalacion de PyInstaller." }
    }

    Write-Host "Compilando $Name.exe ..." -ForegroundColor Cyan
    python -m PyInstaller `
        --noconfirm `
        --onefile `
        --windowed `
        --clean `
        --name $Name `
        --distpath "$root\dist" `
        --workpath "$root\build" `
        --specpath "$root\build" `
        $Entry

    if ($LASTEXITCODE -ne 0) { throw "PyInstaller fallo con codigo $LASTEXITCODE." }

    $exe = Join-Path $root "dist\$Name.exe"
    $mb = [math]::Round((Get-Item $exe).Length / 1MB, 2)
    Write-Host ""
    Write-Host "Listo: $exe  ($mb MB)" -ForegroundColor Green
    Write-Host "Prueba: & `"$exe`" --selftest" -ForegroundColor DarkGray
} finally {
    Pop-Location
}
