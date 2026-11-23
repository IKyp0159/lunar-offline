/// Textos de la interfaz en español, con el inglés preparado al lado.
///
/// La app arranca en español; `cambiarIdioma("en")` no necesita tocar ningún
/// componente, solo el diccionario.

export type Idioma = "es" | "en";

export interface Textos {
  titulo: string;
  subtitulo: string;
  jugar: string;
  jugando: string;
  preparando: string;
  restaurando: string;
  detener: string;
  cuenta: string;
  version: string;
  servidor: string;
  ram: string;
  resolucion: string;
  pantallaCompleta: string;
  avanzado: string;
  java: string;
  javaAuto: string;
  guardar: string;
  guardado: string;
  consola: string;
  consolaVacia: string;
  diagnostico: string;
  copiar: string;
  copiado: string;
  restaurar: string;
  sinRestaurar: string;
  entornoIncompleto: string;
  launcherOficial: string;
  launcherCerrado: string;
  versiones: string;
  sinVersiones: string;
  errorNick: string;
  errorRam: string;
  errorResolucion: string;

  navJugar: string;
  navPerfiles: string;
  navServidores: string;
  navRegistro: string;

  jugarTitulo: string;
  jugarSubtitulo: string;
  sinServidor: string;
  instanciaPid: string;

  perfilesTitulo: string;
  perfilesSubtitulo: string;
  perfilesVacio: string;
  perfilNuevo: string;
  perfilDuplicar: string;
  perfilBorrar: string;
  perfilActivar: string;
  perfilActivo: string;
  perfilRenombrar: string;
  perfilBorrarPrincipal: string;
  perfilUltimoUso: string;
  nombrePerfil: string;
  sinNick: string;
  nunca: string;

  servidoresTitulo: string;
  servidoresSubtitulo: string;
  servidoresVacio: string;
  servidorNuevo: string;
  servidorEditar: string;
  servidorNombre: string;
  servidorDireccion: string;
  servidorPuerto: string;
  servidorFavorito: string;
  servidorGuardar: string;
  servidorCancelar: string;
  usarServidor: string;
  sinVersion: string;
  importado: string;
  puertoInvalido: string;
  direccionVacia: string;

  instancias: string;
  instanciasVacias: string;
  notaNickFijo: string;
  lanzarOtra: string;
  cuentaActiva: string;
  editarEnJugar: string;
  listoParaJugar: string;

  registroTitulo: string;
  registroSubtitulo: string;
  abrirLogs: string;
  rutas: string;
  avisosSalud: string;
  sinAvisosSalud: string;
  copiarRuta: string;
  rutaCopiada: string;

  buscar: string;
  vacio: string;
  cargando: string;
  cancelar: string;
  borrar: string;
  editar: string;
  guardarCambios: string;
  confirmarBorrar: string;
}

const ES: Textos = {
  titulo: "Lunar Offline",
  subtitulo: "Sesión sin premium · tu cuenta se restaura sola",
  jugar: "Jugar",
  jugando: "Jugando",
  preparando: "Preparando",
  restaurando: "Restaurando cuenta",
  detener: "Esperando al juego",
  cuenta: "Cuenta",
  version: "Versión",
  servidor: "Servidor",
  ram: "Memoria",
  resolucion: "Resolución",
  pantallaCompleta: "Pantalla completa",
  avanzado: "Avanzado",
  java: "Java",
  javaAuto: "Detección automática",
  guardar: "Guardar",
  guardado: "Guardado",
  consola: "Consola",
  consolaVacia: "Sin actividad todavía.",
  diagnostico: "Diagnóstico",
  copiar: "Copiar informe",
  copiado: "Informe copiado",
  restaurar: "Restaurar cuenta",
  sinRestaurar: "No había nada que restaurar.",
  entornoIncompleto: "Falta Java, natives o versiones",
  launcherOficial: "Cierra Lunar Client para jugar",
  launcherCerrado: "Launcher oficial cerrado",
  versiones: "Versiones instaladas",
  sinVersiones: "Ninguna versión instalada",
  errorNick: "Nick no válido: 1-16 letras, números o guion bajo",
  errorRam: "La memoria va de 1024 a 8192 MB",
  errorResolucion: "Resolución fuera de rango",

  navJugar: "Jugar",
  navPerfiles: "Perfiles",
  navServidores: "Servidores",
  navRegistro: "Registro",

  jugarTitulo: "Jugar",
  jugarSubtitulo: "Elige cuenta y entorno; el launcher hace el resto.",
  sinServidor: "Menú principal",
  instanciaPid: "PID",

  perfilesTitulo: "Perfiles",
  perfilesSubtitulo: "Cada perfil guarda su nick, versión, memoria y servidor.",
  perfilesVacio: "Todavía no hay perfiles.",
  perfilNuevo: "Nuevo perfil",
  perfilDuplicar: "Duplicar",
  perfilBorrar: "Borrar",
  perfilActivar: "Activar",
  perfilActivo: "Activo",
  perfilRenombrar: "Renombrar",
  perfilBorrarPrincipal:
    "Vas a borrar el perfil principal. No se puede deshacer; si era el último, quedará uno nuevo vacío.",
  perfilUltimoUso: "Último uso",
  nombrePerfil: "Nombre del perfil",
  sinNick: "Sin nick",
  nunca: "Nunca",

  servidoresTitulo: "Servidores",
  servidoresSubtitulo: "Lista propia; se sincroniza con servers.dat cuando tú quieras.",
  servidoresVacio: "Todavía no hay servidores guardados.",
  servidorNuevo: "Nuevo servidor",
  servidorEditar: "Editar servidor",
  servidorNombre: "Nombre",
  servidorDireccion: "Dirección",
  servidorPuerto: "Puerto",
  servidorFavorito: "Favorito",
  servidorGuardar: "Guardar servidor",
  servidorCancelar: "Cancelar",
  usarServidor: "Usar",
  sinVersion: "Cualquiera",
  importado: "servers.dat",
  puertoInvalido: "El puerto va de 1 a 65535",
  direccionVacia: "La dirección no puede estar vacía",

  instancias: "Instancias abiertas",
  instanciasVacias: "No hay ninguna instancia del juego abierta.",
  notaNickFijo: "El nick de una instancia ya arrancada no se puede cambiar; para otro nick, lanza otra instancia.",
  lanzarOtra: "Lanzar otra instancia",
  cuentaActiva: "Cuenta activa",
  editarEnJugar: "Editar en Jugar",
  listoParaJugar: "Listo para jugar",

  registroTitulo: "Registro",
  registroSubtitulo: "Salida del juego, avisos de salud y diagnóstico.",
  abrirLogs: "Abrir carpeta de logs",
  rutas: "Rutas",
  avisosSalud: "Avisos de salud",
  sinAvisosSalud: "Ningún aviso de salud.",
  copiarRuta: "Copiar ruta",
  rutaCopiada: "Ruta copiada",

  buscar: "Buscar",
  vacio: "Nada que mostrar.",
  cargando: "Cargando…",
  cancelar: "Cancelar",
  borrar: "Borrar",
  editar: "Editar",
  guardarCambios: "Guardar cambios",
  confirmarBorrar: "¿Seguro? Esta acción no se puede deshacer.",
};

