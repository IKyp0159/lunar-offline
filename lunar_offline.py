#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Lunar Offline Launcher
Arranca Lunar Client completo (mods, FPS, keystrokes) con un nick no premium.

Replica exacta de lo que hace el launcher oficial:
  - classpath JVM = TODOS los artefactos CLASS_PATH del fileCacheRoot
  - Genesis recibe --ichorClassPath con los mismos nombres
  - --webosrDir apunta al directorio de natives (ahi viven los recursos Ultralight)
  - --version usa el SUBVERSION ("1.8"), no el nombre largo ("1.8.9")
  - el servidor se pasa como host:port en un solo argumento

El nick lo decide Lunar leyendo accounts.json, asi que la app cambia esa cuenta
por una offline, lanza el juego y restaura el archivo original al cerrar.
"""

from __future__ import annotations

import hashlib
import json
import math
import os
import random
import re
import shutil
import subprocess
import sys
import time
import uuid
from pathlib import Path

import tkinter as tk
from tkinter import filedialog
from tkinter import font as tkfont

APP_DIR = Path(os.environ.get("APPDATA", Path.home())) / "LunarOffline"
APP_DIR.mkdir(parents=True, exist_ok=True)
CONFIG = APP_DIR / "config.json"
LOGS = APP_DIR / "logs"
LOGS.mkdir(parents=True, exist_ok=True)

ORIGINAL_ACCOUNTS = APP_DIR / "accounts.original.json"
SWAP_MARKER = APP_DIR / "swap.pending"

LUNAR_ROOT = Path(os.environ["USERPROFILE"]) / ".lunarclient"
MULTIVER = LUNAR_ROOT / "offline" / "multiver"
MC_DIR = Path(os.environ.get("APPDATA", Path.home())) / ".minecraft"
ACCOUNTS = LUNAR_ROOT / "settings" / "game" / "accounts.json"

VERSIONS = {
    "1.8.9": {
        "subversion": "1.8",
        "mapping": "lunar-platform-mappings-v1_8.jar",
        "external": "OptiFine_v1_8.jar",
        "asset_index": "1.8",
    },
    "1.7.10": {
        "subversion": "1.7",
        "mapping": "lunar-platform-mappings-v1_7.jar",
        "external": "OptiFine_v1_7.jar",
        "asset_index": "1.7.10",
    },
}

COMMON_JARS = [
    "lunar-platform-mappings-v26_2.jar",
    "optifine-0.1.0-SNAPSHOT-all.jar",
    "common-0.1.0-SNAPSHOT-all-nomappings.jar",
    "legacy-0.1.0-SNAPSHOT-all-nomappings.jar",
    "modern-0.1.0-SNAPSHOT-all-nomappings.jar",
    "lunar-lang.jar",
    "genesis-0.1.0-SNAPSHOT-all.jar",
    "lunar.jar",
]

MAIN_CLASS = "com.moonsworth.lunar.genesis.Genesis"

JAVA_FLAGS = [
    "-XX:+UnlockExperimentalVMOptions",
    "-XX:+UseG1GC",
    "-XX:G1NewSizePercent=20",
    "-XX:G1ReservePercent=20",
    "-XX:MaxGCPauseMillis=50",
    "-XX:G1HeapRegionSize=32M",
    "-XX:+DisableExplicitGC",
    "-XX:+ParallelRefProcEnabled",
    "-XX:+UseStringDeduplication",
    "-XX:+OptimizeStringConcat",
]

NICK_RE = re.compile(r"^[A-Za-z0-9_]{1,16}$")


# --------------------------------------------------------------------------- #
# discovery
# --------------------------------------------------------------------------- #

def find_java() -> str | None:
    jre_root = LUNAR_ROOT / "jre"
    ranked: list[tuple[int, str]] = []
    if jre_root.is_dir():
        for exe in jre_root.rglob("javaw.exe"):
            text = str(exe).lower()
            if "jre17" in text:
                ranked.append((0, str(exe)))
            elif "jre21" in text:
                ranked.append((1, str(exe)))
            elif "jre25" in text:
                ranked.append((2, str(exe)))
            else:
                ranked.append((3, str(exe)))
        if not ranked:
            for exe in jre_root.rglob("java.exe"):
                ranked.append((3, str(exe)))
    if ranked:
        ranked.sort()
        return ranked[0][1]

    java_home = os.environ.get("JAVA_HOME")
    if java_home:
        for name in ("javaw.exe", "java.exe"):
            cand = Path(java_home) / "bin" / name
            if cand.is_file():
                return str(cand)
    return None


def find_natives() -> str | None:
    base = MULTIVER / "natives"
    if not base.is_dir():
        return None
    for d in sorted(base.iterdir()):
        if d.is_dir() and (d / "lwjgl64.dll").is_file() and (d / "resources").is_dir():
            return str(d)
    for d in sorted(base.iterdir()):
        if d.is_dir() and (d / "lwjgl64.dll").is_file():
            return str(d)
    return None


def offline_uuid(nick: str) -> str:
    """UUID v3 estilo OfflinePlayer, el mismo que usa cualquier servidor offline."""
    digest = bytearray(hashlib.md5(("OfflinePlayer:" + nick).encode("utf-8")).digest())
    digest[6] = (digest[6] & 0x0F) | 0x30
    digest[8] = (digest[8] & 0x3F) | 0x80
    return str(uuid.UUID(bytes=bytes(digest)))


def classpath_jars(version: str) -> list[str]:
    spec = VERSIONS[version]
    names = [spec["mapping"], *COMMON_JARS]
    return [n for n in names if (MULTIVER / n).is_file()]


def available_versions() -> list[str]:
    out = []
    for name, spec in VERSIONS.items():
        if (MULTIVER / spec["mapping"]).is_file() and (MULTIVER / spec["external"]).is_file():
            out.append(name)
    return out


# --------------------------------------------------------------------------- #
# accounts.json swap — Lunar toma el nick de ahi
# --------------------------------------------------------------------------- #

def offline_local_id(nick: str) -> str:
    """localId estable por nick: dos arranques con el mismo nick no duplican entrada."""
    return hashlib.sha1(("LunarOffline:" + nick.lower()).encode("utf-8")).hexdigest()[:32]


def read_accounts(path: Path) -> dict:
    """Lee accounts.json tolerando basura. Devuelve estructura vacia si no se puede."""
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return {"activeAccountLocalId": "", "accounts": {}}
    if not isinstance(data, dict):
        return {"activeAccountLocalId": "", "accounts": {}}
    if not isinstance(data.get("accounts"), dict):
        data["accounts"] = {}
    return data


def _is_real_session(acc: dict) -> bool:
    """Cuenta con token de Microsoft de verdad (no la offline que inyecta la app)."""
    token = str(acc.get("accessToken") or "")
    return bool(token) and token != "0" and not acc.get("legacy")


def merge_accounts(base: dict, incoming: dict, drop_id: str = "") -> dict:
    """
    Une dos accounts.json sin perder nada.

    - Toda cuenta de `incoming` que no este en `base` se conserva: es un login
      nuevo hecho mientras el swap estaba vivo (launcher oficial, otra cuenta).
    - Si la cuenta esta en ambos, gana la que tenga sesion real mas nueva, para
      no pisar un token recien emitido con uno viejo.
    - `drop_id` se elimina del resultado: es la cuenta offline temporal.
    """
    result = {k: v for k, v in (base.get("accounts") or {}).items() if k != drop_id}
    for local_id, acc in (incoming.get("accounts") or {}).items():
        if local_id == drop_id or not isinstance(acc, dict):
            continue
        prev = result.get(local_id)
        if prev is None:
            result[local_id] = acc
            continue
        if _is_real_session(acc) and not _is_real_session(prev):
            result[local_id] = acc
            continue
        if _is_real_session(acc) and _is_real_session(prev):
            if str(acc.get("accessTokenExpiresAt", "")) > str(prev.get("accessTokenExpiresAt", "")):
                result[local_id] = acc
    active = base.get("activeAccountLocalId") or incoming.get("activeAccountLocalId") or ""
    if active not in result:
        real = [k for k, v in result.items() if _is_real_session(v)]
        active = real[0] if real else (next(iter(result), ""))
    return {"activeAccountLocalId": active, "accounts": result}


def official_launcher_running() -> bool:
    """True si el launcher oficial de Lunar esta abierto (se come accounts.json)."""
    if os.name != "nt":
        return False
    try:
        res = subprocess.run(
            ["tasklist", "/FI", "IMAGENAME eq Lunar Client.exe", "/NH"],
            capture_output=True, timeout=8,
            creationflags=subprocess.CREATE_NO_WINDOW,  # type: ignore[attr-defined]
        )
        # tasklist escribe en la codepage OEM: decodificar como UTF-8 revienta
        # con cualquier acento y deja stdout en None.
        out = (res.stdout or b"").decode("latin-1", errors="replace")
    except Exception:
        return False
    return "Lunar Client.exe" in out


def accounts_restore() -> bool:
    """Devuelve la cuenta premium y conserva cualquier login hecho mientras jugabas."""
    offline_id = ""
    try:
        offline_id = json.loads(SWAP_MARKER.read_text(encoding="utf-8")).get("offline_id", "")
    except Exception:
        pass

    if not ORIGINAL_ACCOUNTS.is_file():
        SWAP_MARKER.unlink(missing_ok=True)
        return False

    original = read_accounts(ORIGINAL_ACCOUNTS)
    current = read_accounts(ACCOUNTS)

    merged = merge_accounts(original, current, drop_id=offline_id)

    # Limpieza: entradas offline muertas (legacy sin token) que no estaban en el
    # original. Son basura de arranques anteriores de la app; las cuentas offline
    # que el usuario ya tenia se conservan porque estan en original.
    original_ids = set((original.get("accounts") or {}))
    merged["accounts"] = {
        k: v for k, v in merged["accounts"].items()
        if k in original_ids or not (v.get("legacy") and str(v.get("accessToken")) == "0")
    }

    ACCOUNTS.parent.mkdir(parents=True, exist_ok=True)
    ACCOUNTS.write_text(json.dumps(merged, indent=2), encoding="utf-8")

    ORIGINAL_ACCOUNTS.unlink(missing_ok=True)
    SWAP_MARKER.unlink(missing_ok=True)
    return True


def accounts_swap(nick: str) -> None:
    """
    Anade la cuenta offline y la marca como activa, SIN borrar las que ya hay.

    Antes se reemplazaba el archivo entero: si el launcher oficial se abria con
    el swap vivo, encontraba una sola cuenta sin token valido y cerraba sesion.
    Ahora las cuentas premium se quedan ahi y el swap solo cambia la activa.
    """
    ACCOUNTS.parent.mkdir(parents=True, exist_ok=True)
    current = read_accounts(ACCOUNTS)

    # Copia del original solo la primera vez: si ya hay marca, la copia buena
    # ya existe y reescribirla con la cuenta offline perderia las premium.
    if not SWAP_MARKER.exists():
        if ACCOUNTS.is_file():
            shutil.copy2(ACCOUNTS, ORIGINAL_ACCOUNTS)
        (APP_DIR / "accounts.original.active").write_text(
            str(current.get("activeAccountLocalId", "")), encoding="utf-8")

    local_id = offline_local_id(nick)
    profile_id = offline_uuid(nick)
    merged = dict(current.get("accounts") or {})
    merged[local_id] = {
        "accessToken": "0",
        "accessTokenExpiresAt": "1970-01-01T00:00:00Z",
        "eligibleForMigration": False,
        "hasMultipleProfiles": False,
        "legacy": True,
        "persistent": False,
        "localId": local_id,
        "refreshToken": "",
        "minecraftProfile": {"id": profile_id, "name": nick},
        "remoteId": "",
        "type": "Xbox",
        "username": nick,
    }

    SWAP_MARKER.write_text(
        json.dumps({"nick": nick, "offline_id": local_id, "since": int(time.time())}),
        encoding="utf-8",
    )
    ACCOUNTS.write_text(
        json.dumps({"activeAccountLocalId": local_id, "accounts": merged}, indent=2),
        encoding="utf-8",
    )


# --------------------------------------------------------------------------- #
# command
# --------------------------------------------------------------------------- #

def build_command(nick: str, version: str, ram_mb: int, server: str,
                  width: int, height: int, fullscreen: bool, java: str) -> list[str]:
    spec = VERSIONS[version]
    natives = find_natives()
    if natives is None:
        raise RuntimeError("No encontre el directorio natives de Lunar (lwjgl64.dll).")

    names = classpath_jars(version)
    if not names:
        raise RuntimeError(f"No hay artefactos para {version} en {MULTIVER}.")

    jvm_cp = ";".join(str(MULTIVER / n) for n in names)

    cmd = [
        java,
        f"-Xms{ram_mb}M",
        f"-Xmx{ram_mb}M",
        *JAVA_FLAGS,
        "-Dlog4j2.formatMsgNoLookups=true",
        "-Dlunar.webosr.url=file:index.html",
        f"-Djava.library.path={natives}",
        "-Dichor.usingIsolatedProfiles=true",
        f"-Dichor.fabric.localModPath={LUNAR_ROOT / 'profiles' / '1.8' / 'mods'}",
        "-XX:+DisableAttachMechanism",
    ]

    log_cfg = LUNAR_ROOT / "profiles" / "1.8" / "logs" / "config.xml"
    if log_cfg.is_file():
        cmd.append(f"-Dlog4j.configurationFile={log_cfg}")
    cmd.append(f"-Dichor.logsFile={LUNAR_ROOT / 'profiles' / '1.8' / 'logs' / 'ichor-boot.log'}")

    cmd += [
        "-cp", jvm_cp,
        MAIN_CLASS,
        "--workingDirectory", ".",
        "--classpathDir", str(MULTIVER),
        "--ichorExternalFiles", spec["external"],
        "--ichorClassPath", ",".join(names),
        "--webosrDir", natives,
        "--texturesDir", str(LUNAR_ROOT / "textures"),
        "--jitDir", str(LUNAR_ROOT / "jit"),
        "--uiDir", str(LUNAR_ROOT / "ui"),
        "--version", spec["subversion"],
        "--gameDir", str(MC_DIR),
        "--assetsDir", str(MC_DIR / "assets"),
        "--assetIndex", spec["asset_index"],
        "--uuid", offline_uuid(nick),
        "--xuid", "not specified",
        "--accessToken", "0",
        "--userProperties", "{}",
        "--username", nick,
        "--userType", "legacy",
        "--versionType", "release",
        "--installationId", "not specified",
        "--width", str(width),
        "--height", str(height),
        "--ipcPort", "0",
    ]
    if fullscreen:
        cmd.append("--fullscreen")

    host = server.strip()
    if host:
        if ":" in host and not host.startswith("["):
            h, _, p = host.partition(":")
            host = f"{h}:{p}" if p else f"{h}:25565"
        elif not host.startswith("[") and host.count(":") == 0:
            host = f"{host}:25565"
        cmd += ["--server", host]

    return cmd


# --------------------------------------------------------------------------- #
# gui — toolkit propio sobre Canvas, monocromo (negro / blanco) y animado
# --------------------------------------------------------------------------- #

# Paleta: negro puro y blanco, con grises solo para dar profundidad.
# Los unicos colores fuera de la escala son los semanticos (error y aviso).
BG        = "#050505"   # ventana
SURFACE   = "#0D0D0D"   # tarjetas
FIELD     = "#141414"   # inputs, selectores
FIELD_HI  = "#1E1E1E"   # hover
BORDER    = "#262626"
BORDER_HI = "#424242"
WHITE     = "#FFFFFF"
INK       = "#050505"   # texto sobre superficies blancas
TEXT      = "#FFFFFF"
TEXT_DIM  = "#CFCFCF"
MUTED     = "#9A9A9A"   # labels (contraste AA sobre SURFACE)
FAINT     = "#808080"   # textos de ayuda
AMBER     = "#F0B35E"
RED       = "#FF6B6B"

APP_VERSION = "1.4.0"
WIN_W = 600
SKY_H = 72

F_TITLE = ("Segoe UI Semibold", -24)
F_SUB   = ("Segoe UI", -12)
F_CAPS  = ("Segoe UI Semibold", -10)
F_INPUT = ("Segoe UI", -13)
F_BTN   = ("Segoe UI Semibold", -15)
F_SMALL = ("Segoe UI", -11)
F_LINK  = ("Segoe UI", -12)
F_MONO  = ("Consolas", -11)


def lerp_color(c1: str, c2: str, t: float) -> str:
    """Mezcla lineal entre dos colores #rrggbb."""
    t = min(1.0, max(0.0, t))
    a = [int(c1[i:i + 2], 16) for i in (1, 3, 5)]
    b = [int(c2[i:i + 2], 16) for i in (1, 3, 5)]
    return "#%02x%02x%02x" % tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(3))


