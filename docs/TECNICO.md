# Ingeniería inversa del arranque de Lunar Client

Documento de lo que costó que Lunar arrancara de verdad con una sesión offline.
Todo sale del `main.js` del launcher oficial (`app.asar` →
`dist-electron/electron/main.js`), no de documentación: no hay documentación
pública de esto.

Versión del launcher analizada: Lunar Client `3.7.36`.

---

## 1. El punto de entrada

El launcher oficial no lanza Minecraft directamente. Lanza un proceso Java con
esta clase:

```
com.moonsworth.lunar.genesis.Genesis
```

`Genesis` es el *bootstrapper*. Lee los artefactos declarados en
`--ichorClassPath`, los pasa por el pipeline de **ichor** (el sistema de
transformación de bytecode de Lunar: mixins, hooks, remapeo) y arranca el
Minecraft real desde las clases ya transformadas (el *bake*).

Sus argumentos reconocidos, sacados de `javap -p -c` sobre la clase:

```
version, classpathDir, workingDirectory, integrationTest,
ichorClassPath, ichorExternalFiles, dumpClasses, partialJar,
installationId, overrides, mx-cache
```

Todo lo que no esté en esa lista se ignora silenciosamente. De ahí el famoso:

```
Completely ignored arguments: [--workingDirectory, ., --classpathDir, ...]
```

No es un error: son los argumentos de Minecraft que Genesis no consume y pasa
hacia adelante.

---

## 2. El classpath: la causa del arranque "vanilla"

Este es el fallo que producía el síntoma *"abre Minecraft vanilla, no Lunar"*.

### Lo que había

```
javaw -cp genesis-0.1.0-SNAPSHOT-all.jar com.moonsworth.lunar.genesis.Genesis ...
```

Con solo `genesis.jar` en el classpath **de la JVM**, ichor no veía
`lunar.jar`. Sin `lunar.jar` no hay tabla de transformaciones, así que el
pipeline horneaba las clases crudas de Minecraft y arrancaba un juego vanilla
con el loader de Lunar encima.

Diagnóstico en el log:

```
Config hash: 6f63c7f8  File hash: 80dda92b
Loaded 4389 baked classes.
```

4 389 clases es exactamente un Minecraft 1.8.9 sin tocar.

### Lo que hay que hacer

El launcher oficial construye el `-cp` con **todos** los artefactos marcados
`CLASS_PATH`, en rutas absolutas unidas por `;`:

```js
`-cp`, aX(wJe(e.artifacts).map(t => join(e.fileCacheRoot, t)))
```

Y el mismo conjunto, en nombres relativos separados por `,`, va en
`--ichorClassPath`:

```js
`--ichorClassPath`, ...
```

Con eso:

```
Config hash: 72c13d03  File hash: 2ea6e126
Loaded 19039 baked classes.
```

19 039 clases. Ahí sí entra Lunar entero.

### Orden de la lista

Importa. El orden usado, y el que funciona:

```
lunar-platform-mappings-v1_8.jar
lunar-platform-mappings-v26_2.jar
optifine-0.1.0-SNAPSHOT-all.jar
common-0.1.0-SNAPSHOT-all-nomappings.jar
legacy-0.1.0-SNAPSHOT-all-nomappings.jar
modern-0.1.0-SNAPSHOT-all-nomappings.jar
lunar-lang.jar
genesis-0.1.0-SNAPSHOT-all.jar
lunar.jar
```

Los mappings van primero (remapean los nombres ofuscados de Mojang a los
nombres que espera Lunar), `lunar.jar` al final (es el que aporta el volumen
grande: 19 305 clases).

Los jars `*mixins*` (`lunar-modmenu-mixins`, `lunar-sodium-mixins`,
`lunar-fabric-mixins`, …) se cargan aparte por el sistema de módulos de ichor
(`.ichor/ichormodule-libs`), no por el classpath principal. Meterlos en el `-cp`
no rompe, pero tampoco hace falta.

---

## 3. `--version` es el subversion, no la versión

Error clásico:

```
--version 1.8.9     # mal
--version 1.8       # bien
```

Genesis parsea el argumento con un `subVersion` de dos componentes:

```
Launching Minecraft 1.8.9 v1_8 (parsed from 1.8)
```

Si le pasas `1.8.9` completo, busca un artefacto `v1_8_9` que no existe y cae a
un path por defecto. La forma válida:

| versión de Minecraft | `--version` |
|---|---|
| 1.8.9 | `1.8` |
| 1.7.10 | `1.7` |

El nombre largo (`1.8.9`) sigue siendo lo que muestra la UI del launcher; el
subversion es lo que viaja en la línea de comandos.

---

## 4. `--webosrDir` y los recursos de Ultralight

Lunar renderiza su HUD, el mod menu y la tienda con **Ultralight** (un motor
HTML embebido). Sus recursos (`cacert.pem`, `icudt67l.dat`, `mediaControls.css`,
…) viven en:

