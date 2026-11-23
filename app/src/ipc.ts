/// Tipos que cruzan el IPC y envoltorios tipados de `invoke`.
///
/// Los nombres vienen de los DTO de `app/src/main.rs`; si cambian allí, cambian
/// aquí y TypeScript lo detecta en la compilación.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/// Fase de la partida.
export type EstadoJuego =
  | { fase: "inactivo" }
  | { fase: "preparando" }
  | { fase: "jugando"; pid: number; nick: string; version: string }
  | { fase: "jugando_varios"; cantidad: number; nicks: string[] }
  | { fase: "restaurando" }
  | { fase: "error"; mensaje: string };

/// Preset de JVM.
export type JvmPreset = "lunar" | "low" | "high" | "custom" | "inherit";

/// Ajustes de JVM (`config.json`).
export interface JvmSettings {
  preset: JvmPreset;
  xms_mb: number | null;
  xmx_mb: number | null;
  extra_flags: string[];
}

/// Perfil de juego.
export interface Perfil {
  id: string;
  name: string;
  nick: string;
  version: string;
  ram: number;
  server: string;
  width: number;
  height: number;
  fullscreen: boolean;
  java: string;
  jvm: JvmSettings | null;
  created_at: string;
  last_used_at: string | null;
}

/// Servidor guardado.
export interface Servidor {
  id: string;
  name: string;
  address: string;
  port: number;
  version: string;
  favourite: boolean;
  last_connected: string | null;
  source: "manual" | "servers_dat";
  dat_key: string | null;
}

/// Versión instalada.
export interface Version {
  nombre: string;
  perfil: string;
  optifine: string;
}

/// Preferencias de la interfaz guardadas en `config.json`.
export interface UiState {
  last_tab: string;
  status_bar: boolean;
}

/// Configuración del launcher (`config.json`).
export interface Config {
  nick: string;
  version: string;
  ram: number;
  server: string;
  width: number;
  height: number;
  fullscreen: boolean;
  java: string;
  advanced: boolean;
  schema: number;
  active_profile: string;
  profiles: Perfil[];
  servers: Servidor[];
  jvm: JvmSettings;
  ui: UiState;
}

/// Entorno detectado.
export interface Entorno {
  java: string | null;
  natives: string | null;
  listo: boolean;
}

/// Rutas resueltas.
export interface Rutas {
  app: string;
  lunar: string;
  minecraft: string;
  logs: string;
  multiver: string;
}

/// Foto completa del launcher.
export interface Estado {
  version_app: string;
  estado: EstadoJuego;
  aviso: string | null;
  recuperacion: string | null;
  versiones: Version[];
  config: Config;
  instancias: Juego[];
  launcher_oficial: boolean;
  entorno: Entorno;
  rutas: Rutas;
}

/// Petición de arranque.
export interface Peticion {
  nick: string;
  version: string;
  servidor: string;
  ram_mb: number;
  ancho: number;
  alto: number;
  pantalla_completa: boolean;
  java: string | null;
}

/// Partida lanzada.
export interface Juego {
  pid: number;
  nick: string;
  version: string;
  log: string;
}

/// Aviso de salud del arranque.
export interface Aviso {
  mensaje: string;
  pista: string;
}

/// Pide la foto completa (abre el `Core` la primera vez).
export const pedirEstado = (): Promise<Estado> => invoke<Estado>("estado");

/// Guarda la configuración y devuelve la foto nueva.
export const guardarConfig = (config: Config): Promise<Estado> =>
  invoke<Estado>("guardar", { config });

/// Lanza el juego.
export const lanzarJuego = (peticion: Peticion): Promise<Juego> =>
  invoke<Juego>("lanzar", { peticion });

/// Restaura `accounts.json` si quedó a medias.
export const restaurarCuenta = (): Promise<string | null> =>
  invoke<string | null>("restaurar");

/// Informe de diagnóstico en texto.
export const pedirDiagnostico = (): Promise<string> =>
  invoke<string>("diagnostico");

/// Ruta absoluta de una carpeta del launcher.
export const rutaCarpeta = (cual: "logs" | "app"): Promise<string> =>
  invoke<string>("abrir_carpeta", { cual });

/// Escucha los cambios de fase.
export const alCambiarEstado = (
  manejador: (estado: EstadoJuego) => void,
): Promise<UnlistenFn> =>
  listen<EstadoJuego>("estado", (evento) => manejador(evento.payload));

/// Escucha las líneas del log del juego.
export const alRecibirLog = (
  manejador: (linea: string) => void,
): Promise<UnlistenFn> =>
  listen<string>("log", (evento) => manejador(evento.payload));

/// Escucha los avisos de salud.
export const alRecibirAviso = (
  manejador: (aviso: Aviso) => void,
): Promise<UnlistenFn> =>
  listen<Aviso>("aviso", (evento) => manejador(evento.payload));

// ---------------- perfiles ----------------

/// Perfiles guardados.
export const listarPerfiles = (): Promise<Perfil[]> =>
  invoke<Perfil[]>("perfiles_listar");

/// Crea un perfil vacío con ese nombre.
export const crearPerfil = (nombre: string): Promise<Estado> =>
  invoke<Estado>("perfil_crear", { nombre });

/// Duplica un perfil.
export const duplicarPerfil = (id: string): Promise<Estado> =>
  invoke<Estado>("perfil_duplicar", { id });

/// Borra un perfil.
export const borrarPerfil = (id: string): Promise<Estado> =>
  invoke<Estado>("perfil_borrar", { id });

/// Marca un perfil como activo.
export const activarPerfil = (id: string): Promise<Estado> =>
  invoke<Estado>("perfil_activar", { id });

/// Cambia el nombre visible de un perfil.
export const renombrarPerfil = (id: string, nombre: string): Promise<Estado> =>
  invoke<Estado>("perfil_renombrar", { id, nombre });

// ---------------- servidores ----------------

/// Servidores guardados.
export const listarServidores = (): Promise<Servidor[]> =>
  invoke<Servidor[]>("servidores_listar");

/// Añade o actualiza un servidor.
export const guardarServidor = (servidor: Servidor): Promise<Estado> =>
  invoke<Estado>("servidor_guardar", { servidor });

/// Borra un servidor.
export const borrarServidor = (id: string): Promise<Estado> =>
  invoke<Estado>("servidor_borrar", { id });


