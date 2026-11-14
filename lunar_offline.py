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
import os
import re
import shutil
import subprocess
import sys
import time
import uuid
from pathlib import Path

import tkinter as tk
from tkinter import ttk, messagebox, filedialog

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
# gui
# --------------------------------------------------------------------------- #

# --------------------------------------------------------------------------- #
# minimal ui toolkit (canvas-drawn, sin ttk, sin bordes grises del sistema)
# --------------------------------------------------------------------------- #

BG        = "#0B0C10"
CARD      = "#13151C"
CARD_HI   = "#191C25"
BORDER    = "#222633"
BORDER_HI = "#3C4459"
TEXT      = "#EEEFF4"
MUTED     = "#6B7080"
ACCENT    = "#4C7DFF"
ACCENT_HI = "#6D95FF"
AMBER     = "#E5AE5A"
GREEN     = "#4ED08A"

APP_VERSION = "1.2.0"

F_TITLE = ("Segoe UI Semibold", -22)
F_SUB   = ("Segoe UI", -11)
F_CAPS  = ("Segoe UI Semibold", -9)
F_INPUT = ("Segoe UI", -13)
F_BTN   = ("Segoe UI Semibold", -14)
F_SMALL = ("Segoe UI", -10)


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


class RoundEntry(tk.Canvas):
    """Campo de texto plano con esquinas redondeadas y borde de foco."""

    def __init__(self, master, textvariable: tk.Variable, height: int = 44,
                 font=F_INPUT, pad: int = 15, radius: int = 10) -> None:
        super().__init__(master, height=height, bg=BG, highlightthickness=0, bd=0)
        self._radius = radius
        self._focused = False
        self.entry = tk.Entry(
            self, textvariable=textvariable, bd=0, relief="flat",
            bg=CARD, fg=TEXT, insertbackground=ACCENT, font=font,
            highlightthickness=0, selectbackground=ACCENT, selectforeground="#ffffff",
        )
        self._win = self.create_window(pad, height // 2, anchor="w", window=self.entry)
        self.bind("<Configure>", lambda _e: self._draw())
        self.entry.bind("<FocusIn>", lambda _e: self._focus(True))
        self.entry.bind("<FocusOut>", lambda _e: self._focus(False))
        self.entry.bind("<Button-1>", lambda _e: self.entry.focus_set())

    def _focus(self, on: bool) -> None:
        self._focused = on
        self._draw()

    def _draw(self) -> None:
        w, h = self.winfo_width(), self.winfo_height()
        if w < 8 or h < 8:
            return
        self.delete("shape")
        round_rect(self, 0.5, 0.5, w - 1, h - 1, self._radius,
                   fill=CARD, outline=BORDER_HI if self._focused else BORDER,
                   width=1.4 if self._focused else 1, tags="shape")
        self.tag_lower("shape")
        self.itemconfigure(self._win, width=max(30, w - 2 * 15), height=h - 14)

    def focus(self) -> None:  # type: ignore[override]
        self.entry.focus_set()


class RoundButton(tk.Canvas):
    """Boton primario redondeado con estado hover y texto centrado."""

    def __init__(self, master, text: str, command, height: int = 52,
                 fill: str = ACCENT, hover: str = ACCENT_HI,
                 fg: str = "#FFFFFF", font=F_BTN, radius: int = 12,
                 width: int = 0) -> None:
        super().__init__(master, height=height, width=width or 120,
                         bg=BG, highlightthickness=0, bd=0, cursor="hand2")
        self._text, self._cmd = text, command
        self._fill, self._hover, self._fg = fill, hover, fg
        self._font, self._radius = font, radius
        self._over = False
        self._down = False
        self.bind("<Configure>", lambda _e: self._draw())
        self.bind("<Enter>", lambda _e: self._set(True))
        self.bind("<Leave>", lambda _e: self._set(False))
        self.bind("<ButtonPress-1>", lambda _e: self._press(True))
        self.bind("<ButtonRelease-1>", lambda _e: self._press(False))

    def _set(self, over: bool) -> None:
        self._over = over
        self._draw()

    def _press(self, down: bool) -> None:
        self._down = down
        self._draw()
        if not down and self._over:
            self._cmd()

    def set_text(self, text: str) -> None:
        self._text = text
        self._draw()

    def _draw(self) -> None:
        w, h = self.winfo_width(), self.winfo_height()
        if w < 8 or h < 8:
            return
        self.delete("all")
        fill = self._fill
        if self._down:
            fill = self._hover
        elif self._over:
            fill = self._hover
        round_rect(self, 0.5, 0.5, w - 1, h - 1, self._radius, fill=fill, outline=fill)
        self.create_text(w / 2, h / 2, text=self._text, fill=self._fg,
                         font=self._font)


class Segmented(tk.Canvas):
    """Selector de opciones tipo pill group."""

    def __init__(self, master, values: list[str], variable: tk.StringVar,
                 height: int = 34, on_change=None) -> None:
        super().__init__(master, height=height, bg=BG, highlightthickness=0,
                         bd=0, cursor="hand2")
        self.values = list(values) or ["1.8.9"]
        self.var = variable
        self._on_change = on_change
        self.bind("<Configure>", lambda _e: self._draw())
        self.bind("<Button-1>", self._click)

    def _click(self, ev) -> None:
        n = len(self.values)
        w = self.winfo_width()
        idx = min(n - 1, max(0, int(ev.x / (w / n))))
        self.var.set(self.values[idx])
        self._draw()
        if self._on_change:
            self._on_change()

    def _draw(self) -> None:
        w, h = self.winfo_width(), self.winfo_height()
        if w < 8 or h < 8:
            return
        self.delete("all")
        round_rect(self, 0.5, 0.5, w - 1, h - 1, 9, fill=CARD, outline=BORDER)
        n = len(self.values)
        seg = w / n
        for i, val in enumerate(self.values):
            x1 = i * seg
            active = val == self.var.get()
            if active:
                round_rect(self, x1 + 3, 3, x1 + seg - 3, h - 3, 7,
                           fill=ACCENT, outline=ACCENT)
            self.create_text(x1 + seg / 2, h / 2, text=val,
                             font=("Segoe UI Semibold", -11),
                             fill="#FFFFFF" if active else MUTED)


class Slider(tk.Canvas):
    """Slider horizontal minimalista con pasos fijos."""

    def __init__(self, master, variable: tk.IntVar, lo: int, hi: int,
                 step: int = 256, on_change=None) -> None:
        super().__init__(master, height=34, bg=BG, highlightthickness=0,
                         bd=0, cursor="hand2")
        self.var, self.lo, self.hi, self.step = variable, lo, hi, step
        self._on_change = on_change
        self.bind("<Configure>", lambda _e: self._draw())
        self.bind("<Button-1>", self._set_from_x)
        self.bind("<B1-Motion>", self._set_from_x)

    def _set_from_x(self, ev) -> None:
        w = self.winfo_width()
        frac = min(1.0, max(0.0, (ev.x - 10) / max(1, w - 20)))
        raw = self.lo + frac * (self.hi - self.lo)
        val = int(round(raw / self.step) * self.step)
        val = max(self.lo, min(self.hi, val))
        if val != self.var.get():
            self.var.set(val)
            self._draw()
            if self._on_change:
                self._on_change()

    def _draw(self) -> None:
        w, h = self.winfo_width(), self.winfo_height()
        if w < 8 or h < 8:
            return
        self.delete("all")
        x0, x1, cy = 10, w - 10, h / 2
        frac = (self.var.get() - self.lo) / max(1, (self.hi - self.lo))
        frac = min(1.0, max(0.0, frac))
        round_rect(self, x0, cy - 2, x1, cy + 2, 2, fill=BORDER, outline=BORDER)
        if frac > 0.002:
            round_rect(self, x0, cy - 2, x0 + (x1 - x0) * frac, cy + 2, 2,
                       fill=ACCENT, outline=ACCENT)
        kx = x0 + (x1 - x0) * frac
        self.create_oval(kx - 8, cy - 8, kx + 8, cy + 8, fill="#FFFFFF",
                         outline=ACCENT, width=2)


class Toggle(tk.Canvas):
    """Interruptor on/off."""

    def __init__(self, master, variable: tk.BooleanVar, on_change=None) -> None:
        super().__init__(master, width=42, height=24, bg=BG,
                         highlightthickness=0, bd=0, cursor="hand2")
        self.var = variable
        self._on_change = on_change
        self.bind("<Button-1>", self._click)
        self._draw()

    def _click(self, _ev=None) -> None:
        self.var.set(not self.var.get())
        self._draw()
        if self._on_change:
            self._on_change()

    def _draw(self) -> None:
        w, h = 42, 24
        self.delete("all")
        on = bool(self.var.get())
        round_rect(self, 1, 1, w - 1, h - 1, (h - 2) / 2,
                   fill=ACCENT if on else CARD,
                   outline=ACCENT if on else BORDER)
        cx = w - 12 if on else 12
        self.create_oval(cx - 8, 4, cx + 8, h - 4,
                         fill="#FFFFFF" if on else MUTED, outline="")


class Link(tk.Label):
    """Texto clicable discreto."""

    def __init__(self, master, text: str, command, color: str = MUTED) -> None:
        super().__init__(master, text=text, fg=color, bg=BG, font=F_SMALL,
                         cursor="hand2")
        self._base = color
        self.bind("<Button-1>", lambda _e: command())
        self.bind("<Enter>", lambda _e: self.configure(fg=TEXT))
        self.bind("<Leave>", lambda _e: self.configure(fg=self._base))


def caps(master, text: str) -> tk.Label:
    """Etiqueta de campo en mayusculas pequenas con tracking manual."""
    return tk.Label(master, text=text.upper(), fg=MUTED, bg=BG,
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

        tone = {"error": "#FF6B6B", "warn": AMBER, "ok": GREEN}.get(kind, ACCENT)
        line = tk.Frame(body, bg=tone, width=3)
        line.pack(side="left", fill="y", padx=(0, 14))
        tk.Label(body, text=message, fg="#C2C5D2", bg=BG, font=F_INPUT,
                 justify="left", anchor="w", wraplength=390).pack(
                     side="left", fill="both", expand=True)

        row = tk.Frame(self, bg=BG)
        row.pack(fill="x", padx=24, pady=(0, 22))
        if ask:
            RoundButton(row, "Cancelar", self._no, height=40, width=118, fill=CARD,
                        hover=CARD_HI, fg=TEXT,
                        font=("Segoe UI Semibold", -12)).pack(side="right", padx=(8, 0))
            RoundButton(row, "Continuar", self._yes, height=40, width=118, radius=10,
                        font=("Segoe UI Semibold", -12)).pack(side="right")
        else:
            RoundButton(row, "Entendido", self._yes, height=40, width=132,
                        radius=10, font=("Segoe UI Semibold", -12)).pack(side="right")

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
        self.geometry("580x572")
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
        self._base_h = 572
        self._full_h = 704

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

    def _build(self) -> None:
        # ---- cabecera
        head = tk.Frame(self, bg=BG)
        head.pack(fill="x", padx=26, pady=(24, 0))

        brand = tk.Frame(head, bg=BG)
        brand.pack(side="left", anchor="w")
        mark = tk.Canvas(brand, width=10, height=10, bg=BG, highlightthickness=0)
        mark.pack(side="left", padx=(0, 9), pady=(8, 0))
        mark.create_oval(0, 0, 10, 10, fill=ACCENT, outline="")
        tk.Label(brand, text="Lunar Offline", fg=TEXT, bg=BG,
                 font=F_TITLE).pack(side="left")

        tk.Label(head, text=f"v{APP_VERSION}", fg=MUTED, bg=BG,
                 font=F_SMALL).pack(side="right", pady=(12, 0))
        tk.Label(self, text="Lunar Client completo con un nick no premium.",
                 fg=MUTED, bg=BG, font=F_SUB, anchor="w") \
            .pack(fill="x", padx=26, pady=(4, 0))

        # ---- cuerpo
        body = tk.Frame(self, bg=BG)
        body.pack(fill="x", padx=26, pady=(22, 0))

        caps(body, "Nick").pack(fill="x")
        self.nick_var = tk.StringVar(value=self.cfg.get("nick") or "Player")
        self.nick_entry = RoundEntry(body, self.nick_var, height=46)
        self.nick_entry.pack(fill="x", pady=(6, 0))

        # fila version / ram
        row = tk.Frame(body, bg=BG)
        row.pack(fill="x", pady=(16, 0))
        row.columnconfigure(0, weight=0, minsize=190)
        row.columnconfigure(1, weight=1)

        left = tk.Frame(row, bg=BG)
        left.grid(row=0, column=0, sticky="new", padx=(0, 16))
        caps(left, "Version").pack(fill="x")
        self.version_var = tk.StringVar(value=self.cfg.get("version", "1.8.9"))
        vers = available_versions() or ["1.8.9"]
        if self.version_var.get() not in vers:
            self.version_var.set(vers[0])
        Segmented(left, vers, self.version_var).pack(fill="x", pady=(6, 0))

        right = tk.Frame(row, bg=BG)
        right.grid(row=0, column=1, sticky="new")
        cap_row = tk.Frame(right, bg=BG)
        cap_row.pack(fill="x")
        caps(cap_row, "Memoria").pack(side="left")
        self.ram_lbl = tk.Label(cap_row, text="", fg=TEXT, bg=BG,
                                font=("Segoe UI Semibold", -9))
        self.ram_lbl.pack(side="right")
        self.ram_var = tk.IntVar(value=int(self.cfg.get("ram", 3072)))
        Slider(right, self.ram_var, 1024, 8192, step=256,
               on_change=self._ram_label).pack(fill="x", pady=(6, 0))
        self._ram_label()

        # servidor
        caps(body, "Servidor").pack(fill="x", pady=(16, 0))
        self.server_var = tk.StringVar(value=self.cfg.get("server", ""))
        self.server_entry = RoundEntry(body, self.server_var, height=42)
        self.server_entry.pack(fill="x", pady=(6, 0))
        tk.Label(body, text="host o host:puerto — vacio entra al menu principal",
                 fg=MUTED, bg=BG, font=F_SMALL, anchor="w").pack(fill="x", pady=(5, 0))

        # avanzado
        adv_head = tk.Frame(body, bg=BG)
        adv_head.pack(fill="x", pady=(14, 0))
        self.adv_link = Link(adv_head, "Avanzado", self._toggle_advanced,
                             color="#8E94A6")
        self.adv_link.pack(side="left")

        self.adv = tk.Frame(body, bg=BG)
        self.adv.columnconfigure(3, weight=1)

        caps(self.adv, "Resolucion").grid(row=0, column=0, sticky="w", pady=(10, 0))
        res = tk.Frame(self.adv, bg=BG)
        res.grid(row=0, column=1, columnspan=3, sticky="w", pady=(10, 0), padx=(14, 0))
        self.w_var = tk.StringVar(value=str(self.cfg.get("width", 1280)))
        self.h_var = tk.StringVar(value=str(self.cfg.get("height", 720)))
        w_entry = tk.Entry(res, textvariable=self.w_var, width=6, bd=0, relief="flat",
                           bg=CARD, fg=TEXT, insertbackground=ACCENT, font=F_INPUT,
                           highlightthickness=1, highlightbackground=BORDER,
                           highlightcolor=ACCENT, justify="center")
        w_entry.pack(side="left", ipady=5)
        tk.Label(res, text="x", fg=MUTED, bg=BG, font=F_INPUT).pack(side="left", padx=7)
        h_entry = tk.Entry(res, textvariable=self.h_var, width=6, bd=0, relief="flat",
                           bg=CARD, fg=TEXT, insertbackground=ACCENT, font=F_INPUT,
                           highlightthickness=1, highlightbackground=BORDER,
                           highlightcolor=ACCENT, justify="center")
        h_entry.pack(side="left", ipady=5)

        self.fs_var = tk.BooleanVar(value=bool(self.cfg.get("fullscreen", False)))
        fs_box = tk.Frame(self.adv, bg=BG)
        fs_box.grid(row=1, column=0, columnspan=2, sticky="w", pady=(10, 0))
        Toggle(fs_box, self.fs_var).pack(side="left")
        tk.Label(fs_box, text="Pantalla completa", fg="#B9BCC9", bg=BG,
                 font=F_SUB).pack(side="left", padx=(9, 0))

        caps(self.adv, "Java").grid(row=2, column=0, sticky="w", pady=(12, 0))
        jf = tk.Frame(self.adv, bg=BG)
        jf.grid(row=2, column=1, columnspan=3, sticky="ew", padx=(14, 0), pady=(12, 0))
        jf.columnconfigure(0, weight=1)
        self.java_var = tk.StringVar(value=self.cfg.get("java") or (find_java() or ""))
        tk.Entry(jf, textvariable=self.java_var, bd=0, relief="flat", bg=CARD,
                 fg=TEXT, insertbackground=ACCENT, font=("Consolas", -10),
                 highlightthickness=1, highlightbackground=BORDER,
                 highlightcolor=ACCENT).grid(row=0, column=0, sticky="ew", ipady=6)
        Link(jf, "Examinar", self._pick_java, color="#8E94A6") \
            .grid(row=0, column=1, padx=(10, 0))

        # ---- pie fijo
        foot = tk.Frame(self, bg=BG)
        foot.pack(fill="x", padx=26, pady=(26, 20))

        self.launch_btn = RoundButton(foot, "Jugar con Lunar", self.launch, height=54)
        self.launch_btn.pack(fill="x")

        bar = tk.Frame(foot, bg=BG)
        bar.pack(fill="x", pady=(10, 0))
        self.status = tk.Label(bar, text="Listo", fg=MUTED, bg=BG, font=F_SMALL,
                               anchor="w")
        self.status.pack(side="left")
        Link(bar, "Restaurar", self._restore).pack(side="right")
        Link(bar, "Diagnostico", self._diagnose).pack(side="right", padx=(0, 14))
        Link(bar, "Log", self._open_log).pack(side="right", padx=(0, 14))

        # medir las dos alturas para que el panel avanzado no rompa el layout
        self.adv.pack(fill="x")
        self.update_idletasks()
        self._full_h = self.winfo_reqheight()
        self.adv.pack_forget()
        self.update_idletasks()
        self._base_h = self.winfo_reqheight()
        self.geometry(f"580x{self._base_h}")
        if self.cfg.get("advanced"):
            self._toggle_advanced()

        self.update_idletasks()
        dark_titlebar(self)
        self.after(250, lambda: dark_titlebar(self))
        self.nick_entry.focus()
        self.bind("<Return>", lambda _e: self.launch())
        self.protocol("WM_DELETE_WINDOW", self._on_close)

    def _toggle_advanced(self) -> None:
        self._advanced_open = not self._advanced_open
        if self._advanced_open:
            self.adv.pack(fill="x")
            self.adv_link.configure(text="Ocultar avanzado")
            self.geometry(f"580x{self._full_h}")
        else:
            self.adv.pack_forget()
            self.adv_link.configure(text="Avanzado")
            self.geometry(f"580x{self._base_h}")
        self.cfg["advanced"] = self._advanced_open
        try:
            self._save_cfg()
        except Exception:
            pass

    def _ram_label(self, _v=None) -> None:
        self.ram_lbl.configure(text=f"{int(self.ram_var.get())} MB")

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

    def _set_status(self, text: str, color: str = MUTED) -> None:
        self.status.configure(text=text, fg=color)
        self.update_idletasks()

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
            self._info("Nick invalido",
                       "Usa de 1 a 16 caracteres: A-Z, 0-9 o _.", kind="error")
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

        self.launch_btn.set_text("Lunar abierto")
        self._set_status(f"Corriendo · PID {self.proc.pid}", GREEN)
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
                self._set_status(f"Corriendo · PID {self.proc.pid}", GREEN)
            self.after(3000, self._watch)
        else:
            if self.pending_restore:
                accounts_restore()
                self.pending_restore = False
            self.launch_btn.set_text("Jugar con Lunar")
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
