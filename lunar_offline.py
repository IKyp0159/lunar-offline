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

def accounts_restore() -> bool:
    if ORIGINAL_ACCOUNTS.is_file():
        target = ACCOUNTS
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ORIGINAL_ACCOUNTS, target)
        ORIGINAL_ACCOUNTS.unlink(missing_ok=True)
        SWAP_MARKER.unlink(missing_ok=True)
        return True
    return False


def accounts_swap(nick: str) -> None:
    ACCOUNTS.parent.mkdir(parents=True, exist_ok=True)

    # Si quedo un swap sin restaurar de una sesion anterior, no sobrescribimos
    # el original bueno con la cuenta offline.
    if not SWAP_MARKER.exists() and ACCOUNTS.is_file():
        shutil.copy2(ACCOUNTS, ORIGINAL_ACCOUNTS)
    SWAP_MARKER.write_text(str(int(time.time())), encoding="utf-8")

    local_id = uuid.uuid4().hex
    profile_id = offline_uuid(nick)
    payload = {
        "activeAccountLocalId": local_id,
        "accounts": {
            local_id: {
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
        },
    }
    ACCOUNTS.write_text(json.dumps(payload, indent=2), encoding="utf-8")


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

class Launcher(tk.Tk):
    def __init__(self) -> None:
        super().__init__()
        self.title("Lunar Offline Launcher")
        self.geometry("640x600")
        self.minsize(580, 560)
        self.configure(bg="#141418")

        self.cfg = self._load_cfg()
        self.proc: subprocess.Popen | None = None
        self.log_path: Path | None = None
        self.pending_restore = False

        # recuperacion de un cierre sucio
        if SWAP_MARKER.exists() or ORIGINAL_ACCOUNTS.is_file():
            accounts_restore()

        self._styles()
        self._build()

    # ---------------- persistence ----------------

    def _load_cfg(self) -> dict:
        base = {
            "nick": "", "version": "1.8.9", "ram": 3072, "server": "",
            "width": 1280, "height": 720, "fullscreen": False, "java": "",
        }
        if CONFIG.is_file():
            try:
                base.update(json.loads(CONFIG.read_text(encoding="utf-8")))
            except Exception:
                pass
        return base

    def _save_cfg(self) -> None:
        CONFIG.write_text(json.dumps(self.cfg, indent=2), encoding="utf-8")

    # ---------------- styles ----------------

    def _styles(self) -> None:
        st = ttk.Style(self)
        try:
            st.theme_use("clam")
        except tk.TclError:
            pass
        st.configure(".", background="#141418", foreground="#e8e8ee",
                     fieldbackground="#1e1e26", bordercolor="#2c2c38")
        st.configure("TLabel", background="#141418", foreground="#c9c9d6")
        st.configure("H1.TLabel", font=("Segoe UI", 16, "bold"), foreground="#ffffff")
        st.configure("Hint.TLabel", foreground="#7b7b90", font=("Segoe UI", 8))
        st.configure("Warn.TLabel", foreground="#e0a030", font=("Segoe UI", 8))
        st.configure("TEntry", fieldbackground="#1e1e26", foreground="#ffffff",
                     insertcolor="#ffffff", borderwidth=1)
        st.configure("TCombobox", fieldbackground="#1e1e26", foreground="#ffffff",
                     background="#1e1e26")
        st.configure("Launch.TButton", font=("Segoe UI", 11, "bold"),
                     background="#5b8cff", foreground="#0a0a0f", borderwidth=0, padding=10)
        st.map("Launch.TButton", background=[("active", "#7aa2ff"), ("disabled", "#33333f")])
        st.configure("Tool.TButton", background="#23232d", foreground="#d6d6e2",
                     borderwidth=0, padding=6)
        st.map("Tool.TButton", background=[("active", "#2e2e3b")])
        st.configure("TCheckbutton", background="#141418", foreground="#c9c9d6")

    # ---------------- layout ----------------

    def _build(self) -> None:
        head = ttk.Frame(self)
        head.pack(fill="x", padx=22, pady=(20, 4))
        ttk.Label(head, text="Lunar Client — modo offline", style="H1.TLabel").pack(anchor="w")
        ttk.Label(head, text="Lunar completo: mods, FPS, keystrokes. Nick no premium.",
                  style="Hint.TLabel").pack(anchor="w", pady=(2, 0))

        body = ttk.Frame(self)
        body.pack(fill="both", expand=True, padx=22, pady=6)
        body.columnconfigure(1, weight=1)

        row = 0

        def label(text: str) -> None:
            nonlocal row
            ttk.Label(body, text=text).grid(row=row, column=0, sticky="w", pady=7, padx=(0, 12))

        label("Nick")
        self.nick_var = tk.StringVar(value=self.cfg.get("nick") or "Player")
        self.nick_entry = ttk.Entry(body, textvariable=self.nick_var, font=("Consolas", 11))
        self.nick_entry.grid(row=row, column=1, sticky="ew", pady=7)
        row += 1
        ttk.Label(body, text="1-16 caracteres: A-Z, 0-9, _", style="Hint.TLabel") \
            .grid(row=row, column=1, sticky="w")
        row += 1

        label("Version")
        self.version_var = tk.StringVar(value=self.cfg.get("version", "1.8.9"))
        vers = available_versions() or ["1.8.9"]
        if self.version_var.get() not in vers:
            self.version_var.set(vers[0])
        ttk.Combobox(body, textvariable=self.version_var, values=vers,
                     state="readonly").grid(row=row, column=1, sticky="ew", pady=7)
        row += 1

        label("Servidor")
        self.server_var = tk.StringVar(value=self.cfg.get("server", ""))
        ttk.Entry(body, textvariable=self.server_var).grid(row=row, column=1, sticky="ew", pady=7)
        row += 1
        ttk.Label(body, text="host o host:puerto. Vacio = menu principal.",
                  style="Hint.TLabel").grid(row=row, column=1, sticky="w")
        row += 1

        label("RAM (MB)")
        ram_frame = ttk.Frame(body)
        ram_frame.grid(row=row, column=1, sticky="ew", pady=7)
        ram_frame.columnconfigure(0, weight=1)
        self.ram_var = tk.IntVar(value=int(self.cfg.get("ram", 3072)))
        ttk.Scale(ram_frame, from_=1024, to=8192, variable=self.ram_var,
                  command=lambda v: self._ram_label(v)).grid(row=0, column=0, sticky="ew")
        self.ram_lbl = ttk.Label(ram_frame, text=f"{self.ram_var.get()} MB", width=9)
        self.ram_lbl.grid(row=0, column=1, padx=(10, 0))
        row += 1

        label("Resolucion")
        res = ttk.Frame(body)
        res.grid(row=row, column=1, sticky="ew", pady=7)
        self.w_var = tk.StringVar(value=str(self.cfg.get("width", 1280)))
        self.h_var = tk.StringVar(value=str(self.cfg.get("height", 720)))
        ttk.Entry(res, textvariable=self.w_var, width=7).grid(row=0, column=0)
        ttk.Label(res, text="x").grid(row=0, column=1, padx=6)
        ttk.Entry(res, textvariable=self.h_var, width=7).grid(row=0, column=2)
        self.fs_var = tk.BooleanVar(value=bool(self.cfg.get("fullscreen", False)))
        ttk.Checkbutton(res, text="Pantalla completa", variable=self.fs_var) \
            .grid(row=0, column=3, padx=(18, 0))
        row += 1

        ttk.Separator(body, orient="horizontal").grid(row=row, column=0, columnspan=2,
                                                      sticky="ew", pady=12)
        row += 1

        label("Java")
        jf = ttk.Frame(body)
        jf.grid(row=row, column=1, sticky="ew", pady=7)
        jf.columnconfigure(0, weight=1)
        self.java_var = tk.StringVar(value=self.cfg.get("java") or (find_java() or ""))
        ttk.Entry(jf, textvariable=self.java_var).grid(row=0, column=0, sticky="ew")
        ttk.Button(jf, text="...", width=3, style="Tool.TButton",
                   command=self._pick_java).grid(row=0, column=1, padx=(6, 0))
        row += 1

        ttk.Label(body, text="La cuenta de Lunar se cambia por una offline mientras juegas "
                             "y se restaura al salir.",
                  style="Warn.TLabel").grid(row=row, column=0, columnspan=2, sticky="w", pady=(4, 0))

        self.launch_btn = ttk.Button(self, text="JUGAR CON LUNAR", style="Launch.TButton",
                                     command=self.launch)
        self.launch_btn.pack(fill="x", padx=22, pady=(12, 6))

        foot = ttk.Frame(self)
        foot.pack(fill="x", padx=22, pady=(0, 14))
        ttk.Button(foot, text="Ver log", style="Tool.TButton",
                   command=self._open_log).pack(side="left")
        ttk.Button(foot, text="Diagnostico", style="Tool.TButton",
                   command=self._diagnose).pack(side="left", padx=(6, 0))
        ttk.Button(foot, text="Restaurar cuenta", style="Tool.TButton",
                   command=self._restore).pack(side="left", padx=(6, 0))
        self.status = ttk.Label(foot, text="Listo.", style="Hint.TLabel")
        self.status.pack(side="right")

        self.nick_entry.focus_set()
        self.bind("<Return>", lambda _e: self.launch())
        self.protocol("WM_DELETE_WINDOW", self._on_close)

    def _ram_label(self, v) -> None:
        self.ram_lbl.configure(text=f"{int(float(v))} MB")

    def _pick_java(self) -> None:
        path = filedialog.askopenfilename(
            title="Selecciona javaw.exe",
            filetypes=[("Java", "javaw.exe java.exe"), ("Todos", "*.*")],
        )
        if path:
            self.java_var.set(path)

    # ---------------- actions ----------------

    def _set_status(self, text: str) -> None:
        self.status.configure(text=text)
        self.update_idletasks()

    def _restore(self) -> None:
        if accounts_restore():
            messagebox.showinfo("Cuenta", "accounts.json restaurado al original.")
        else:
            messagebox.showinfo("Cuenta", "No hay nada que restaurar.")

    def _diagnose(self) -> None:
        java = self.java_var.get().strip()
        natives = find_natives()
        lines = [
            f"Lunar root: {LUNAR_ROOT}  -> {'OK' if LUNAR_ROOT.is_dir() else 'FALTA'}",
            f"Multiver:   {MULTIVER}  -> {'OK' if MULTIVER.is_dir() else 'FALTA'}",
            f"Java:       {java or 'NO ENCONTRADO'}  -> {'OK' if java and Path(java).is_file() else 'FALTA'}",
            f"Natives:    {natives or 'NO ENCONTRADO'}",
            f"Minecraft:  {MC_DIR}  -> {'OK' if MC_DIR.is_dir() else 'FALTA'}",
            f"Cuentas:    {ACCOUNTS}  -> {'OK' if ACCOUNTS.is_file() else 'FALTA'}",
            "",
            "Versiones listas: " + (", ".join(available_versions()) or "ninguna"),
        ]
        for v in available_versions():
            missing = [j for j in classpath_jars(v) if not (MULTIVER / j).is_file()]
            lines.append(f"  {v}: {len(classpath_jars(v))} jars" +
                         (f", FALTAN {missing}" if missing else ""))
        messagebox.showinfo("Diagnostico", "\n".join(lines))

    def _open_log(self) -> None:
        if self.log_path and self.log_path.is_file():
            subprocess.Popen(["notepad.exe", str(self.log_path)])
        else:
            messagebox.showinfo("Log", "Todavia no lanzaste el juego en esta sesion.")

    def launch(self) -> None:
        if self.proc and self.proc.poll() is None:
            messagebox.showinfo("Ya corriendo", "Lunar ya esta abierto.")
            return

        nick = self.nick_var.get().strip()
        if not NICK_RE.match(nick):
            messagebox.showerror("Nick invalido",
                                 "El nick debe tener 1-16 caracteres: A-Z, 0-9 o _.")
            return

        java = self.java_var.get().strip()
        if not java or not Path(java).is_file():
            messagebox.showerror("Java", "No encuentro un javaw.exe valido. Usa el boton '...'.")
            return

        try:
            ram = int(self.ram_var.get())
            width = int(self.w_var.get())
            height = int(self.h_var.get())
        except ValueError:
            messagebox.showerror("Valores", "RAM y resolucion deben ser numeros.")
            return

        stamp = time.strftime("%Y%m%d-%H%M%S")
        self.log_path = LOGS / f"run-{stamp}.log"

        try:
            cmd = build_command(nick, self.version_var.get(), ram,
                                self.server_var.get(), width, height,
                                self.fs_var.get(), java)
        except Exception as exc:
            messagebox.showerror("Error", str(exc))
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
            messagebox.showerror("Cuentas", f"No pude escribir accounts.json:\n{exc}")
            return

        self.pending_restore = True

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
            messagebox.showerror("No arranco", str(exc))
            return

        self._set_status(f"Corriendo (PID {self.proc.pid})")
        self.after(4000, self._watch)

    def _watch(self) -> None:
        if self.proc is None:
            return
        rc = self.proc.poll()
        if rc is None:
            self._set_status(f"Corriendo (PID {self.proc.pid})")
            self.after(3000, self._watch)
        else:
            if self.pending_restore:
                accounts_restore()
                self.pending_restore = False
            self._set_status(f"Cerrado (codigo {rc}) — cuenta restaurada")

    def _on_close(self) -> None:
        if self.proc and self.proc.poll() is None:
            if not messagebox.askyesno("Lunar abierto",
                                       "El juego sigue corriendo. Cerrar la app igual?\n"
                                       "La cuenta se restaura cuando el juego termine."):
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
        f"swap_pending: {SWAP_MARKER.exists()}",
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
