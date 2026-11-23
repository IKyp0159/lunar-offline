# Empaquetado

Genera `dist/Lunar Offline.exe`: un solo binario con la ventana, la CLI y el
frontend embebido. No hay instalador: el `.exe` se copia, se ejecuta y se borra
sin dejar nada en el registro ni escribir junto al ejecutable.

## Requisitos

- Rust MSVC (`rustup` + Visual Studio Build Tools). `rust-toolchain.toml` fija
  la versión del compilador.
- Node.js 22 (solo para compilar el frontend).
- WebView2 Runtime en el equipo destino. Si falta, la app muestra un diálogo
  nativo con el enlace de descarga y sale con código 1.

## Construir

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
.\build.ps1
```

Opciones:

```text
-SkipChecks    no ejecuta `cargo deny` ni `cargo audit`
-NoFrontend    reutiliza `app\dist` ya compilado
```

Salida:

```text
dist/Lunar Offline.exe          binario único (GUI + CLI, frontend embebido)
dist/Lunar Offline.exe.sha256   hash para verificar la descarga
```

`scripts\empaquetar.ps1` delega en `build.ps1 -SkipChecks` y añade
`dist/LEEME.txt` con las notas de entrega.

## Pasos

1. `cargo deny check` y `cargo audit` si están instalados (avisa si no).
2. `npm ci` y `npm run build` en `app/` → `app/dist`.
3. `cargo build --release --locked -p lunar-offline-app --features custom-protocol`.
4. Copia `target/release/lunar-offline.exe` a `dist/Lunar Offline.exe`, calcula
   el sha256 y lo escribe en formato `sha256sum`.
5. `dumpbin /dependents` si está disponible y comprobación de tamaño
   (objetivo ≤ 15 MB).

El script antepone al `PATH` las carpetas de Windows Kits (`rc.exe`) y de MSVC
(`link.exe`): en una compilación limpia `tauri-winres` falla si no encuentra
`rc.exe`, aunque Build Tools esté instalado.

## Un solo fichero

- Tauri embebe `app/dist` con `generate_context!`: no hay carpeta de assets al
  lado.
- `.cargo/config.toml` añade `-C target-feature=+crt-static` para
  `x86_64-pc-windows-msvc`: no hace falta `vcruntime140.dll` ni
  `WebView2Loader.dll` junto al `.exe`.
- `app/app.manifest` fija Common-Controls v6, `asInvoker`, DPI `PerMonitorV2` y
  `longPathAware`. Sin `<assemblyIdentity>`: con identidad propia Windows
  devuelve el error 14001.
- El binario es de subsistema Windows. Si los argumentos piden un subcomando
  (`selftest`, `launch`, `restore`, `versions`, `diagnose`, `help`,
  `--version`), engancha la consola del padre con
  `AttachConsole(ATTACH_PARENT_PROCESS)` antes de ejecutar la CLI. Sin
  argumentos abre la ventana.

## Compilar a mano

```powershell
cd app; npm run build; cd ..
cargo build --release --locked -p lunar-offline-app --features custom-protocol
```

Dos detalles que importan:

- **`--features custom-protocol`**: Tauri 2 decide `is_dev()` como
  `!cfg!(feature = "custom-protocol")`. Sin la feature, el binario de release
  carga `devUrl` (`http://localhost:1420`) y la ventana sale con
  `ERR_CONNECTION_REFUSED`. Un `compile_error!` en `app/src/main.rs` rompe el
  build si se olvida.
- **`frontendDist: "dist"`** en `app/tauri.conf.json`: la ruta se resuelve desde
  `app/`, no desde la raíz. Con `"../dist"` el binario arranca con
  `asset not found: index.html`.

## Reproducibilidad

`.cargo/config.toml` pasa `/Brepro`, que quita la marca de tiempo del encabezado
PE y la firma del PDB. Dos compilaciones limpias del mismo código dan el mismo
sha256:

```powershell
cargo clean -p lunar-offline-app
cargo build --release --locked -p lunar-offline-app --features custom-protocol
```

`build.ps1` comprueba que `Lunar Offline.exe` no esté abierto antes de copiar y
verifica que el hash de `dist/` coincide con el del binario compilado, para no
dejar un artefacto viejo con un `.sha256` nuevo.

## Verificar la descarga

```powershell
certutil -hashfile "Lunar Offline.exe" SHA256
```

Debe coincidir con `Lunar Offline.exe.sha256`.

## Firma de código (opcional)

No se incluye certificado. Con uno propio:

```powershell
signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 `
  /f mi-certificado.pfx /p <contraseña> "dist\Lunar Offline.exe"
```

Firmar cambia el hash: hay que recalcular el `.sha256` después de firmar.

## CI

`.github/workflows/ci.yml` comprueba formato, clippy, `cargo deny` y
`cargo audit`. En etiquetas `v*` compila en `windows-latest` con `--locked`,
sube el `.exe` y el `.sha256` como artefacto y los publica en la release. El
trabajo `release` falla si el binario supera 15 MB o si `dumpbin` encuentra
`vcruntime140`/`WebView2Loader`.
