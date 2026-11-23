# Construye el único `.exe` portable de Lunar Offline (GUI + CLI).
#
# Uso:
#   .\build.ps1                 frontend + release + sha256
#   .\build.ps1 -SkipChecks     sin `cargo deny` / `cargo audit`
#   .\build.ps1 -NoFrontend     reutiliza `app\dist` ya compilado
#
# Salida:
#   dist\Lunar Offline.exe
#   dist\Lunar Offline.exe.sha256
#
# No genera instalador: el `.exe` se copia y se usa tal cual. Los datos viven
# en %APPDATA%\LunarOffline; nunca se escribe al lado del ejecutable.
#
# `$ErrorActionPreference = "Continue"` a propósito: PowerShell 5.1 convierte
# los avisos de `cargo`/`npm` por stderr en errores terminantes aunque el
# proceso salga con 0. Cada paso se comprueba con `$LASTEXITCODE`.

param(
    [switch] $SkipChecks,
    [switch] $NoFrontend
)

$ErrorActionPreference = "Continue"
$raiz = $PSScriptRoot
$app = Join-Path $raiz "app"
$dist = Join-Path $raiz "dist"

# `cargo`/`rustc` viven en el perfil del usuario; una consola nueva puede no
# tenerlos en PATH.
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$env:CARGO_TERM_COLOR = "always"

function Invoke-Nativo {
    param(
        [Parameter(Mandatory = $true)][string] $Nombre,
        [Parameter(Mandatory = $true)][scriptblock] $Accion
    )
    & $Accion | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "$Nombre terminó con código $LASTEXITCODE"
    }
}

function Get-Herramienta {
    param([Parameter(Mandatory = $true)][string] $Nombre)
    $comando = Get-Command $Nombre -ErrorAction SilentlyContinue
    if ($comando) { return $comando.Source }
    return $null
}

# `tauri-build` compila los recursos de Windows con `rc.exe` (Windows Kits) y el
# enlace usa `link.exe` (MSVC). Visual Studio Build Tools no los pone en PATH en
# una consola normal: en una compilación limpia falla con «Are you sure you have
# RC.EXE in your $PATH». Se añaden sus carpetas si existen.
function Add-VsToolsToPath {
    $patrones = @(
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64",
        "${env:ProgramFiles}\Windows Kits\10\bin\*\x64",
        "${env:ProgramFiles}\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64",
        "${env:ProgramFiles(x86)}\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64"
    )
    foreach ($patron in $patrones) {
        $carpetas = Get-ChildItem $patron -Directory -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending
        foreach ($carpeta in $carpetas) {
            if ($env:PATH -notlike "*$($carpeta.FullName)*") {
                $env:PATH = "$($carpeta.FullName);$env:PATH"
            }
        }
    }
}

Add-VsToolsToPath

if (-not $SkipChecks) {
    Write-Host "1/5  licencias, avisos y vulnerabilidades (cargo deny + cargo audit)" -ForegroundColor Cyan
    Push-Location $raiz
    try {
        if (Get-Herramienta "cargo-deny") {
            Invoke-Nativo "cargo deny" { cargo deny check }
        }
        else {
            Write-Warning "cargo-deny no está instalado: cargo install cargo-deny --locked"
        }
        if (Get-Herramienta "cargo-audit") {
            Invoke-Nativo "cargo audit" { cargo audit }
        }
        else {
            Write-Warning "cargo-audit no está instalado: cargo install cargo-audit --locked"
        }
    }
    finally {
        Pop-Location
    }
}
else {
    Write-Host "1/5  deny/audit omitidos (-SkipChecks)" -ForegroundColor DarkGray
}

if (-not $NoFrontend) {
    Write-Host "2/5  frontend (npm ci + vite build)" -ForegroundColor Cyan
    Push-Location $app
    try {
        Invoke-Nativo "npm ci" { npm ci --no-fund --no-audit }
        Invoke-Nativo "npm run build" { npm run build }
    }
    finally {
        Pop-Location
    }
}
else {
    Write-Host "2/5  frontend omitido (-NoFrontend)" -ForegroundColor DarkGray
}

Write-Host "3/5  binario release (cargo, +crt-static)" -ForegroundColor Cyan
Push-Location $raiz
try {
    Invoke-Nativo "cargo build" {
        cargo build --release --locked -p lunar-offline-app --features custom-protocol
    }
}
finally {
    Pop-Location
}

Write-Host "4/5  copia y sha256" -ForegroundColor Cyan
New-Item -ItemType Directory -Force $dist | Out-Null
$origen = Join-Path $raiz "target\release\lunar-offline.exe"
if (-not (Test-Path $origen)) {
    throw "no se generó $origen"
}
$destino = Join-Path $dist "Lunar Offline.exe"
if (Get-Process -Name "Lunar Offline" -ErrorAction SilentlyContinue) {
    throw "cierra 'Lunar Offline' antes de compilar: Windows no deja reemplazar el .exe en uso"
}
$hashOrigen = (Get-FileHash $origen -Algorithm SHA256).Hash.ToLower()
try {
    Copy-Item $origen $destino -Force -ErrorAction Stop
}
catch {
    throw "no se pudo copiar a $destino (¿sigue abierto el .exe?): $($_.Exception.Message)"
}
$hash = (Get-FileHash $destino -Algorithm SHA256).Hash.ToLower()
if ($hash -ne $hashOrigen) {
    throw "la copia no coincide con el binario compilado: $destino"
}
Set-Content -Path "$destino.sha256" -Value "$hash  Lunar Offline.exe" -Encoding ASCII

Write-Host "5/5  dependencias y tamaño" -ForegroundColor Cyan
$dumpbin = Get-Herramienta "dumpbin"
if (-not $dumpbin) {
    # Visual Studio no siempre está en PATH; se busca donde lo deja instalado.
    $patrones = @(
        "${env:ProgramFiles}\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe",
        "${env:ProgramFiles(x86)}\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe",
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\dumpbin.exe"
    )
    foreach ($patron in $patrones) {
        $candidato = Get-ChildItem $patron -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($candidato) { $dumpbin = $candidato.FullName; break }
    }
}
if ($dumpbin) {
    $dependencias = & $dumpbin /nologo /dependents $destino 2>&1 | Out-String
    $dependencias | Out-Host
    if ($dependencias -match "vcruntime140") {
        Write-Warning "aparece vcruntime140.dll: revisa .cargo/config.toml (+crt-static)"
    }
    if ($dependencias -match "WebView2Loader\.dll") {
        Write-Warning "aparece WebView2Loader.dll: debería ir enlazado estáticamente"
    }
}
else {
    Write-Host "dumpbin no está en PATH; para comprobar dependencias abre una consola de Visual Studio." -ForegroundColor DarkGray
}

$bytes = (Get-Item $destino).Length
$tamano = [math]::Round($bytes / 1MB, 2)
Write-Host ""
if ($bytes -gt 15MB) {
    Write-Warning "el .exe supera el objetivo de 15 MB ($tamano MB); prueba opt-level = `"s`""
}
Write-Host "listo:" -ForegroundColor Green
Write-Host "  $destino  ($tamano MB, $bytes bytes)"
Write-Host "  sha256 $hash"
exit 0
