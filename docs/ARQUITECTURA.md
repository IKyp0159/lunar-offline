# Arquitectura

## Capas

```text
app (Tauri + Svelte)  ─┐
                       ├─→ lo-core ─→ lo-launch ─→ lo-accounts
lo-cli ────────────────┘      │            │            │
                              │            ├─→ lo-jvm    ├─→ lo-paths
                              │            ├─→ lo-detect └─→ lo-config
                              └─→ lo-diag ─┘
```

`lo-core` es la única API que consumen la ventana y la CLI. Ninguna de las dos
reimplementa lógica: la UI traduce eventos a estado y la CLI imprime el
resultado.

## Crates

| Crate | Responsabilidad |
|---|---|
| `lo-paths` | Rutas de `%APPDATA%`, `~/.lunarclient` y `.minecraft`; escritura atómica |
| `lo-config` | `config.json` v2: migración, claves desconocidas, perfiles y servidores |
| `lo-detect` | JRE, natives, classpath y versiones instaladas, leídos de disco |
| `lo-jvm` | Presets de JVM, validación de flags y prueba con `java -version` |
| `lo-accounts` | Swap aditivo de `accounts.json`, marcador y restauración |
| `lo-launch` | Comando de Genesis, proceso y salud del log |
| `lo-servers` | `servers.dat`: NBT comprimido, fusión con `config.json`, import/export |
| `lo-diag` | Informe de soporte en tipos y en texto |
| `lo-core` | Fachada asíncrona, estado de la partida y eventos |
| `lo-cli` | Subcomandos `selftest`, `launch`, `restore`, `versions`, `diagnose` |

## Flujo de arranque

```text
Core::launch
  ├─ comprobar que Lunar Client.exe no corre
  ├─ guardar logs/cmd-<marca>.txt
  ├─ AccountsManager::swap
  │    ├─ escribir swap.pending
  │    ├─ copiar accounts.json → accounts.original.json (una vez)
  │    └─ añadir cuenta legacy y activarla
  ├─ build_plan → javaw.exe + argumentos
  ├─ Supervisor::start
  │    ├─ Popen con CREATE_NO_WINDOW y stdout al log
  │    ├─ tail del log en vivo
  │    └─ emitir LaunchEvent
  └─ al terminar: AccountsManager::restore
```

Si el proceso no llega a arrancar, `accounts.json` se restaura antes de
devolver el error: nunca queda una cuenta temporal viva en disco.

## Invariantes

**Cuentas.** El swap no borra ni reescribe cuentas existentes. `swap.pending`
se escribe antes de tocar `accounts.json`, de modo que un cierre brusco deja
testigo. La copia original se crea una sola vez y no se sobrescribe. La
restauración fusiona: gana la sesión más nueva, se elimina la cuenta temporal y
se devuelve la activa original; una cuenta con la que el jugador haya iniciado
sesión mientras jugaba sobrevive.

**Configuración.** La raíz de `config.json` manda y el perfil activo se
sincroniza con ella al cargar. Las claves desconocidas se conservan en todos los
niveles. Un fichero sin `schema` se migra a v2 con copia previa. Un fichero
corrupto se aparta a `config.json.corrupto` y se arranca con valores por
defecto. Un `config.json` de una versión más nueva se lee sin degradarlo.

**Servidores.** `servers.dat` no se toca si no se entiende: gzip o NBT
inválidos dejan el fichero en paz. Las etiquetas NBT desconocidas se conservan,
incluido el icono. Antes de escribir se deja una copia `servers.dat.bak`. La
fusión no borra entradas y empareja por nombre, así que los nombres repetidos se
renombran con sufijo.

**Arranque.** El comando se construye como `Vec<OsString>`, nunca concatenando
cadenas, y se guarda en `logs/cmd-<marca>.txt` para comparar. La JVM descarta
flags que cambiarían qué se ejecuta o romperían el arranque; los que el jugador
puede confirmar se piden explícitamente.

**Diagnóstico.** El informe no contiene tokens ni contenido de cuentas: solo
existencia, conteos y el resultado de construir el comando.

## Concurrencia

`Core` mantiene el estado de la partida detrás de un `Mutex` y expone una API
asíncrona. El supervisor corre en su propia tarea, lee el log desde el byte cero
y emite eventos; la UI no consulta el proceso, escucha. El proceso del juego no
se mata al cerrar la ventana del launcher: si el jugador cierra la ventana, el
juego sigue y la restauración la hace el siguiente arranque.

## Compatibilidad

Solo Windows 10/11 x64. El binario enlaza el runtime de C estáticamente
(`+crt-static`) y no depende de `vcruntime140.dll` ni de `WebView2Loader.dll`.
