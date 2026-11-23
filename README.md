# Lunar Offline Launcher

Launcher de Lunar Client para jugadores no premium, escrito en Rust. Arranca el
core Genesis con una sesión offline sin tocar las cuentas reales: escribe una
cuenta temporal en `accounts.json`, lanza el juego y restaura el fichero al
cerrar.

- Un solo `.exe` portable para Windows 10/11 x64. Sin instalador, sin
  dependencias externas, sin telemetría.
- Usa los archivos que el launcher oficial ya descargó. No descarga ni
  redistribuye nada de Lunar ni de Mojang.
- Interfaz monócroma, en español, sobre Tauri 2 + Svelte 5. La misma biblioteca
  responde por consola cuando se le pasan argumentos.
- Licencia MIT.

## Cómo funciona

1. Comprueba que `Lunar Client.exe` esté cerrado.
2. Guarda una copia de `accounts.json` y la marca de swap; añade una cuenta
   `legacy` con el nick indicado y la deja activa.
3. Construye el comando de Genesis con los artefactos instalados y lanza
   `javaw.exe` con el log en vivo.
4. Al salir el juego, fusiona lo que haya cambiado y restaura las cuentas
   originales. Si la app muere a mitad, el siguiente arranque restaura.

La JVM usa los flags del launcher oficial. La memoria y los flags extra se
configuran por perfil; los flags que cambiarían qué se ejecuta (`-cp`, `-jar`,
`-javaagent`, `-Djava.library.path`, `-Dlunar.*`) se descartan con aviso.

## Requisitos

- Windows 10/11 de 64 bits.
- WebView2 Runtime (incluido en Windows 11 y en Windows 10 actualizado).
- Lunar Client instalado y abierto al menos una vez, con 1.8.9 o 1.7.10
  descargadas.

## Uso

Doble clic en `Lunar Offline.exe`. Las vistas son Jugar, Perfiles, Servidores,
Registro y Avanzado; la consola muestra el log del juego en directo.

Desde una terminal:

```text
"Lunar Offline.exe" selftest [--out <ruta>]
"Lunar Offline.exe" launch --nick <nick> [--version 1.8.9] [--dry-run]
"Lunar Offline.exe" restore
"Lunar Offline.exe" versions
"Lunar Offline.exe" diagnose [--out <ruta>]
```

`launch --dry-run` valida e imprime el comando sin lanzar nada.

## Datos

Todo vive en `%APPDATA%\LunarOffline`:

| Fichero | Contenido |
|---|---|
| `config.json` | Ajustes, perfiles, servidores y JVM |
| `config.json.bak` | Copia del guardado anterior |
| `accounts.original.json` | Cuentas tal como estaban antes del swap |
| `swap.pending` | Marcador de swap en curso, para recuperar tras un cierre brusco |
| `logs\` | Log de cada partida y comando exacto usado |

`config.json` es aditivo: las claves que no conoce se conservan al guardar.

## Compilar

```powershell
.\build.ps1
```

Genera `dist\Lunar Offline.exe` y su `.sha256`. Requiere Rust estable con el
toolchain del repo, Node 22 y Visual Studio Build Tools con Windows Kits.

- `-SkipChecks` omite `cargo deny` y `cargo audit`.
- `-NoFrontend` reutiliza `app\dist`.

Detalles en [docs/EMPAQUETADO.md](docs/EMPAQUETADO.md).

## Estructura

```text
crates/lo-paths      rutas de Lunar, %APPDATA% y .minecraft
crates/lo-config     config.json con migración y claves preservadas
crates/lo-servers    servers.dat: lectura, fusión e import/export
crates/lo-jvm        presets de JVM y validación de flags
crates/lo-detect     JRE, natives, classpath y versiones en disco
crates/lo-accounts   swap y restauración de accounts.json
crates/lo-launch     comando de arranque y supervisor del proceso
crates/lo-diag       informe de diagnóstico
crates/lo-core       fachada que consumen la UI y la CLI
crates/lo-cli        comandos de consola
app                  ventana Tauri 2 + Svelte 5
```

Diseño y garantías en [docs/ARQUITECTURA.md](docs/ARQUITECTURA.md).

## Límites

- La sesión es offline: los servidores premium rechazan la conexión.
- Sin cosméticos ni skins premium.
- Solo 1.8.9 y 1.7.10, las versiones que el launcher oficial deja en
  `offline/multiver`.

## Licencia

MIT. Lunar Client y Minecraft son propiedad de sus respectivos titulares; este
proyecto no está afiliado a ninguno de los dos.