```
~/.lunarclient/offline/multiver/natives/<hash>/resources/
```

Si `--webosrDir` apunta a `<natives>/web` (que no existe) o al directorio
equivocado:

```
java.lang.RuntimeException: Did not find Ultralight Resource Directory
```

El valor correcto es el directorio `natives/<hash>` **pelado**. Es el mismo
directorio que se pasa en `-Djava.library.path` para `lwjgl64.dll`.

Los de texto, UI y JIT van separados:

```
--texturesDir ~/.lunarclient/textures
--uiDir       ~/.lunarclient/ui
--jitDir      ~/.lunarclient/jit
```

El log lo confirma cuando están bien:

```
[Bridge] Found textures dir: ...\.lunarclient\textures
[Bridge] Found JIT dir: ...\.lunarclient\jit
[Bridge] Found ui dir: ...\.lunarclient\ui
[Bridge] Found webosr dir: ...\natives\<hash>\web
```

(Ojo: el bridge reporta `<natives>/web`, pero el argumento que hay que pasar es
`<natives>`. El bridge añade `/web` él mismo.)

---

## 5. `--server` necesita `host:puerto` en un solo argumento

Minecraft 1.8.9 lanza con:

```
--server 127.0.0.1 --port 25566
```

Resultado:

```
Connecting to 127.0.0.1, 25565
```

`--port` no existe como argumento de arranque de Minecraft en esa versión: el
puerto se ignora y el juego conecta al default. La forma que sí funciona:

```
--server 127.0.0.1:25566
```

```
Connecting to 127.0.0.1, 25566
```

Verificado con un servidor de prueba en 25566.

---

## 6. La sesión: Lunar ignora `--username`

Aunque se pase:

```
--username TestOffline --accessToken 0 --userType legacy --uuid <uuid offline>
```

Minecraft arranca con ese usuario (el log de `Client thread` lo muestra), pero
**inmediatamente después** Lunar lo pisa:

```
[LC Accounts] Setting account 'Klyvrax' as the current session.
[LC] Setting user: Klyvrax
```

Lunar lee la cuenta activa de:

```
~/.lunarclient/settings/game/accounts.json
```

y la aplica por encima de lo que digan los argumentos. No hay flag para
desactivarlo.

### La solución

Sustituir `accounts.json` por una cuenta `legacy` con el nick deseado, lanzar,
y restaurar el original al terminar:

```json
{
  "activeAccountLocalId": "<uuid-hex>",
  "accounts": {
    "<uuid-hex>": {
      "accessToken": "0",
      "accessTokenExpiresAt": "1970-01-01T00:00:00Z",
      "eligibleForMigration": false,
      "hasMultipleProfiles": false,
      "legacy": true,
      "persistent": false,
      "localId": "<uuid-hex>",
      "refreshToken": "",
      "minecraftProfile": { "id": "<uuid>", "name": "<nick>" },
      "remoteId": "",
      "type": "Xbox",
      "username": "<nick>"
    }
  }
}
```

Resultado:

```
[LC Accounts] Setting account 'TestOffline' as the current session.
[LC] Setting user: TestOffline
```

### Restauración segura

Tres piezas, para que un cierre sucio no destruya las cuentas del usuario:

1. `accounts.original.json` — copia byte a byte del original, en
   `%APPDATA%\LunarOffline\`.
2. `swap.pending` — marca de "hay un swap vivo". Se escribe **antes** de tocar
   `accounts.json`.
3. En el arranque de la app: si existe la marca o la copia, se restaura antes de
   mostrar la ventana.

Y el orden importa: si `swap.pending` ya existe, **no** se vuelve a copiar el
original — si no, el "original" sería la cuenta offline y la del usuario se
perdería para siempre.

---

## 7. UUID offline

El estándar de cualquier servidor `online-mode=false`:

```
"OfflinePlayer:" + nick
  → MD5
  → byte[6] = (byte[6] & 0x0F) | 0x30    // versión 3
  → byte[8] = (byte[8] & 0x3F) | 0x80    // variante RFC 4122
  → UUID