const EN: Textos = {
  titulo: "Lunar Offline",
  subtitulo: "Offline session · your account is restored automatically",
  jugar: "Play",
  jugando: "Playing",
  preparando: "Preparing",
  restaurando: "Restoring account",
  detener: "Waiting for the game",
  cuenta: "Account",
  version: "Version",
  servidor: "Server",
  ram: "Memory",
  resolucion: "Resolution",
  pantallaCompleta: "Fullscreen",
  avanzado: "Advanced",
  java: "Java",
  javaAuto: "Detect automatically",
  guardar: "Save",
  guardado: "Saved",
  consola: "Console",
  consolaVacia: "No activity yet.",
  diagnostico: "Diagnostics",
  copiar: "Copy report",
  copiado: "Report copied",
  restaurar: "Restore account",
  sinRestaurar: "Nothing to restore.",
  entornoIncompleto: "Missing Java, natives or versions",
  launcherOficial: "Close Lunar Client to play",
  launcherCerrado: "Official launcher closed",
  versiones: "Installed versions",
  sinVersiones: "No versions installed",
  errorNick: "Invalid nick: 1-16 letters, digits or underscore",
  errorRam: "Memory must be between 1024 and 8192 MB",
  errorResolucion: "Resolution out of range",

  navJugar: "Play",
  navPerfiles: "Profiles",
  navServidores: "Servers",
  navRegistro: "Log",

  jugarTitulo: "Play",
  jugarSubtitulo: "Pick an account and environment; the launcher does the rest.",
  sinServidor: "Main menu",
  instanciaPid: "PID",

  perfilesTitulo: "Profiles",
  perfilesSubtitulo: "Each profile keeps its nick, version, memory and server.",
  perfilesVacio: "No profiles yet.",
  perfilNuevo: "New profile",
  perfilDuplicar: "Duplicate",
  perfilBorrar: "Delete",
  perfilActivar: "Activate",
  perfilActivo: "Active",
  perfilRenombrar: "Rename",
  perfilBorrarPrincipal:
    "You are about to delete the main profile. This cannot be undone; if it was the last one, a new empty profile will take its place.",
  perfilUltimoUso: "Last used",
  nombrePerfil: "Profile name",
  sinNick: "No nick",
  nunca: "Never",

  servidoresTitulo: "Servers",
  servidoresSubtitulo: "Your own list; it syncs with servers.dat when you ask.",
  servidoresVacio: "No saved servers yet.",
  servidorNuevo: "New server",
  servidorEditar: "Edit server",
  servidorNombre: "Name",
  servidorDireccion: "Address",
  servidorPuerto: "Port",
  servidorFavorito: "Favourite",
  servidorGuardar: "Save server",
  servidorCancelar: "Cancel",
  usarServidor: "Use",
  sinVersion: "Any",
  importado: "servers.dat",
  puertoInvalido: "Port must be between 1 and 65535",
  direccionVacia: "Address cannot be empty",

  instancias: "Open instances",
  instanciasVacias: "No game instance is running.",
  notaNickFijo: "The nick of a running instance cannot be changed; launch another instance for a different nick.",
  lanzarOtra: "Launch another instance",
  cuentaActiva: "Active account",
  editarEnJugar: "Edit in Play",
  listoParaJugar: "Ready to play",

  registroTitulo: "Log",
  registroSubtitulo: "Game output, health warnings and diagnostics.",
  abrirLogs: "Open logs folder",
  rutas: "Paths",
  avisosSalud: "Health warnings",
  sinAvisosSalud: "No health warnings.",
  copiarRuta: "Copy path",
  rutaCopiada: "Path copied",

  buscar: "Search",
  vacio: "Nothing to show.",
  cargando: "Loading…",
  cancelar: "Cancel",
  borrar: "Delete",
  editar: "Edit",
  guardarCambios: "Save changes",
  confirmarBorrar: "Are you sure? This cannot be undone.",
};

/// Diccionarios disponibles.
export const DICCIONARIOS: Record<Idioma, Textos> = { es: ES, en: EN };

/// Textos de un idioma (por defecto, español).
export const textos = (idioma: Idioma = "es"): Textos => DICCIONARIOS[idioma];