def round_rect(cv: tk.Canvas, x1: float, y1: float, x2: float, y2: float,
               r: float, **kw) -> int:
    """Rectangulo redondeado sobre un Canvas (poligono suavizado)."""
    r = max(0.0, min(r, (x2 - x1) / 2, (y2 - y1) / 2))
    pts = [
        x1 + r, y1, x2 - r, y1, x2, y1, x2, y1 + r,
        x2, y2 - r, x2, y2, x2 - r, y2, x1 + r, y2,
        x1, y2, x1, y2 - r, x1, y1 + r, x1, y1,
    ]
    return cv.create_polygon(pts, smooth=True, splinesteps=48, **kw)


def strip_round(cv: tk.Canvas, x1: float, y1: float, x2: float, y2: float,
                r: float, color_fn, step: int = 2) -> None:
    """Rectangulo redondeado pintado en columnas; `color_fn(0..1)` da el color de cada una."""
    w = x2 - x1
    n = int(w // step)
    if n < 2:
        return
    for i in range(n):
        x = x1 + i * step + step / 2
        d = min(x - x1, x2 - x)
        dy = 0.0
        if d < r:                              # recorte de la esquina
            dy = r - math.sqrt(max(0.0, r * r - (r - d) ** 2))
        cv.create_line(x, y1 + dy, x, y2 - dy, fill=color_fn((x - x1) / w), width=step)


class Tween:
    """Valor que se acerca suavemente a un objetivo (ease-out). Llama a `on_change` en cada paso."""

    def __init__(self, widget, on_change, value: float = 0.0, rate: float = 0.22,
                 done=None) -> None:
        self.w, self.cb, self.done = widget, on_change, done
        self.value = self.target = float(value)
        self.rate = rate
        self._job = None

    def go(self, target: float) -> None:
        self.target = float(target)
        if self._job is None:
            self._step()

    def jump(self, value: float) -> None:
        self.value = self.target = float(value)
        try:
            self.cb()
        except tk.TclError:
            pass

    def _step(self) -> None:
        self._job = None
        d = self.target - self.value
        finished = abs(d) < 0.004
        self.value = self.target if finished else self.value + d * self.rate
        try:
            self.cb()
            if finished:
                if self.done:
                    self.done()
                return
            self._job = self.w.after(16, self._step)
        except tk.TclError:                    # el widget ya no existe
            self._job = None


class RoundEntry(tk.Canvas):
    """Campo de texto plano con borde animado al enfocar, estado de error y sacudida."""

    def __init__(self, master, textvariable: tk.Variable, height: int = 44,
                 font=F_INPUT, pad: int = 15, radius: int = 10,
                 bg: str = SURFACE, counter: int = 0) -> None:
        super().__init__(master, width=10, height=height, bg=bg,
                         highlightthickness=0, bd=0)
        self._radius = radius
        self._pad = pad
        self._counter = counter
        self._var = textvariable
        self._error = False
        self._focus_tw = Tween(self, self._draw, rate=0.28)
        self.entry = tk.Entry(
            self, textvariable=textvariable, bd=0, relief="flat",
            bg=FIELD, fg=TEXT, insertbackground=WHITE, font=font,
            highlightthickness=0, selectbackground=WHITE, selectforeground=INK,
        )
        self._win = self.create_window(pad, height // 2, anchor="w", window=self.entry)
        self.bind("<Configure>", lambda _e: self._draw())
        self.entry.bind("<FocusIn>", lambda _e: self._focus_tw.go(1.0))
        self.entry.bind("<FocusOut>", lambda _e: self._focus_tw.go(0.0))
        self.entry.bind("<Button-1>", lambda _e: self.entry.focus_set())
        self.bind("<Button-1>", lambda _e: self.entry.focus_set())
        if counter:
            textvariable.trace_add("write", lambda *_a: self._draw())

    def set_error(self, on: bool) -> None:
        if on != self._error:
            self._error = on
            self._draw()

    def shake(self) -> None:
        """Sacudida horizontal corta: avisa que el valor no sirve."""
        offsets = [9, -9, 7, -7, 4, -4, 0]

        def step(i: int = 0) -> None:
            try:
                d = offsets[i]
                self.pack_configure(padx=(max(d, 0), max(-d, 0)))
                if i + 1 < len(offsets):
                    self.after(26, lambda: step(i + 1))
            except tk.TclError:
                pass
        step()

    def _draw(self) -> None:
        w, h = self.winfo_width(), self.winfo_height()
        if w < 8 or h < 8:
            return
        self.delete("shape")
        self.delete("count")
        f = self._focus_tw.value
        outline = RED if self._error else lerp_color(BORDER, WHITE, f)
        round_rect(self, 0.5, 0.5, w - 1, h - 1, self._radius,
                   fill=FIELD, outline=outline, width=1 + 0.6 * f, tags="shape")
        self.tag_lower("shape")
        right = self._pad
        if self._counter:
            n = len(self._var.get())
            bad = n > self._counter or n == 0
            self.create_text(w - self._pad, h / 2, anchor="e", text=f"{n}/{self._counter}",
                             fill=RED if bad else FAINT, font=F_SMALL, tags="count")
            right = self._pad + 42
        self.itemconfigure(self._win, width=max(30, w - self._pad - right), height=h - 14)

    def focus(self) -> None:  # type: ignore[override]
        self.entry.focus_set()


class RoundButton(tk.Canvas):
    """
    Boton redondeado.
      style="primary": blanco con texto negro; al pasar el raton lo cruza un destello.
      style="ghost":   oscuro con borde que se ilumina en hover.
    Deshabilitado: borde fino con un trazo de carga recorriendo la base.
    """

    def __init__(self, master, text: str, command, height: int = 52,
                 font=F_BTN, radius: int = 12, width: int = 0, bg: str = BG,
                 style: str = "primary", icon: str | None = None) -> None:
        super().__init__(master, height=height, width=width or 120,
                         bg=bg, highlightthickness=0, bd=0, cursor="hand2")
        self._text, self._cmd = text, command
        self._font, self._radius = font, radius
        self._style, self._icon = style, icon
        self._over = False
        self._down = False
        self._disabled = False
        self._measure = None
        self._hover_tw = Tween(self, self._draw, rate=0.25)
        self._sheen = -1.0
        self._sheen_job = None
        self._busy = 0.0
        self._busy_job = None
        self.bind("<Configure>", lambda _e: self._draw())
        self.bind("<Enter>", lambda _e: self._set(True))
        self.bind("<Leave>", lambda _e: self._set(False))
        self.bind("<ButtonPress-1>", lambda _e: self._press(True))
        self.bind("<ButtonRelease-1>", lambda _e: self._press(False))

    # ---- interaccion

    def _set(self, over: bool) -> None:
        self._over = over
        self._hover_tw.go(1.0 if over and not self._disabled else 0.0)
        if over and not self._disabled and self._style == "primary":
            self._sheen = 0.0
            if self._sheen_job is None:
                self._sheen_step()

    def _press(self, down: bool) -> None:
        if self._disabled:
            return
        self._down = down
        self._draw()
        if not down and self._over:
            self._cmd()

    def set_text(self, text: str) -> None:
        self._text = text
        self._draw()

    def set_state(self, text: str, disabled: bool) -> None:
        self._text = text
        self._disabled = disabled
        self._down = False
        self.configure(cursor="arrow" if disabled else "hand2")
        if disabled:
            self._hover_tw.jump(0.0)
            if self._busy_job is None:
                self._busy_step()
        else:
            self._busy = 0.0
        self._draw()

    # ---- animacion

    def _sheen_step(self) -> None:
        self._sheen_job = None
        self._sheen += 0.055
        try:
            if self._sheen >= 1.25:
                self._sheen = -1.0
                self._draw()
                return
            self._draw()
            self._sheen_job = self.after(20, self._sheen_step)
        except tk.TclError:
            self._sheen_job = None

    def _busy_step(self) -> None:
        self._busy_job = None
        if not self._disabled:
            return
        self._busy = (self._busy + 0.016) % 1.0
        try:
            self._draw()
            self._busy_job = self.after(30, self._busy_step)
        except tk.TclError:
            self._busy_job = None

    # ---- dibujo

    def _text_width(self) -> int:
        if self._measure is None:
            self._measure = tkfont.Font(font=self._font).measure
        return self._measure(self._text)

    def _draw(self) -> None:
        w, h = self.winfo_width(), self.winfo_height()
        if w < 8 or h < 8:
            return
        self.delete("all")
        cy = h / 2
        hv = self._hover_tw.value

        if self._disabled:
            round_rect(self, 0.5, 0.5, w - 1, h - 1, self._radius,
                       fill=SURFACE, outline=BORDER_HI)
            # trazo de carga recorriendo la base
            r = self._radius
            x0, x1 = r, w - r
            seg = (x1 - x0) * 0.28
            a = x0 + ((x1 - x0) + seg) * self._busy - seg
            lo, hi = max(x0, a), min(x1, a + seg)
            if hi > lo:
                self.create_line(lo, h - 4, hi, h - 4, fill=WHITE, width=2, capstyle="round")
            tw = self._text_width()
            x = (w - (tw + 18)) / 2
            pulse = (math.sin(self._busy * 2 * math.pi) + 1) / 2
            self.create_oval(x, cy - 3, x + 6, cy + 3, fill=lerp_color(FAINT, WHITE, pulse),
                             outline="")
            self.create_text(x + 18, cy, text=self._text, fill=TEXT_DIM, font=self._font,
                             anchor="w")
            return

        if self._style == "primary":
            fg = INK
            if self._down:
                round_rect(self, 0.5, 0.5, w - 1, h - 1, self._radius,
                           fill="#D6D6D6", outline="#D6D6D6")
            elif 0.0 <= self._sheen <= 1.25:
                center = -0.2 + self._sheen * 1.2

                def col(f: float) -> str:
                    k = max(0.0, 1 - abs(f - center) / 0.16)
                    return lerp_color(WHITE, "#BEBEBE", k * k * 0.9)
                strip_round(self, 0, 0, w, h, self._radius, col)
            else:
                round_rect(self, 0.5, 0.5, w - 1, h - 1, self._radius,
                           fill=WHITE, outline=WHITE)
        else:
            fg = lerp_color(TEXT_DIM, WHITE, hv)
            round_rect(self, 0.5, 0.5, w - 1, h - 1, self._radius,
                       fill=lerp_color(FIELD, FIELD_HI, hv),
                       outline=lerp_color(BORDER_HI, WHITE, hv))

        tw = self._text_width()
        icon_w = 26 if self._icon else 0
        x0 = (w - (tw + icon_w)) / 2 + (3 * hv if self._icon else 0)
        if self._icon == "play":
            self.create_polygon(x0, cy - 7, x0, cy + 7, x0 + 12, cy, fill=fg, outline=fg)
        self.create_text(x0 + icon_w, cy, text=self._text, fill=fg,
                         font=self._font, anchor="w")


class Segmented(tk.Canvas):
    """Selector con una pastilla blanca que se desliza entre opciones."""

    def __init__(self, master, values: list[str], variable: tk.StringVar,
                 height: int = 38, on_change=None, bg: str = SURFACE) -> None:
        super().__init__(master, width=10, height=height, bg=bg,
                         highlightthickness=0, bd=0, cursor="hand2")
        self.values = list(values) or ["1.8.9"]
        self.var = variable
        self._on_change = on_change
        cur = variable.get()
        idx = self.values.index(cur) if cur in self.values else 0
        self._pos = Tween(self, self._draw, value=idx, rate=0.3)
        self.bind("<Configure>", lambda _e: self._draw())
        self.bind("<Button-1>", self._click)

    def _click(self, ev) -> None:
        n = len(self.values)
        w = max(1, self.winfo_width())
        idx = min(n - 1, max(0, int(ev.x / (w / n))))
        self.var.set(self.values[idx])
        self._pos.go(idx)
        if self._on_change:
            self._on_change()

    def _draw(self) -> None:
        w, h = self.winfo_width(), self.winfo_height()
        if w < 8 or h < 8:
            return
        self.delete("all")
        round_rect(self, 0.5, 0.5, w - 1, h - 1, 10, fill=FIELD, outline=BORDER)
        n = len(self.values)
        seg = w / n
        pos = self._pos.value
        px = pos * seg
        round_rect(self, px + 3, 3, px + seg - 3, h - 3, 8, fill=WHITE, outline=WHITE)
        for i, val in enumerate(self.values):
            near = max(0.0, 1 - abs(pos - i))
            self.create_text(i * seg + seg / 2, h / 2, text=val,
                             font=("Segoe UI Semibold", -12),
                             fill=lerp_color(MUTED, INK, near))


class Slider(tk.Canvas):
    """Slider con marcas por GB; la bolita crece al pasar el raton y el relleno la sigue."""

    def __init__(self, master, variable: tk.IntVar, lo: int, hi: int,
                 step: int = 256, on_change=None, height: int = 38,
                 bg: str = SURFACE) -> None:
        super().__init__(master, width=10, height=height, bg=bg,
                         highlightthickness=0, bd=0, cursor="hand2")
        self.var, self.lo, self.hi, self.step = variable, lo, hi, step
        self._on_change = on_change
        self._over = False
        self._drag = False
        self._shown = Tween(self, self._draw, value=self._frac(), rate=0.32)
        self._grow = Tween(self, self._draw, rate=0.3)
        self.bind("<Configure>", lambda _e: self._draw())
        self.bind("<Enter>", lambda _e: self._hover(True))
        self.bind("<Leave>", lambda _e: self._hover(False))
        self.bind("<ButtonPress-1>", self._press)
        self.bind("<ButtonRelease-1>", self._release)
        self.bind("<B1-Motion>", self._set_from_x)

    def _frac(self) -> float:
        f = (self.var.get() - self.lo) / max(1, (self.hi - self.lo))
        return min(1.0, max(0.0, f))

    def _hover(self, on: bool) -> None:
        self._over = on
        self._grow.go(1.0 if (on or self._drag) else 0.0)

    def _press(self, ev) -> None:
        self._drag = True
        self._grow.go(1.0)
        self._set_from_x(ev)

    def _release(self, _ev) -> None:
        self._drag = False
        self._grow.go(1.0 if self._over else 0.0)

    def _set_from_x(self, ev) -> None:
        w = self.winfo_width()
        frac = min(1.0, max(0.0, (ev.x - 10) / max(1, w - 20)))
        raw = self.lo + frac * (self.hi - self.lo)
        val = int(round(raw / self.step) * self.step)
        val = max(self.lo, min(self.hi, val))
        if val != self.var.get():
            self.var.set(val)
            self._shown.go(self._frac())
            if self._on_change:
                self._on_change()

    def _draw(self) -> None:
        w, h = self.winfo_width(), self.winfo_height()
        if w < 8 or h < 8:
            return
        self.delete("all")
        x0, x1, cy = 10, w - 10, h / 2 - 4
        frac = min(1.0, max(0.0, self._shown.value))
        kx = x0 + (x1 - x0) * frac
        round_rect(self, x0, cy - 2, x1, cy + 2, 2, fill=BORDER, outline=BORDER)
        if frac > 0.002:
            round_rect(self, x0, cy - 2, kx, cy + 2, 2, fill=WHITE, outline=WHITE)
        # marcas cada GB
        gb_steps = max(1, (self.hi - self.lo) // 1024)
        for i in range(gb_steps + 1):
            tx = x0 + (x1 - x0) * (i / gb_steps)
            self.create_line(tx, cy + 12, tx, cy + 16,
                             fill=TEXT_DIM if tx <= kx + 0.5 else BORDER_HI)
        r = 7 + 3 * self._grow.value
        self.create_oval(kx - r, cy - r, kx + r, cy + r, fill=WHITE, outline="")
        self.create_oval(kx - 2.5, cy - 2.5, kx + 2.5, cy + 2.5, fill=INK, outline="")


class Toggle(tk.Canvas):
    """Interruptor: la bolita se desliza y la pista se rellena de blanco."""

    def __init__(self, master, variable: tk.BooleanVar, on_change=None,
                 bg: str = SURFACE) -> None:
        super().__init__(master, width=44, height=24, bg=bg,
                         highlightthickness=0, bd=0, cursor="hand2")
        self.var = variable
        self._on_change = on_change
        self._p = Tween(self, self._draw, value=1.0 if variable.get() else 0.0, rate=0.3)
        self.bind("<Button-1>", self._click)
        self._draw()

    def _click(self, _ev=None) -> None:
        self.var.set(not self.var.get())
        self._p.go(1.0 if self.var.get() else 0.0)
        if self._on_change:
            self._on_change()

    def _draw(self) -> None:
        w, h = 44, 24
        self.delete("all")
        p = self._p.value
        round_rect(self, 1, 1, w - 1, h - 1, (h - 2) / 2,
                   fill=lerp_color(FIELD, WHITE, p),
                   outline=lerp_color(BORDER_HI, WHITE, p))
        cx = 13 + (w - 26) * p
        self.create_oval(cx - 8, 4, cx + 8, h - 4,
                         fill=lerp_color(MUTED, INK, p), outline="")


class PathField(tk.Canvas):
    """Ruta de solo lectura: se acorta por el medio con puntos suspensivos."""

    def __init__(self, master, variable: tk.StringVar, height: int = 38,
                 bg: str = SURFACE) -> None:
        super().__init__(master, width=10, height=height, bg=bg,
                         highlightthickness=0, bd=0)
        self._var = variable
        self._measure = None
        self.bind("<Configure>", lambda _e: self._draw())
        variable.trace_add("write", lambda *_a: self._draw())

    def _fit(self, text: str, maxw: float) -> str:
        if self._measure is None:
            self._measure = tkfont.Font(font=F_MONO).measure
        if self._measure(text) <= maxw:
            return text
        ell = "…"
        for k in range(len(text) - 1, 1, -1):
            head, tail = (k + 1) // 2, k // 2
            cand = text[:head] + ell + text[len(text) - tail:]
            if self._measure(cand) <= maxw:
                return cand
        return ell

    def _draw(self) -> None:
        w, h = self.winfo_width(), self.winfo_height()
        if w < 8 or h < 8:
            return
        self.delete("all")
        round_rect(self, 0.5, 0.5, w - 1, h - 1, 9, fill=FIELD, outline=BORDER)
        value = self._var.get().strip()
        text = value or "No se encontro Java: usa Examinar"
        self.create_text(13, h / 2, anchor="w", text=self._fit(text, w - 26),
                         fill=TEXT_DIM if value else AMBER, font=F_MONO)


class RoundCard(tk.Canvas):
    """Tarjeta redondeada que se ajusta sola a la altura de su contenido (`.body`)."""

    def __init__(self, master, pad: int = 16, bg: str = BG) -> None:
        super().__init__(master, width=10, height=40, bg=bg, highlightthickness=0, bd=0)
        self._pad = pad
        self.locked = False          # True mientras una animacion controla la altura
        self.body = tk.Frame(self, bg=SURFACE)
        self._win = self.create_window(pad, pad, anchor="nw", window=self.body)
        self.bind("<Configure>", self._on_conf)
        self.body.bind("<Configure>", lambda _e: self.fit())

    def fit(self) -> None:
        if self.locked:
            return
        h = self.body.winfo_reqheight() + 2 * self._pad
        if int(float(self.cget("height"))) != h:
            self.configure(height=h)

    def _on_conf(self, ev) -> None:
        w, h = ev.width, ev.height
        if w < 8 or h < 8:
            return
        self.delete("bg")
        round_rect(self, 0.5, 0.5, w - 1, h - 1, 12, fill=SURFACE, outline=BORDER, tags="bg")
        self.tag_lower("bg")
        self.itemconfigure(self._win, width=max(20, w - 2 * self._pad))


class Sky(tk.Canvas):
    """Cabecera: luna creciente, estrellas que titilan y una estrella fugaz de vez en cuando."""

    def __init__(self, master, version: str) -> None:
        super().__init__(master, width=10, height=SKY_H, bg=BG, highlightthickness=0, bd=0)
        self._version = version
        self._sky_w = 0
        self._stars: list[tuple[int, float, float, str]] = []
        self._t = 0
        self._cool = 90
        self._sn = -1
        self._sitems: list[int] = []
        self._rnd = random.Random(11)
        self._job = None
        self.bind("<Configure>", lambda e: self._build(e.width))

    def _build(self, w: int) -> None:
        if w < 120 or w == self._sky_w:
            return
        self._sky_w = w
        self.delete("all")
        self._stars = []
        self._sitems = []
        self._sn = -1
        rnd = random.Random(11)
        placed = tries = 0
        while placed < 22 and tries < 800:
            tries += 1
            x = rnd.uniform(5, w - 5)
            y = rnd.uniform(5, SKY_H - 5)
            if x < 400 and 4 < y < SKY_H - 4:          # zona de marca y titulo
                continue
            if x > w - 84 and y < 46:                  # zona de la version
                continue
            s = rnd.choice((1, 1, 1, 1.5, 1.5, 2))
            hi = lerp_color("#555555", "#D0D0D0", rnd.random())
            item = self.create_oval(x - s, y - s, x + s, y + s, fill=BG, outline="")
            self._stars.append((item, rnd.uniform(0, 6.28), rnd.uniform(0.05, 0.16), hi))
            placed += 1

        # luna creciente = circulo blanco recortado por otro del color del fondo
        self.create_oval(14, 12, 58, 56, fill=WHITE, outline="")
        self.create_oval(27, 6, 67, 46, fill=BG, outline="")
        self.create_text(84, 24, anchor="w", text="Lunar Offline", fill=WHITE, font=F_TITLE)
        self.create_text(84, 50, anchor="w", fill=MUTED, font=F_SUB,
                         text="Lunar Client completo con un nick no premium.")
        round_rect(self, w - 66, 10, w - 1, 34, 12, fill=BG, outline=BORDER_HI)
        self.create_text(w - 33.5, 22, text=self._version, fill=TEXT_DIM, font=F_SMALL)

    def start(self) -> None:
        if self._job is None:
            self._tick()

    def _tick(self) -> None:
        self._job = None
        try:
            self._t += 1
            for item, phase, speed, hi in self._stars:
                v = (math.sin(self._t * speed + phase) + 1) / 2
                self.itemconfigure(item, fill=lerp_color("#161616", hi, v))
            self._shoot_step()
            self._job = self.after(45, self._tick)
        except tk.TclError:
            self._job = None

    def _shoot_step(self) -> None:
        if self._sn < 0:
            self._cool -= 1
            if self._cool > 0 or self._sky_w < 200:
                return
            self._sn = 0                                  # nace una estrella fugaz
            self._sitems = [
                self.create_line(0, 0, 0, 0, fill=BG, width=2 if i == 0 else 1,
                                 capstyle="round") for i in range(5)]
        n, dx, dy = self._sn, 4.2, 2.4
        hx, hy = self._sky_w - 130 + n * dx, 20 + n * dy
        life = 1.0 if n < 14 else max(0.0, (22 - n) / 8)
        for i, item in enumerate(self._sitems):
            self.coords(item, hx - i * dx * 1.2, hy - i * dy * 1.2,
                        hx - (i + 1) * dx * 1.2, hy - (i + 1) * dy * 1.2)
            self.itemconfigure(item, fill=lerp_color(BG, WHITE, (1 - i / 5) * life))
        self._sn += 1
        if self._sn > 22:
            for item in self._sitems:
                self.delete(item)
            self._sitems = []
            self._sn = -1
            self._cool = self._rnd.randint(130, 260)      # 6-12 s hasta la siguiente


class Tooltip:
    """Globo de ayuda breve al pasar el raton. `text` puede ser un str o una funcion."""

    def __init__(self, widget, text, delay: int = 550) -> None:
        self.widget, self.text, self.delay = widget, text, delay
        self._tip: tk.Toplevel | None = None
        self._job = None
        targets = [widget]
        inner = getattr(widget, "entry", None)
        if inner is not None:
            targets.append(inner)
        for t in targets:
            t.bind("<Enter>", self._schedule, add="+")
            t.bind("<Leave>", self._hide, add="+")
            t.bind("<ButtonPress>", self._hide, add="+")

    def _schedule(self, _e=None) -> None:
        self._cancel()
        self._job = self.widget.after(self.delay, self._show)

    def _cancel(self) -> None:
        if self._job is not None:
            try:
                self.widget.after_cancel(self._job)
            except Exception:
                pass
            self._job = None

    def _show(self) -> None:
        self._job = None
        if self._tip is not None:
            return
        text = self.text() if callable(self.text) else self.text
        if not text:
            return
        x, y = self.widget.winfo_pointerxy()
        tip = tk.Toplevel(self.widget)
        tip.withdraw()
        tip.overrideredirect(True)
        try:
            tip.attributes("-topmost", True)
        except Exception:
            pass
        tk.Label(tip, text=text, fg=WHITE, bg=FIELD_HI, font=F_SMALL, justify="left",
                 wraplength=280, padx=10, pady=6, bd=0,
                 highlightthickness=1, highlightbackground=BORDER_HI).pack()
        tip.geometry(f"+{x + 14}+{y + 18}")
        tip.deiconify()
        self._tip = tip

    def _hide(self, _e=None) -> None:
        self._cancel()
        if self._tip is not None:
            try:
                self._tip.destroy()
            except Exception:
                pass
            self._tip = None


class Link(tk.Label):
    """Texto clicable que se ilumina con una transicion suave."""

    def __init__(self, master, text: str, command, color: str = MUTED,
                 bg: str = BG) -> None:
        super().__init__(master, text=text, fg=color, bg=bg, font=F_LINK,
                         cursor="hand2", padx=9, pady=4)
        self._base = color
        self._tw = Tween(self, lambda: self.configure(fg=lerp_color(self._base, WHITE, self._tw.value)),
                         rate=0.3)
        self.bind("<Button-1>", lambda _e: command())
        self.bind("<Enter>", lambda _e: self._tw.go(1.0))
        self.bind("<Leave>", lambda _e: self._tw.go(0.0))


def caps(master, text: str, bg: str = SURFACE) -> tk.Label:
    """Etiqueta de campo en mayusculas pequenas."""
    return tk.Label(master, text=text.upper(), fg=MUTED, bg=bg,
                    font=F_CAPS, anchor="w")


class Dialog(tk.Toplevel):
    """Aviso modal oscuro, coherente con el resto de la interfaz."""

    def __init__(self, master, title: str, message: str,
                 kind: str = "info", ask: bool = False) -> None:
        super().__init__(master)
        self.result = False
        self.withdraw()
        self.title(title)
        self.configure(bg=BG)
        self.resizable(False, False)
        self.transient(master)

        body = tk.Frame(self, bg=BG)
        body.pack(fill="both", expand=True, padx=24, pady=22)

        tone = {"error": RED, "warn": AMBER}.get(kind, WHITE)
        line = tk.Frame(body, bg=tone, width=3)
        line.pack(side="left", fill="y", padx=(0, 14))
        tk.Label(body, text=message, fg=TEXT_DIM, bg=BG, font=F_INPUT,
                 justify="left", anchor="w", wraplength=390).pack(
                     side="left", fill="both", expand=True)

        row = tk.Frame(self, bg=BG)
        row.pack(fill="x", padx=24, pady=(0, 22))
        small = ("Segoe UI Semibold", -12)
        if ask:
            RoundButton(row, "Cancelar", self._no, height=40, width=118, radius=10,
                        style="ghost", font=small).pack(side="right", padx=(8, 0))
            RoundButton(row, "Continuar", self._yes, height=40, width=118, radius=10,
                        font=small).pack(side="right")
        else:
            RoundButton(row, "Entendido", self._yes, height=40, width=132,
                        radius=10, font=small).pack(side="right")

        self.update_idletasks()
        w = max(360, self.winfo_reqwidth())
        h = self.winfo_reqheight()
        mx, my = master.winfo_rootx(), master.winfo_rooty()
        mw, mh = master.winfo_width(), master.winfo_height()
        self.geometry(f"{w}x{h}+{mx + (mw - w) // 2}+{my + (mh - h) // 3}")
        self.deiconify()
        dark_titlebar(self)
        self.after(200, lambda: dark_titlebar(self))
        self.grab_set()
        self.bind("<Return>", lambda _e: self._yes())
        self.bind("<Escape>", lambda _e: self._no())
        self.protocol("WM_DELETE_WINDOW", self._no)
        self.wait_window(self)

    def _yes(self) -> None:
        self.result = True
        self.destroy()

    def _no(self) -> None:
        self.result = False
        self.destroy()


def dark_titlebar(win: tk.Misc) -> None:
    """Pinta la barra de titulo nativa en modo oscuro. Silencioso si falla."""
    if os.name != "nt":
        return
    try:
        import ctypes
        hwnd = ctypes.windll.user32.GetParent(win.winfo_id())
        value = ctypes.c_int(1)
        for attr in (20, 19):  # DWMWA_USE_IMMERSIVE_DARK_MODE, luego el legacy
            if ctypes.windll.dwmapi.DwmSetWindowAttribute(
                    hwnd, attr, ctypes.byref(value), ctypes.sizeof(value)) == 0:
                break
    except Exception:
        pass


class Launcher(tk.Tk):
    def __init__(self) -> None:
        super().__init__()
        self.title("Lunar Offline Launcher")
        self.geometry(f"{WIN_W}x720")
        self.resizable(False, False)
        self.configure(bg=BG)
        try:
            self.tk.call("tk", "scaling", 1.0)
        except tk.TclError:
            pass

        self.cfg = self._load_cfg()
        self.proc: subprocess.Popen | None = None
        self.log_path: Path | None = None
        self.pending_restore = False
        self.launcher_warned = False
        self._advanced_open = False
        self._cards: list[RoundCard] = []
        self._pulsing = False
        self._pulse_t = 0.0
        self._pulse_job = None
        self._adv_dims: dict[bool, tuple[int, int]] = {}

        # recuperacion de un cierre sucio
        if SWAP_MARKER.exists() or ORIGINAL_ACCOUNTS.is_file():
            accounts_restore()

        self._build()

    # ---------------- persistence ----------------

    def _load_cfg(self) -> dict:
        base = {
            "nick": "", "version": "1.8.9", "ram": 3072, "server": "",
            "width": 1280, "height": 720, "fullscreen": False, "java": "",
            "advanced": False,
        }
        if CONFIG.is_file():
            try:
                base.update(json.loads(CONFIG.read_text(encoding="utf-8")))
            except Exception:
                pass
        return base

    def _save_cfg(self) -> None:
        CONFIG.write_text(json.dumps(self.cfg, indent=2), encoding="utf-8")

    # ---------------- dialogs ----------------

    def _info(self, title: str, msg: str, kind: str = "info") -> None:
        Dialog(self, title, msg, kind=kind)

    def _ask(self, title: str, msg: str) -> bool:
        return Dialog(self, title, msg, kind="warn", ask=True).result

    # ---------------- layout ----------------

    def _card(self, top: int) -> RoundCard:
        card = RoundCard(self)
        card.pack(fill="x", padx=26, pady=(top, 0))
        self._cards.append(card)
        return card

    def _build(self) -> None:
        pad = 26

        # ---- cabecera animada
        self.sky = Sky(self, f"v{APP_VERSION}")
        self.sky.pack(fill="x", padx=pad, pady=(18, 0))

        # ---- tarjeta: cuenta
        b = self._card(14).body
        caps(b, "Nick").pack(fill="x")
        self.nick_var = tk.StringVar(value=self.cfg.get("nick") or "Player")
        self.nick_entry = RoundEntry(b, self.nick_var, height=46, counter=16)
        self.nick_entry.pack(fill="x", pady=(8, 0))
        self.nick_hint = tk.Label(b, text="", fg=FAINT, bg=SURFACE, font=F_SMALL,
                                  anchor="w")
        self.nick_hint.pack(fill="x", pady=(6, 0))
        self.nick_var.trace_add("write", lambda *_a: self._check_nick())
        self._check_nick()
        Tooltip(self.nick_entry,
                "1 a 16 caracteres: A-Z, 0-9 o _.\n"
                "Mismo nick = mismo UUID = mismo inventario en servidores offline.")

        # ---- tarjeta: juego (version, memoria, servidor)
        b = self._card(12).body
        row = tk.Frame(b, bg=SURFACE)
        row.pack(fill="x")
        row.columnconfigure(0, weight=1, uniform="cols")
        row.columnconfigure(1, weight=1, uniform="cols")

        left = tk.Frame(row, bg=SURFACE)
        left.grid(row=0, column=0, sticky="new", padx=(0, 12))
        caps(left, "Versión").pack(fill="x")
        self.version_var = tk.StringVar(value=self.cfg.get("version", "1.8.9"))
        vers = available_versions() or ["1.8.9"]
        if self.version_var.get() not in vers:
            self.version_var.set(vers[0])
        Segmented(left, vers, self.version_var).pack(fill="x", pady=(8, 0))

        right = tk.Frame(row, bg=SURFACE)
        right.grid(row=0, column=1, sticky="new", padx=(12, 0))
        cap_row = tk.Frame(right, bg=SURFACE)
        cap_row.pack(fill="x")
        caps(cap_row, "Memoria").pack(side="left")
        self.ram_lbl = tk.Label(cap_row, text="", fg=TEXT, bg=SURFACE,
                                font=("Segoe UI Semibold", -11))
        self.ram_lbl.pack(side="right")
        self.ram_var = tk.IntVar(value=int(self.cfg.get("ram", 3072)))
        self.ram_slider = Slider(right, self.ram_var, 1024, 8192, step=256,
                                 on_change=self._ram_label)
        self.ram_slider.pack(fill="x", pady=(8, 0))
        self._ram_label()
        Tooltip(self.ram_slider,
                "RAM asignada al juego (-Xms/-Xmx).\n3072 MB va bien para 1.8.9.")

        caps(b, "Servidor").pack(fill="x", pady=(16, 0))
        self.server_var = tk.StringVar(value=self.cfg.get("server", ""))
        self.server_entry = RoundEntry(b, self.server_var, height=44)
        self.server_entry.pack(fill="x", pady=(8, 0))
        tk.Label(b, text="host o host:puerto — vacío entra al menú principal",
                 fg=FAINT, bg=SURFACE, font=F_SMALL, anchor="w").pack(fill="x", pady=(6, 0))
        Tooltip(self.server_entry,
                "Ej.: mc.miservidor.net o mc.miservidor.net:25566\n"
                "El servidor debe tener online-mode=false.")

        # ---- tarjeta: avanzado (se despliega con animacion)
        self.adv_card = self._card(12)
        b = self.adv_card.body
        self.adv_head = tk.Canvas(b, width=10, height=26, bg=SURFACE,
                                  highlightthickness=0, bd=0, cursor="hand2")
        self.adv_head.pack(fill="x")

        self.adv = tk.Frame(b, bg=SURFACE)
        self.adv.columnconfigure(1, weight=1)

        caps(self.adv, "Resolución").grid(row=0, column=0, sticky="w", pady=(14, 0))
        res = tk.Frame(self.adv, bg=SURFACE)
        res.grid(row=0, column=1, sticky="w", pady=(14, 0), padx=(16, 0))
        self.w_var = tk.StringVar(value=str(self.cfg.get("width", 1280)))
        self.h_var = tk.StringVar(value=str(self.cfg.get("height", 720)))
        for i, var in enumerate((self.w_var, self.h_var)):
            if i == 1:
                tk.Label(res, text="×", fg=FAINT, bg=SURFACE,
                         font=F_INPUT).pack(side="left", padx=9)
            tk.Entry(res, textvariable=var, width=6, bd=0, relief="flat",
                     bg=FIELD, fg=TEXT, insertbackground=WHITE, font=F_INPUT,
                     selectbackground=WHITE, selectforeground=INK,
                     highlightthickness=1, highlightbackground=BORDER,
                     highlightcolor=WHITE, justify="center").pack(side="left", ipady=6)

        self.fs_var = tk.BooleanVar(value=bool(self.cfg.get("fullscreen", False)))
        fs_box = tk.Frame(self.adv, bg=SURFACE)
        fs_box.grid(row=1, column=0, columnspan=2, sticky="w", pady=(14, 0))
        Toggle(fs_box, self.fs_var).pack(side="left")
        tk.Label(fs_box, text="Pantalla completa", fg=TEXT_DIM, bg=SURFACE,
                 font=F_INPUT).pack(side="left", padx=(10, 0))

        caps(self.adv, "Java").grid(row=2, column=0, sticky="w", pady=(14, 0))
        jf = tk.Frame(self.adv, bg=SURFACE)
        jf.grid(row=2, column=1, sticky="ew", padx=(16, 0), pady=(14, 0))
        jf.columnconfigure(0, weight=1)
        self.java_var = tk.StringVar(value=self.cfg.get("java") or (find_java() or ""))
        self.java_field = PathField(jf, self.java_var)
        self.java_field.grid(row=0, column=0, sticky="ew")
        RoundButton(jf, "Examinar", self._pick_java, height=38, width=96, radius=9,
                    bg=SURFACE, style="ghost",
                    font=("Segoe UI Semibold", -12)).grid(row=0, column=1, padx=(10, 0))
        Tooltip(self.java_field,
                lambda: self.java_var.get() or "Usa Examinar para elegir javaw.exe")

        # animacion del panel: 0 = plegado, 1 = desplegado
        self._adv_tw = Tween(self, self._adv_frame, rate=0.22, done=self._adv_done)
        self.adv_head.bind("<Configure>", lambda _e: self._draw_adv_head())
        self.adv_head.bind("<Button-1>", lambda _e: self._toggle_advanced())

        # ---- boton principal
        foot = tk.Frame(self, bg=BG)
        foot.pack(fill="x", padx=pad, pady=(20, 0))
        self.launch_btn = RoundButton(foot, "Jugar con Lunar", self.launch,
                                      height=56, radius=14, icon="play")
        self.launch_btn.pack(fill="x")

        # ---- pie: estado a la izquierda, acciones a la derecha
        tk.Frame(self, bg=BORDER, height=1).pack(fill="x", pady=(18, 0))
        bar = tk.Frame(self, bg=BG)
        bar.pack(fill="x", padx=pad - 9, pady=(8, 10))
        self.dot = tk.Canvas(bar, width=10, height=10, bg=BG, highlightthickness=0)
        self.dot.pack(side="left", padx=(9, 8))
        self._dot_item = self.dot.create_oval(1, 1, 9, 9, fill=WHITE, outline="")
        self.status = tk.Label(bar, text="Listo", fg=TEXT_DIM, bg=BG, font=F_LINK,
                               anchor="w")
        self.status.pack(side="left")
        Link(bar, "Restaurar", self._restore).pack(side="right")
        Link(bar, "Diagnóstico", self._diagnose).pack(side="right")
        Link(bar, "Log", self._open_log).pack(side="right")

        # ---- alturas con el panel avanzado abierto y cerrado
        self._measure_adv()
        if self.cfg.get("advanced"):
            self._advanced_open = True
            self.adv_card.locked = True
            self.adv.pack(fill="x")
            self._adv_tw.jump(1.0)
            self.adv_card.locked = False

        self.update_idletasks()
        dark_titlebar(self)
        self.after(250, lambda: dark_titlebar(self))
        self.nick_entry.focus()
        self.bind("<Return>", lambda _e: self.launch())
        self.protocol("WM_DELETE_WINDOW", self._on_close)
        self.sky.start()
        self._fade_in()

    # ---------------- animaciones de la ventana ----------------

    def _fade_in(self) -> None:
        """Aparece con un fundido corto. Si el sistema no soporta alpha, no pasa nada."""
        try:
            self.attributes("-alpha", 0.0)
        except tk.TclError:
            return

        def step(a: float = 0.0) -> None:
            a = min(1.0, a + 0.09)
            try:
                self.attributes("-alpha", a)
                if a < 1.0:
                    self.after(16, lambda: step(a))
            except tk.TclError:
                pass
        step()

    def _apply_height(self) -> None:
        """Ajusta tarjetas y ventana a la altura real del contenido."""
        self.update_idletasks()
        for card in self._cards:
            card.fit()
        self.update_idletasks()
        self.geometry(f"{WIN_W}x{self.winfo_reqheight()}")

    def _measure_adv(self) -> None:
        """Mide (alto de tarjeta, alto de ventana) con Avanzado abierto y cerrado."""
        for opened in (True, False):
            if opened:
                self.adv.pack(fill="x")
            else:
                self.adv.pack_forget()
            self._apply_height()
            self._adv_dims[opened] = (int(float(self.adv_card.cget("height"))),
                                      self.winfo_reqheight())
        self._draw_adv_head()

    def _adv_frame(self) -> None:
        v = self._adv_tw.value
        (c0, r0), (c1, r1) = self._adv_dims[False], self._adv_dims[True]
        self.adv_card.configure(height=int(round(c0 + (c1 - c0) * v)))
        self.geometry(f"{WIN_W}x{int(round(r0 + (r1 - r0) * v))}")
        self._draw_adv_head()

    def _adv_done(self) -> None:
        if not self._advanced_open:
            self.adv.pack_forget()
        self.adv_card.locked = False
        self._apply_height()

    def _draw_adv_head(self) -> None:
        w, h = self.adv_head.winfo_width(), self.adv_head.winfo_height()
        if w < 8 or h < 8:
            return
        c = self.adv_head
        c.delete("all")
        c.create_text(0, h / 2, anchor="w", text="Avanzado", fill=TEXT,
                      font=("Segoe UI Semibold", -13))
        v = self._adv_tw.value if hasattr(self, "_adv_tw") else 0.0
        cx, cy = w - 8, h / 2
        d = 3 - 6 * v                                  # la flecha gira al abrir
        c.create_line(cx - 5, cy - d, cx, cy + d, cx + 5, cy - d, fill=MUTED, width=2,
                      capstyle="round", joinstyle="round")

    def _toggle_advanced(self) -> None:
        self._advanced_open = not self._advanced_open
        self.adv_card.locked = True
        if self._advanced_open:
            self.adv.pack(fill="x")
        self._adv_tw.go(1.0 if self._advanced_open else 0.0)
        self.cfg["advanced"] = self._advanced_open
        try:
            self._save_cfg()
        except Exception:
            pass

    # ---------------- estado y validacion ----------------

    def _check_nick(self) -> None:
        """Validacion visual: borde rojo y mensaje corto si el nick no sirve."""
        nick = self.nick_var.get()
        if NICK_RE.match(nick):
            self.nick_entry.set_error(False)
            self.nick_hint.configure(text="Nick válido", fg=FAINT)
            return
        if not nick:
            msg = "Escribe un nick (1 a 16 caracteres)."
        elif len(nick) > 16:
            msg = "Máximo 16 caracteres."
        else:
            msg = "Solo letras, números y _ (sin espacios)."
        self.nick_entry.set_error(True)
        self.nick_hint.configure(text=msg, fg=RED)

    def _ram_label(self, _v=None) -> None:
        mb = int(self.ram_var.get())
        gb = f"{mb // 1024}" if mb % 1024 == 0 else f"{mb / 1024:.1f}"
        self.ram_lbl.configure(text=f"{mb} MB · {gb} GB")

    def report_callback_exception(self, exc, val, tb) -> None:  # noqa: N802
        """Una excepcion en un callback de Tk no puede desaparecer en silencio."""
        import traceback
        texto = "".join(traceback.format_exception(exc, val, tb))
        try:
            (APP_DIR / "error.log").write_text(texto, encoding="utf-8")
        except Exception:
            pass
        try:
            self._info("Error interno",
                       f"{val}\n\nDetalle guardado en:\n{APP_DIR / 'error.log'}",
                       kind="error")
        except Exception:
            pass

    def _pick_java(self) -> None:
        path = filedialog.askopenfilename(
            title="Selecciona javaw.exe",
            filetypes=[("Java", "javaw.exe java.exe"), ("Todos", "*.*")],
        )
        if path:
            self.java_var.set(path)

    # ---------------- actions ----------------

    def _set_status(self, text: str, color: str = MUTED, pulse: bool = False) -> None:
        """Punto + texto del pie. `pulse` hace latir el punto (juego corriendo)."""
        self._pulsing = pulse
        self.dot.itemconfigure(self._dot_item, fill=WHITE if color == MUTED else color)
        self.status.configure(text=text, fg=TEXT_DIM if color == MUTED else color)
        if pulse and self._pulse_job is None:
            self._pulse_step()
        self.update_idletasks()

    def _pulse_step(self) -> None:
        self._pulse_job = None
        if not self._pulsing:
            self.dot.itemconfigure(self._dot_item, fill=WHITE)
            return
        self._pulse_t += 0.13
        v = (math.sin(self._pulse_t) + 1) / 2
        try:
            self.dot.itemconfigure(self._dot_item, fill=lerp_color("#4A4A4A", WHITE, v))
            self._pulse_job = self.after(40, self._pulse_step)
        except tk.TclError:
            self._pulse_job = None

    def _restore(self) -> None:
        if accounts_restore():
            self._info("Cuenta restaurada",
                       "Tu cuenta premium volvio a quedar activa.\n"
                       "Los logins hechos mientras jugabas se conservaron.", kind="ok")
        else:
            self._info("Nada que restaurar",
                       "No hay ningun cambio de cuenta pendiente.")

    def _diagnose(self) -> None:
        java = self.java_var.get().strip()
        natives = find_natives()
        lines = [
            f"Lunar:      {LUNAR_ROOT}  {'OK' if LUNAR_ROOT.is_dir() else 'FALTA'}",
            f"Multiver:   {MULTIVER}  {'OK' if MULTIVER.is_dir() else 'FALTA'}",
            f"Java:       {'OK' if java and Path(java).is_file() else 'FALTA'}",
            f"Natives:    {natives or 'NO ENCONTRADO'}",
            f"Minecraft:  {'OK' if MC_DIR.is_dir() else 'FALTA'}",
            f"Cuentas:    {'OK' if ACCOUNTS.is_file() else 'FALTA'}",
            "",
            "Versiones listas: " + (", ".join(available_versions()) or "ninguna"),
        ]
        for v in available_versions():
            missing = [j for j in classpath_jars(v) if not (MULTIVER / j).is_file()]
            lines.append(f"  {v}: {len(classpath_jars(v))} jars" +
                         (f", FALTAN {missing}" if missing else ""))
        self._info("Diagnostico", "\n".join(lines))

    def _open_log(self) -> None:
        if self.log_path and self.log_path.is_file():
            subprocess.Popen(["notepad.exe", str(self.log_path)])
        else:
            self._info("Log", "Todavia no lanzaste el juego en esta sesion.")

    def launch(self) -> None:
        if self.proc and self.proc.poll() is None:
            self._info("Ya esta abierto", "Lunar ya se esta ejecutando.")
            return

        # El launcher oficial reescribe accounts.json: si esta abierto mientras
        # el swap vive, la app no puede restaurar limpio y Lunar puede cerrar
        # sesion. Se bloquea el arranque hasta que se cierre.
        if official_launcher_running():
            self._info(
                "Cierra Lunar Client",
                "El launcher oficial esta abierto y reescribe accounts.json.\n"
                "Cierralo y vuelve a pulsar Jugar.", kind="warn")
            return

        nick = self.nick_var.get().strip()
        if not NICK_RE.match(nick):
            # sin dialogo: el campo se sacude y el mensaje rojo ya dice que pasa
            self.nick_entry.shake()
            self.nick_entry.focus()
            return

        java = self.java_var.get().strip()
        if not java or not Path(java).is_file():
            self._info("Falta Java",
                       "No encuentro un javaw.exe valido.\nAbre Avanzado y usa Examinar.",
                       kind="error")
            return

        try:
            ram = int(self.ram_var.get())
            width = int(self.w_var.get())
            height = int(self.h_var.get())
        except ValueError:
            self._info("Valores invalidos", "RAM y resolucion deben ser numeros.",
                       kind="error")
            return

        stamp = time.strftime("%Y%m%d-%H%M%S")
        self.log_path = LOGS / f"run-{stamp}.log"

        try:
            cmd = build_command(nick, self.version_var.get(), ram,
                                self.server_var.get(), width, height,
                                self.fs_var.get(), java)
        except Exception as exc:
            self._info("Error", str(exc), kind="error")
            return

        self.cfg.update({
            "nick": nick, "version": self.version_var.get(), "ram": ram,
            "server": self.server_var.get(), "width": width, "height": height,
            "fullscreen": self.fs_var.get(), "java": java,
        })
        self._save_cfg()

        (LOGS / f"cmd-{stamp}.txt").write_text(
            " ".join(f'"{c}"' if " " in c else c for c in cmd), encoding="utf-8")

        try:
            accounts_swap(nick)
        except Exception as exc:
            self._info("Cuentas", f"No pude escribir accounts.json:\n{exc}",
                       kind="error")
            return

        self.pending_restore = True
        self.launcher_warned = False

        flags = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0  # type: ignore[attr-defined]
        try:
            with open(self.log_path, "wb") as log:
                self.proc = subprocess.Popen(
                    cmd, cwd=str(MULTIVER), stdout=log, stderr=subprocess.STDOUT,
                    creationflags=flags,
                )
        except Exception as exc:
            accounts_restore()
            self.pending_restore = False
            self._info("No arranco", str(exc), kind="error")
            return

        self.launch_btn.set_state("Lunar abierto", disabled=True)
        self._set_status(f"Corriendo · PID {self.proc.pid}", WHITE, pulse=True)
        self.after(4000, self._watch)

    def _watch(self) -> None:
        if self.proc is None:
            return
        rc = self.proc.poll()
        if rc is None:
            if official_launcher_running() and not self.launcher_warned:
                self.launcher_warned = True
                self._set_status("Cierra el launcher oficial", AMBER)
            elif not official_launcher_running():
                self._set_status(f"Corriendo · PID {self.proc.pid}", WHITE, pulse=True)
            self.after(3000, self._watch)
        else:
            if self.pending_restore:
                accounts_restore()
                self.pending_restore = False
            self.launch_btn.set_state("Jugar con Lunar", disabled=False)
            self._set_status("Cuenta restaurada", MUTED)

    def _on_close(self) -> None:
        if self.proc and self.proc.poll() is None:
            if not self._ask("Lunar sigue abierto",
                             "El juego esta corriendo. Cerrar la ventana igualmente?\n"
                             "La cuenta se restaura sola cuando el juego termine."):
                return
            self.after(1500, self._watch)
            self.iconify()
            return
        if self.pending_restore:
            accounts_restore()
        self.destroy()


def selftest() -> int:
    """Modo soporte: escribe un informe en %APPDATA%\\LunarOffline\\selftest.txt."""
    out = APP_DIR / "selftest.txt"
    java = find_java()
    natives = find_natives()
    lines = [
        f"python: {sys.version.split()[0]} frozen={getattr(sys, 'frozen', False)}",
        f"lunar_root: {LUNAR_ROOT} exists={LUNAR_ROOT.is_dir()}",
        f"multiver: {MULTIVER} exists={MULTIVER.is_dir()}",
        f"java: {java} ok={bool(java and Path(java).is_file())}",
        f"natives: {natives}",
        f"minecraft: {MC_DIR} exists={MC_DIR.is_dir()}",
        f"accounts: {ACCOUNTS} exists={ACCOUNTS.is_file()}",
        f"launcher_oficial_abierto: {official_launcher_running()}",
        f"swap_pending: {SWAP_MARKER.exists()}",
        f"cuentas_registradas: {len(read_accounts(ACCOUNTS).get('accounts', {}))}",
        f"versions: {available_versions()}",
    ]
    rc = 0
    for v in available_versions():
        try:
            cmd = build_command("TestOffline", v, 2048, "example.com:25565", 854, 480, False, java or "")
            lines.append(f"build_command[{v}]: OK ({len(cmd)} args)")
        except Exception as exc:
            lines.append(f"build_command[{v}]: FAIL {exc}")
            rc = 1
    for v in ("1.8.9", "1.7.10"):
        if v in available_versions():
            for j in classpath_jars(v):
                if not (MULTIVER / j).is_file():
                    lines.append(f"FALTA {v}: {j}")
                    rc = 1
    out.write_text("\n".join(lines), encoding="utf-8")
    return rc


def main() -> None:
    if "--selftest" in sys.argv:
        raise SystemExit(selftest())
    app = Launcher()
    app.mainloop()


if __name__ == "__main__":
    main()