```

Mismo nick = mismo UUID = mismo inventario, en todos los servidores offline y en
todas las sesiones.

---

## 8. JVM flags

Los que usa el launcher oficial y los que se replican:

```
-XX:+UnlockExperimentalVMOptions
-XX:+UseG1GC
-XX:G1NewSizePercent=20
-XX:G1ReservePercent=20
-XX:MaxGCPauseMillis=50
-XX:G1HeapRegionSize=32M
-XX:+DisableExplicitGC
-XX:+ParallelRefProcEnabled
-XX:+UseStringDeduplication
-XX:+OptimizeStringConcat
-Dlog4j2.formatMsgNoLookups=true
-Dichor.usingIsolatedProfiles=true
-XX:+DisableAttachMechanism
```

Más los de rutas:

```
-Djava.library.path=<natives>
-Dlunar.webosr.url=file:index.html
-Dichor.logsFile=<profile>/logs/ichor-boot.log
-Dlog4j.configurationFile=<profile>/logs/config.xml
```

`-Dichor.usingIsolatedProfiles=true` hace que cada profile (1.8, 1.7, 26) tenga
su propio directorio de config, mods y logs bajo `~/.lunarclient/profiles/`.

`-Dlunar.webosr.url=file:index.html` es el documento raíz del webview de Lunar.
Sin él, el HUD intenta cargar una URL remota.

---

## 9. Detección de artefactos

El launcher no tiene una lista fija: los artefactos vienen del endpoint de
`/launch` y se cachean en disco. Por eso este proyecto **no** trae un manifiesto
hardcodeado; detecta lo que hay:

| cosa | cómo se detecta |
|---|---|
| JRE | `~/.lunarclient/jre/*/*/bin/javaw.exe`, prioridad a `jre17` |
| natives | primer subdirectorio de `multiver/natives/` con `lwjgl64.dll` |
| mappings | `lunar-platform-mappings-v1_8.jar` / `-v1_7.jar` |
| artefactos comunes | `COMMON_JARS` en `lunar_offline.py`, filtrados por existencia |
| versión disponible | tiene que existir el mapping **y** el `OptiFine_vX_Y.jar` |

Si Lunar cambia de layout, el botón **Diagnóstico** y `--selftest` enumeran
exactamente qué falta por versión.

---

## 10. Cómo se verificó

Cada pieza se probó con un arranque real y lectura del log, no por inspección:

| prueba | criterio de éxito |
|---|---|
| classpath correcto | `Loaded 19039 baked classes` (vs 4 389) |
| Bridge activo | `[Bridge] Setting Bridge Implementation to com.moonsworth.lunar.legacy...` |
| sesión offline | `[LC] Setting user: TestOffline` |
| join real | `TestOffline joined the game` en el log del servidor |
| puerto no estándar | `Connecting to 127.0.0.1, 25566` |
| 1.7.10 | 56 líneas `[LC]`/`[Bridge]`, cero excepciones |
| restauración | `accounts.json` idéntico byte a byte |
| portable | `.exe` extraído del zip, `--selftest` OK |

---

## 11. Por qué el swap es aditivo y no sustitutivo

La primera versión escribía `accounts.json` **entero**, con la cuenta offline
como única entrada. La copia del original quedaba en
`accounts.original.json` y se restauraba al cerrar. En el papel era limpio.

El fallo aparece cuando el launcher oficial se abre **mientras el swap está
vivo**:

1. La app ya escribió un `accounts.json` con una sola cuenta, `legacy: true`,
   `accessToken: "0"`.
2. El launcher oficial lee ese archivo, no encuentra ninguna sesión de Microsoft
   válida y **cierra la sesión**.
3. Le pide al usuario iniciar sesión de nuevo. El archivo nuevo que escribe
   contiene la cuenta offline que había más la recién logueada.
4. La restauración de la app, que copiaba el original encima, habría borrado ese
   login nuevo.

Ese es exactamente el síntoma *"se cerró sesión de mi cuenta de Lunar porque
pasó"*.

### Diseño nuevo

**Swap aditivo** (`accounts_swap`):

- lee el archivo actual y **conserva todas** las cuentas;
- añade o reemplaza solo la entrada offline;
- `localId` = `sha1("LunarOffline:" + nick.lower())[:32]`, estable: dos arranques
  con el mismo nick no duplican nada;
- guarda copia completa del original la primera vez, y el `localId` de la cuenta
  offline dentro de `swap.pending` como JSON.

**Restauración fusionada** (`accounts_restore` + `merge_accounts`):

| situación | resultado |
|---|---|
| cuenta en el original, no en el actual | vuelve (el launcher oficial la borró) |
| cuenta en el actual, no en el original | se conserva (login nuevo) |
| cuenta en ambos | gana el token con `accessTokenExpiresAt` más nuevo |
| entrada offline temporal | se elimina |
| `activeAccountLocalId` | vuelve al original; si ya no existe, a una cuenta real |

Y un guardia: `official_launcher_running()` consulta `tasklist` por
`Lunar Client.exe` y **bloquea el arranque** si el launcher oficial está abierto.
Durante el juego, la barra de estado avisa si aparece.

Un `accounts.json` corrupto ya no revienta nada: `read_accounts()` devuelve una
estructura vacía y el swap trabaja sobre ella.

---

## 12. Limitaciones que no tienen arreglo

- **Servidores premium.** El login pasa por `session.minecraft.net`. Sin token
  de Mojang, el servidor responde `Invalid session`. No hay nada que un launcher
  offline pueda hacer: es el servidor quien valida.
- **Cosméticos y skin.** El backend de Lunar los asocia al `accessToken` real.
  Con `accessToken: "0"` no hay perfil que consultar.
- **Actualizaciones grandes de Lunar.** Si cambian el formato de artefactos o el
  nombre de la clase principal, hay que volver a leer el `main.js`. El
  `Diagnóstico` dice qué pieza falta.
