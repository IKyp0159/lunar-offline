# Lunar Offline Launcher

Lanza **Lunar Client completo** — mods, FPS boost, keystrokes, mod menu — con un
nick **no premium**, sin cuenta de Microsoft y compatible con servidores
`online-mode=false`.

![estado](https://img.shields.io/badge/estado-probado-brightgreen)
![plataforma](https://img.shields.io/badge/plataforma-Windows%2010%2F11-blue)
![python](https://img.shields.io/badge/python-3.10%2B-blue)

---

## Qué es

Un lanzador que arranca el core **Genesis** de Lunar Client pasándole una sesión
offline, replicando exactamente la línea de comandos que usa el launcher oficial.

No es un cliente alternativo. No es un mod. Es el **mismo** `lunar.jar` que ya
tienes instalado, arrancado con `--accessToken 0 --userType legacy`.

Lo único que cambia respecto al launcher oficial es la autenticación.

| | launcher oficial | este launcher |
|---|---|---|
| ventana de Lunar | sí | no |
| el juego es Lunar | sí | **sí, el mismo** |
| mods / FPS / keystrokes / mod menu | sí | **sí** |
| cosméticos y skin premium | sí | no |
| requiere cuenta Microsoft | sí | no |

---

## Estado

Probado de punta a punta en Windows 11 con Lunar Client `3.7.36`:

| prueba | resultado |
|---|---|
| arranque 1.8.9 | 19 039 clases horneadas, `[LC]` y `[Bridge]` activos |
| arranque 1.7.10 | 56 líneas `[LC]`/`[Bridge]`, cero excepciones |
| join a servidor 1.8.9 `online-mode=false` | `joined the game` |
| puerto no estándar | conecta con `host:25566` |
| `accounts.json` tras cerrar | restaurado byte por byte |
| `.exe` portable extraído de zip | diagnóstico OK |

---

## Requisitos

**En la máquina donde se juega:**

1. Windows 10 u 11, 64 bits.
2. Lunar Client instalado (https://lunarclient.com).
3. Haber abierto el launcher oficial **una vez** y dejado que descargue la
   versión que vas a usar (`1.8.9` o `1.7.10`). Eso baja:
   - `~/.lunarclient/offline/multiver/` → jars y natives
   - `~/.lunarclient/jre/` → el JRE 17 que usa Lunar
4. Un servidor de Minecraft con `online-mode=false`.

**Para ejecutar el `.exe`:** nada más. No necesita Python.

**Para ejecutar desde código:** Python 3.10+ con Tkinter (viene de serie en
Windows).

---

## Instalación

### Opción A — portable (recomendado)

1. Descarga `lunar-offline-portable.zip` de la sección **Releases**.
2. Descomprime donde quieras.
3. Doble clic en `Lunar Offline.exe`.

### Opción B — desde código

```powershell
git clone https://github.com/<usuario>/lunar-offline.git
cd lunar-offline
.\Lunar Offline.bat
```

o directamente:

```powershell
python lunar_offline.py
```

### Opción C — compilar tu propio `.exe`

```powershell
pip install pyinstaller
.\build.ps1
```

El resultado queda en `dist\Lunar Offline.exe`.

---

## Uso

1. **Nick** — 1 a 16 caracteres: `A-Z`, `0-9`, `_`.
2. **Versión** — la misma que use el servidor.
3. **Servidor** — `minecraft.midominio.net` o `minecraft.midominio.net:25565`.
   Vacío = menú principal, te conectas a mano.
4. **RAM** — 3072 MB está bien para 1.8.9.
5. **Resolución** / pantalla completa.
6. **JUGAR CON LUNAR**.

Dentro del juego, **Shift derecho** abre el mod menu de Lunar.

### La regla de oro

**No abras el launcher oficial mientras el juego offline esté corriendo.**
El launcher reescribe `accounts.json` y la restauración automática ya no puede
dejar el archivo como estaba. Cierra el juego primero.

---

## Cómo funciona

```
┌──────────────────────────┐
│ lunar_offline.py         │
│  1. detecta JRE 17       │
│  2. detecta natives      │
│  3. escribe accounts.json│──┐
│  4. arma el comando      │  │
│  5. lanza javaw          │  │
│  6. espera al proceso    │  │
│  7. restaura accounts    │  │
└──────────────────────────┘  │
                              ▼
                 ~/.lunarclient/settings/game/accounts.json
                              │
                              ▼
   javaw -cp <todos los jars> com.moonsworth.lunar.genesis.Genesis
         --ichorClassPath <todos los jars>
         --webosrDir <natives>
         --version 1.8
         --username <nick> --accessToken 0 --userType legacy
         --uuid <uuid offline> --server host:puerto
```

### Los cinco detalles que hacen que arranque Lunar y no vanilla

Estos son los que costaron: sacados del `main.js` del launcher oficial
(`app.asar`), no de la documentación.

| pieza | valor correcto | qué pasa si está mal |
|---|---|---|
| `-cp` de la JVM | **todos** los artefactos `CLASS_PATH`, rutas absolutas unidas con `;` | ichor solo ve `genesis.jar`, hornea 4 389 clases vanilla |
| `--ichorClassPath` | los mismos nombres separados por `,` | el pipeline no registra módulos |
| `--webosrDir` | el directorio `natives/<hash>` pelado | `Did not find Ultralight Resource Directory` |
| `--version` | subversion: `1.8` / `1.7` | no encuentra el mapping correspondiente |
| `--server` | `host:puerto` en un solo argumento | se ignora el puerto y conecta a 25565 |
| `--texturesDir` / `--uiDir` / `--jitDir` | rutas de `~/.lunarclient` | cosméticos e interfaz caen a defaults |

Detalle completo del proceso de ingeniería inversa en
[`docs/TECNICO.md`](docs/TECNICO.md).

### El nick

Lunar **ignora** `--username` para la sesión y lee `accounts.json`. Por eso la app:

1. guarda copia en `%APPDATA%\LunarOffline\accounts.original.json`;
2. escribe una cuenta `legacy` con tu nick y su UUID offline;
3. lanza el juego;
4. al morir el proceso, restaura el archivo original byte por byte.

Si la app muere de golpe, al abrirla de nuevo restaura sola. También hay botón
**Restaurar cuenta**.

El UUID se genera como `OfflinePlayer:<nick>` → MD5 → v3, el estándar de
cualquier servidor offline. Mismo nick = mismo UUID = mismo inventario.

---

## Estructura del repositorio

| archivo | qué es |
|---|---|
| `lunar_offline.py` | la aplicación completa (GUI Tkinter + armado del comando + swap de cuentas) |
| `Lunar Offline.bat` | lanzador sin ventana de consola (`pythonw`) |
| `build.ps1` | compila el `.exe` portable con PyInstaller |
| `README.md` | este documento |
| `docs/TECNICO.md` | ingeniería inversa: cómo se obtuvo cada argumento |
| `LEEME.txt` | instrucciones cortas para la carpeta portable |
| `.gitignore` | excluye `dist/`, `build/`, caches, el zip y artefactos |
| `LICENSE` | MIT |

### Dentro de `lunar_offline.py`

| bloque | función |
|---|---|
| `find_java()` | busca `javaw.exe` en `~/.lunarclient/jre`, prefiere JRE 17 |
| `find_natives()` | primer directorio de natives con `lwjgl64.dll` |
| `offline_uuid()` | UUID offline con prefijo `OfflinePlayer:` |
| `classpath_jars()` | arma la lista de jars según versión |
| `accounts_swap()` / `accounts_restore()` | cambio y restauración de cuenta |
| `build_command()` | construye el argv completo de Genesis |
| `Launcher` | GUI, validación, lanzamiento y vigilancia del proceso |
| `selftest()` | informe de soporte en `%APPDATA%\LunarOffline\selftest.txt` |

---

## Diagnóstico

Botón **Diagnóstico** dentro de la app, o sin abrir ventana:

```powershell
& ".\Lunar Offline.exe" --selftest
```

Escribe `%APPDATA%\LunarOffline\selftest.txt` con:

- rutas de Lunar, multiver y Minecraft y si existen
- Java detectado y si el archivo es válido
- directorio de natives
- estado de `accounts.json` y de un swap pendiente
- versiones disponibles
- si el comando se arma bien para cada versión

---

## Datos que guarda

Todo en `%APPDATA%\LunarOffline\`:

| archivo | contenido |
|---|---|
| `config.json` | nick, versión, RAM, resolución, java |
| `accounts.original.json` | copia temporal mientras juegas |
| `swap.pending` | marca de swap sin restaurar (recuperación de crash) |
| `logs\run-*.log` | salida completa de cada arranque |
| `logs\cmd-*.txt` | línea de comandos exacta de cada arranque |
| `selftest.txt` | informe de diagnóstico |

Nada de esto se sube a ningún sitio.

---

## Limitaciones

- **Servidores premium** rechazan el login con `Invalid session`. No hay token
  de Mojang que validar. No es un fallo del launcher.
- **Cosméticos y skin premium** no funcionan: van atados a la cuenta Microsoft.
- Si Lunar Client cambia el formato de artefactos en una actualización grande,
  el botón `Diagnóstico` señala exactamente qué falta.
- Solo Windows. El código es portable en concepto, pero las rutas y los nativos
  son de Windows.

---

## Licencia

MIT — ver [`LICENSE`](LICENSE).

Este proyecto no distribuye código de Lunar Client ni de Mojang. Usa los
archivos que el launcher oficial ya descargó en tu equipo.
