//! Capa Tauri de la aplicación: comandos tipados y eventos hacia Svelte.
//!
//! Reglas:
//!
//! - Toda la lógica vive en [`lo_core::Core`]; aquí solo hay traducción a JSON.
//! - Los tipos de `lo-core` **no** se serializan directamente: se copian a DTOs
//!   con nombres de campo estables, para que el TypeScript no dependa de los
//!   nombres internos de Rust.
//! - El estado del juego se emite por eventos (`estado`, `log`, `aviso`) y
//!   también se puede consultar con el comando `estado` (la UI arranca con él).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Tauri 2 marca el build como producción solo si existe la feature
// `custom-protocol`; sin ella, incluso en release carga `devUrl`
// (http://localhost:1420) y la ventana queda en ERR_CONNECTION_REFUSED.
#[cfg(all(not(debug_assertions), not(feature = "custom-protocol")))]
compile_error!(
    "release sin la feature `custom-protocol`: la app cargaría devUrl en vez del frontend embebido. \
     Compila con `cargo build --release -p lunar-offline-app --features custom-protocol` \
     (lo hace scripts/empaquetar.ps1)."
);

mod plataforma;

use std::path::PathBuf;
use std::time::Duration;

use lo_accounts::Nick;
use lo_config::{Config, JvmSettings, Profile, Server};
use lo_core::{Core, CoreEvent, GameState, VERSION};
use lo_launch::{HealthIssue, LaunchEvent, LaunchRequest, RunningGame};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

/// Nombre del evento de estado.
const EVENTO_ESTADO: &str = "estado";
/// Nombre del evento de línea de log.
const EVENTO_LOG: &str = "log";
/// Nombre del evento de aviso de salud del arranque.
const EVENTO_AVISO: &str = "aviso";

/// Estado compartido de la aplicación.
struct AppState {
    /// `None` hasta el primer comando; abrir es asíncrono y barato.
    core: Mutex<Option<Core>>,
}

impl AppState {
    fn new() -> Self {
        Self {
            core: Mutex::new(None),
        }
    }
}

/// Fase en la que está la partida, tal cual la pinta la UI.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "fase", rename_all = "snake_case")]
enum EstadoJuegoDto {
    /// Nada en marcha.
    Inactivo,
    /// Guardando la cuenta y armando el comando.
    Preparando,
    /// El juego está corriendo.
    Jugando {
        /// PID del proceso Java.
        pid: u32,
        /// Nick con el que se juega.
        nick: String,
        /// Versión arrancada.
        version: String,
    },
    /// Hay varias instancias abiertas a la vez.
    JugandoVarios {
        /// Cuántas siguen vivas.
        cantidad: usize,
        /// Nicks, en orden de apertura.
        nicks: Vec<String>,
    },
    /// Cerrando el juego y restaurando `accounts.json`.
    Restaurando,
    /// Algo falló.
    Error {
        /// Mensaje ya listo para el usuario.
        mensaje: String,
    },
}

impl From<&GameState> for EstadoJuegoDto {
    fn from(estado: &GameState) -> Self {
        match estado {
            GameState::Idle => Self::Inactivo,
            GameState::Preparing => Self::Preparando,
            GameState::Running(juego) => Self::Jugando {
                pid: juego.pid,
                nick: juego.nick.clone(),
                version: juego.version.clone(),
            },
            GameState::RunningMultiple { count, nicks } => Self::JugandoVarios {
                cantidad: *count,
                nicks: nicks.clone(),
            },
            GameState::Restoring => Self::Restaurando,
            GameState::Error { message } => Self::Error {
                mensaje: message.clone(),
            },
        }
    }
}

/// Una versión instalada, tal cual se ofrece en el desplegable.
#[derive(Debug, Clone, Serialize)]
struct VersionDto {
    /// `1.8.9` / `1.7.10`.
    nombre: &'static str,
    /// Perfil que usa (`1.8` / `1.7`).
    perfil: &'static str,
    /// OptiFine externo asociado.
    optifine: &'static str,
}

/// Entorno detectado (JRE, natives).
#[derive(Debug, Clone, Serialize)]
struct EntornoDto {
    /// Ruta del `javaw.exe` elegido.
    java: Option<String>,
    /// Carpeta de natives elegida.
    natives: Option<String>,
    /// Hay todo lo necesario para arrancar.
    listo: bool,
}

/// Rutas que usa el launcher.
#[derive(Debug, Clone, Serialize)]
struct RutasDto {
    /// `%APPDATA%\LunarOffline`.
    app: String,
    /// `%USERPROFILE%\.lunarclient`.
    lunar: String,
    /// `%APPDATA%\.minecraft`.
    minecraft: String,
    /// `...\LunarOffline\logs`.
    logs: String,
    /// `...\.lunarclient\offline\multiver`.
    multiver: String,
}

/// Foto completa que la UI pide al arrancar.
#[derive(Debug, Clone, Serialize)]
struct EstadoDto {
    /// Versión de esta compilación.
    version_app: &'static str,
    /// Fase actual de la partida.
    estado: EstadoJuegoDto,
    /// Aviso de la última carga de configuración.
    aviso: Option<String>,
    /// Restauración hecha al abrir por un cierre sucio.
    recuperacion: Option<String>,
    /// Versiones instaladas.
    versiones: Vec<VersionDto>,
    /// Configuración en vigor.
    config: Config,
    /// Instancias del juego vivas (para el cambio rápido).
    instancias: Vec<JuegoDto>,
    /// El launcher oficial está abierto.
    launcher_oficial: bool,
    /// Entorno detectado.
    entorno: EntornoDto,
    /// Rutas resueltas.
    rutas: RutasDto,
}

/// Petición de arranque que llega desde Svelte.
#[derive(Debug, Clone, Deserialize)]
struct PeticionDto {
    /// Nick no premium.
    nick: String,
    /// Versión instalada.
    version: String,
    /// Servidor (`host[:puerto]`, vacío = menú).
    servidor: String,
    /// RAM en MB.
    ram_mb: u32,
    /// Ancho de la ventana del juego.
    ancho: u32,
    /// Alto de la ventana del juego.
    alto: u32,
    /// Arrancar a pantalla completa.
    pantalla_completa: bool,
    /// Java elegido a mano (vacío = detectado).
    java: Option<String>,
}

impl PeticionDto {
    fn into_request(self) -> Result<LaunchRequest, String> {
        let nick = Nick::parse(&self.nick).map_err(|error| error.to_string())?;
        let java = self
            .java
            .filter(|ruta| !ruta.trim().is_empty())
            .map(PathBuf::from);
        Ok(LaunchRequest {
            nick,
            version: self.version,
            ram_mb: self.ram_mb,
            server: self.servidor,
            width: self.ancho,
            height: self.alto,
            fullscreen: self.pantalla_completa,
            java,
            jvm: None,
            jvm_global: None,
            jvm_confirm: false,
        })
    }
}

/// Partida viva devuelta por `lanzar`.
#[derive(Debug, Clone, Serialize)]
struct JuegoDto {
    /// PID del proceso Java.
    pid: u32,
    /// Nick con el que se juega.
    nick: String,
    /// Versión arrancada.
    version: String,
    /// Log que está siguiendo el supervisor.
    log: String,
}

impl From<&RunningGame> for JuegoDto {
    fn from(juego: &RunningGame) -> Self {
        Self {
            pid: juego.pid,
            nick: juego.nick.clone(),
            version: juego.version.clone(),
            log: juego.log_path.display().to_string(),
        }
    }
}

/// Aviso del validador de flags, ya listo para pintar.
#[derive(Debug, Clone, Serialize)]
struct JvmAvisoDto {
    /// Código estable (`jvm.blocked_flag`, …).
    codigo: String,
    /// Texto para el usuario.
    mensaje: String,
}

/// Vista previa del comando de JVM.
#[derive(Debug, Clone, Serialize)]
struct JvmPreviewDto {
    /// Línea `java …` con la memoria y los flags que se emitirían.
    comando: String,
    /// `-Xms` efectivo (MB), si lo hay.
    xms: Option<u32>,
    /// `-Xmx` efectivo (MB).
    xmx: u32,
    /// Flags descartados o pendientes de confirmación.
    avisos: Vec<JvmAvisoDto>,
}

/// Resultado de «Probar flags».
#[derive(Debug, Clone, Serialize)]
struct JvmPruebaDto {
    /// Versión que imprimió Java.
    version_java: String,
    /// Flags aceptados.
    aceptados: Vec<String>,
    /// Flags rechazados y el motivo.
    rechazados: Vec<JvmAvisoDto>,
    /// Duración de la prueba.
    ms: u64,
    /// Salida de `stderr`, recortada.
    stderr: String,
}

/// Resultado de una importación o exportación de servidores.
#[derive(Debug, Clone, Serialize)]
struct ServidoresResultadoDto {
    /// Texto corto para el aviso.
    resumen: String,
    /// Aviso si `servers.dat` no se pudo leer.
    aviso: Option<String>,
    /// Fichero tocado, si aplica.
    ruta: Option<String>,
}

/// Preset ofrecido por la UI.
#[derive(Debug, Clone, Serialize)]
struct JvmPresetDto {
    /// Identificador (`lunar`, `low`, …).
    id: String,
    /// Nombre visible.
    nombre: String,
    /// Qué hace, en una línea.
    descripcion: String,
}

fn avisos_de_jvm(warnings: &[lo_jvm::JvmWarning]) -> Vec<JvmAvisoDto> {
    warnings
        .iter()
        .map(|warning| JvmAvisoDto {
            codigo: warning.code.to_owned(),
            mensaje: warning.message.clone(),
        })
        .collect()
}

fn estado_dto(core: &Core) -> EstadoDto {
    let entorno = core.environment();
    EstadoDto {
        version_app: VERSION,
        estado: EstadoJuegoDto::from(&core.state()),
        aviso: core.notice().map(str::to_owned),
        recuperacion: core.recovery().map(describe_recovery),
        versiones: core
            .versions()
            .into_iter()
            .map(|spec| VersionDto {
                nombre: spec.name,
                perfil: spec.subversion,
                optifine: spec.external,
            })
            .collect(),
        config: core.config().clone(),
        instancias: core.instances().iter().map(JuegoDto::from).collect(),
        launcher_oficial: core.official_launcher_running(),
        entorno: EntornoDto {
            java: entorno
                .java
                .as_ref()
                .map(|java| java.path.display().to_string()),
            natives: entorno
                .natives
                .as_ref()
                .map(|natives| natives.path.display().to_string()),
            listo: entorno.ready(),
        },
        rutas: RutasDto {
            app: core.paths().app_dir().display().to_string(),
            lunar: core.paths().lunar_root().display().to_string(),
            minecraft: core.paths().minecraft_dir().display().to_string(),
            logs: core.paths().logs_dir().display().to_string(),
            multiver: core.paths().multiver().display().to_string(),
        },
    }
}

fn describe_recovery(outcome: &lo_accounts::RestoreOutcome) -> String {
    match outcome {
        lo_accounts::RestoreOutcome::NothingPending => "nada pendiente".to_owned(),
        lo_accounts::RestoreOutcome::Restored { removed, .. } => {
            format!(
                "restaurada la cuenta original ({} temporales retiradas)",
                removed.len()
            )
        }
        lo_accounts::RestoreOutcome::SessionEnded { remaining, .. } => {
            format!("sesión cerrada; quedan {remaining} abiertas")
        }
    }
}

/// Abre el `Core` una sola vez y engancha el reenvío de eventos a la UI.
async fn core_guard<'a>(
    state: &'a AppState,
    app: &AppHandle,
) -> Result<tokio::sync::MutexGuard<'a, Option<Core>>, String> {
    let mut guard = state.core.lock().await;
    if guard.is_none() {
        let core = Core::open().await.map_err(|error| error.to_string())?;
        let mut eventos = core.subscribe();
        let handle = app.clone();
        tokio::spawn(async move {
            while let Ok(evento) = eventos.recv().await {
                match evento {
                    CoreEvent::State(estado) => {
                        let _ = handle.emit(EVENTO_ESTADO, EstadoJuegoDto::from(&estado));
                    }
                    CoreEvent::Launch(LaunchEvent::LogLine(linea)) => {
                        let _ = handle.emit(EVENTO_LOG, linea);
                    }
                    CoreEvent::Launch(LaunchEvent::Warning(aviso)) => {
                        let _ = handle.emit(EVENTO_AVISO, aviso_dto(&aviso));
                    }
                    CoreEvent::Launch(LaunchEvent::Started(juego)) => {
                        let _ = handle.emit(
                            EVENTO_ESTADO,
                            EstadoJuegoDto::Jugando {
                                pid: juego.pid,
                                nick: juego.nick,
                                version: juego.version,
                            },
                        );
                    }
                    CoreEvent::Launch(LaunchEvent::Exited { .. }) => {}
                }
            }
        });
        *guard = Some(core);
    }
    Ok(guard)
}

#[derive(Debug, Clone, Serialize)]
struct AvisoDto {
    mensaje: String,
    pista: &'static str,
}

fn aviso_dto(aviso: &HealthIssue) -> AvisoDto {
    AvisoDto {
        mensaje: aviso.message(),
        pista: aviso.hint(),
    }
}

/// Foto completa del launcher (configuración, entorno, estado).
#[tauri::command]
async fn estado(state: State<'_, AppState>, app: AppHandle) -> Result<EstadoDto, String> {
    let guard = core_guard(&state, &app).await?;
    let core = guard.as_ref().ok_or("el launcher no está listo")?;
    Ok(estado_dto(core))
}

/// Guarda la configuración y devuelve la foto nueva.
#[tauri::command]
async fn guardar(
    state: State<'_, AppState>,
    app: AppHandle,
    config: Config,
) -> Result<EstadoDto, String> {
    let mut guard = core_guard(&state, &app).await?;
    let core = guard.as_mut().ok_or("el launcher no está listo")?;
    core.save_config(config)
        .await
        .map_err(|error| error.to_string())?;
    Ok(estado_dto(core))
}

/// Arranca el juego con la sesión offline.
#[tauri::command]
async fn lanzar(
    state: State<'_, AppState>,
    app: AppHandle,
    peticion: PeticionDto,
) -> Result<JuegoDto, String> {
    let mut guard = core_guard(&state, &app).await?;
    let core = guard.as_mut().ok_or("el launcher no está listo")?;
    let request = peticion.into_request()?;
    let juego = core
        .launch(request)
        .await
        .map_err(|error| error.to_string())?;
    Ok(JuegoDto::from(&juego))
}

/// Restaura `accounts.json` si quedó un swap a medias.
#[tauri::command]
async fn restaurar(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let mut guard = state.core.lock().await;
    let Some(core) = guard.as_mut() else {
        return Ok(None);
    };
    let outcome = core
        .restore_pending()
        .await
        .map_err(|error| error.to_string())?;
    Ok(outcome.as_ref().map(describe_recovery))
}

/// Lista de perfiles guardados.
#[tauri::command]
async fn perfiles_listar(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Vec<Profile>, String> {
    let guard = core_guard(&state, &app).await?;
    let core = guard.as_ref().ok_or("el launcher no está listo")?;
    Ok(core.config().profiles.clone())
}

/// Crea un perfil y devuelve la foto nueva.
#[tauri::command]
async fn perfil_crear(
    state: State<'_, AppState>,
    app: AppHandle,
    nombre: String,
) -> Result<EstadoDto, String> {
    let mut guard = core_guard(&state, &app).await?;
    let core = guard.as_mut().ok_or("el launcher no está listo")?;
    let mut config = core.config().clone();
    config
        .add_profile(nombre.trim())
        .map_err(|error| error.to_string())?;
    core.save_config(config)
        .await
        .map_err(|error| error.to_string())?;
    Ok(estado_dto(core))
}

/// Duplica un perfil por identificador.
#[tauri::command]
async fn perfil_duplicar(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> Result<EstadoDto, String> {
    let mut guard = core_guard(&state, &app).await?;
    let core = guard.as_mut().ok_or("el launcher no está listo")?;
    let mut config = core.config().clone();
    config
        .duplicate_profile(&id)
        .map_err(|error| error.to_string())?;
    core.save_config(config)
        .await
        .map_err(|error| error.to_string())?;
    Ok(estado_dto(core))
}

/// Borra un perfil por identificador.
#[tauri::command]
async fn perfil_borrar(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> Result<EstadoDto, String> {
    let mut guard = core_guard(&state, &app).await?;
    let core = guard.as_mut().ok_or("el launcher no está listo")?;
    let mut config = core.config().clone();
    config
        .remove_profile(&id)
        .map_err(|error| error.to_string())?;
    core.save_config(config)
        .await
        .map_err(|error| error.to_string())?;
    Ok(estado_dto(core))
}

/// Marca un perfil como activo.
#[tauri::command]
async fn perfil_activar(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> Result<EstadoDto, String> {
    let mut guard = core_guard(&state, &app).await?;
    let core = guard.as_mut().ok_or("el launcher no está listo")?;
    let mut config = core.config().clone();
    config
        .select_profile(&id)
        .map_err(|error| error.to_string())?;
    core.save_config(config)
        .await
        .map_err(|error| error.to_string())?;
    Ok(estado_dto(core))
}

/// Cambia el nombre visible de un perfil.
#[tauri::command]
async fn perfil_renombrar(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
    nombre: String,
) -> Result<EstadoDto, String> {
    let limpio = nombre.trim();
    if limpio.is_empty() {
        return Err("el nombre del perfil no puede estar vacío".to_owned());
    }
    let mut guard = core_guard(&state, &app).await?;
    let core = guard.as_mut().ok_or("el launcher no está listo")?;
    let mut config = core.config().clone();
    let perfil = config
        .profiles
        .iter_mut()
        .find(|perfil| perfil.id == id)
        .ok_or("ese perfil no existe")?;
    limpio.clone_into(&mut perfil.name);
    core.save_config(config)
        .await
        .map_err(|error| error.to_string())?;
    Ok(estado_dto(core))
}

/// Lista de servidores guardados.
#[tauri::command]
async fn servidores_listar(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Vec<Server>, String> {
    let guard = core_guard(&state, &app).await?;
    let core = guard.as_ref().ok_or("el launcher no está listo")?;
    Ok(core.config().servers.clone())
}

/// Añade o actualiza un servidor.
#[tauri::command]
async fn servidor_guardar(
    state: State<'_, AppState>,
    app: AppHandle,
    servidor: Server,
) -> Result<EstadoDto, String> {
    let mut guard = core_guard(&state, &app).await?;
    let core = guard.as_mut().ok_or("el launcher no está listo")?;
    let mut config = core.config().clone();
    config
        .upsert_server(servidor)
        .map_err(|error| error.to_string())?;
    core.save_config(config)
        .await
        .map_err(|error| error.to_string())?;
    Ok(estado_dto(core))
}

/// Borra un servidor por identificador.
#[tauri::command]
async fn servidor_borrar(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> Result<EstadoDto, String> {
    let mut guard = core_guard(&state, &app).await?;
    let core = guard.as_mut().ok_or("el launcher no está listo")?;
    let mut config = core.config().clone();
    config
        .remove_server(&id)
        .map_err(|error| error.to_string())?;
    core.save_config(config)
        .await
        .map_err(|error| error.to_string())?;
    Ok(estado_dto(core))
}

/// Lleva a `config.json` lo que haya en `servers.dat` (nunca borra).
#[tauri::command]
async fn servidores_importar_dat(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<ServidoresResultadoDto, String> {
    let mut guard = core_guard(&state, &app).await?;
    let core = guard.as_mut().ok_or("el launcher no está listo")?;
    let ruta = lo_servers::dat_path(core.paths());
    let dat = lo_servers::ServersDat::read(&ruta).map_err(|error| error.to_string())?;
    let aviso = dat.notice().map(str::to_owned);
    let mut config = core.config().clone();
    let reporte = dat.merge_into_config(&mut config.servers);
    if !matches!(reporte, lo_servers::MergeReport::Unchanged) {
        core.save_config(config)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(ServidoresResultadoDto {
        resumen: reporte.summary(),
        aviso,
        ruta: Some(ruta.display().to_string()),
    })
}

/// Lleva a `servers.dat` lo que haya en `config.json` (nunca borra).
#[tauri::command]
async fn servidores_exportar_dat(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<ServidoresResultadoDto, String> {
    let guard = core_guard(&state, &app).await?;
    let core = guard.as_ref().ok_or("el launcher no está listo")?;
    let ruta = lo_servers::dat_path(core.paths());
    let mut dat = lo_servers::ServersDat::read(&ruta).map_err(|error| error.to_string())?;
    let reporte = dat
        .merge_from_config(&core.config().servers)
        .map_err(|error| error.to_string())?;
    if !matches!(reporte, lo_servers::MergeReport::Unchanged) {
        dat.save().map_err(|error| error.to_string())?;
    }
    Ok(ServidoresResultadoDto {
        resumen: reporte.summary(),
        aviso: dat.notice().map(str::to_owned),
        ruta: Some(ruta.display().to_string()),
    })
}

/// Importa los servidores de `%APPDATA%\LunarOffline\servers.json`.
#[tauri::command]
async fn servidores_importar_json(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<ServidoresResultadoDto, String> {
    let mut guard = core_guard(&state, &app).await?;
    let core = guard.as_mut().ok_or("el launcher no está listo")?;
    let ruta = core.paths().app_dir().join("servers.json");
    let importados = lo_servers::import_json(&ruta).map_err(|error| error.to_string())?;
    let mut config = core.config().clone();
    let mut anadidos = 0usize;
    let mut repetidos = 0usize;
    for mut servidor in importados {
        let existe = config.servers.iter().any(|actual| {
            actual.address.eq_ignore_ascii_case(&servidor.address) && actual.port == servidor.port
        });
        if existe {
            repetidos += 1;
            continue;
        }
        servidor.id.clear();
        config
            .upsert_server(servidor)
            .map_err(|error| error.to_string())?;
        anadidos += 1;
    }
    if anadidos > 0 {
        core.save_config(config)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(ServidoresResultadoDto {
        resumen: format!("{anadidos} importados, {repetidos} repetidos"),
        aviso: None,
        ruta: Some(ruta.display().to_string()),
    })
}

/// Exporta la lista a `%APPDATA%\LunarOffline\servers.json`.
#[tauri::command]
async fn servidores_exportar_json(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<ServidoresResultadoDto, String> {
    let guard = core_guard(&state, &app).await?;
    let core = guard.as_ref().ok_or("el launcher no está listo")?;
    let ruta = core.paths().app_dir().join("servers.json");
    lo_servers::export_json(&ruta, &core.config().servers).map_err(|error| error.to_string())?;
    Ok(ServidoresResultadoDto {
        resumen: format!("{} servidores exportados", core.config().servers.len()),
        aviso: None,
        ruta: Some(ruta.display().to_string()),
    })
}

/// Presets de JVM que ofrece el launcher.
#[tauri::command]
fn jvm_presets() -> Vec<JvmPresetDto> {
    lo_jvm::presets()
        .iter()
        .map(|preset| JvmPresetDto {
            id: preset.id.as_str().to_owned(),
            nombre: preset.name.to_owned(),
            descripcion: preset.description.to_owned(),
        })
        .collect()
}

/// Guarda la JVM global (`config.jvm`).
#[tauri::command]
async fn jvm_guardar(
    state: State<'_, AppState>,
    app: AppHandle,
    jvm: JvmSettings,
) -> Result<EstadoDto, String> {
    let mut guard = core_guard(&state, &app).await?;
    let core = guard.as_mut().ok_or("el launcher no está listo")?;
    let mut config = core.config().clone();
    config.jvm = jvm;
    core.save_config(config)
        .await
        .map_err(|error| error.to_string())?;
    Ok(estado_dto(core))
}

/// Guarda la JVM propia del perfil activo; `None` vuelve a la global.
#[tauri::command]
async fn jvm_perfil_guardar(
    state: State<'_, AppState>,
    app: AppHandle,
    jvm: Option<JvmSettings>,
) -> Result<EstadoDto, String> {
    let mut guard = core_guard(&state, &app).await?;
    let core = guard.as_mut().ok_or("el launcher no está listo")?;
    let mut config = core.config().clone();
    config.active_profile_mut().jvm = jvm;
    core.save_config(config)
        .await
        .map_err(|error| error.to_string())?;
    Ok(estado_dto(core))
}

/// Vista previa del comando de JVM con la RAM indicada.
#[tauri::command]
async fn jvm_preview(
    state: State<'_, AppState>,
    app: AppHandle,
    jvm: JvmSettings,
    ram_mb: u32,
) -> Result<JvmPreviewDto, String> {
    let guard = core_guard(&state, &app).await?;
    let core = guard.as_ref().ok_or("el launcher no está listo")?;
    let java = core
        .environment()
        .java
        .as_ref()
        .map_or_else(|| PathBuf::from("java"), |java| java.path.clone());
    let resolved = lo_jvm::ResolvedJvm::resolve(&jvm, &core.config().jvm, ram_mb);
    Ok(JvmPreviewDto {
        comando: resolved.preview(&java),
        xms: resolved.xms_mb(),
        xmx: resolved.xmx_mb(),
        avisos: avisos_de_jvm(resolved.warnings()),
    })
}

/// Prueba los flags con `java <flags> -version`.
#[tauri::command]
async fn jvm_probar(
    state: State<'_, AppState>,
    app: AppHandle,
    jvm: JvmSettings,
    ram_mb: u32,
) -> Result<JvmPruebaDto, String> {
    let (java, flags) = {
        let guard = core_guard(&state, &app).await?;
        let core = guard.as_ref().ok_or("el launcher no está listo")?;
        let java = core
            .environment()
            .java
            .as_ref()
            .map_or_else(|| PathBuf::from("java"), |java| java.path.clone());
        let resolved = lo_jvm::ResolvedJvm::resolve(&jvm, &core.config().jvm, ram_mb);
        (java, resolved.arguments())
    };
    let report = lo_jvm::probe_flags(&java, &flags, Duration::from_secs(20))
        .await
        .map_err(|error| error.to_string())?;
    Ok(JvmPruebaDto {
        version_java: report.java_version,
        aceptados: report.accepted,
        rechazados: report
            .rejected
            .into_iter()
            .map(|(flag, motivo)| JvmAvisoDto {
                codigo: "jvm.rejected_flag".to_owned(),
                mensaje: format!("«{flag}»: {motivo}"),
            })
            .collect(),
        ms: report.elapsed_ms,
        stderr: report.stderr,
    })
}

/// Informe de diagnóstico en texto plano.
#[tauri::command]
async fn diagnostico(state: State<'_, AppState>, app: AppHandle) -> Result<String, String> {
    let guard = core_guard(&state, &app).await?;
    let core = guard.as_ref().ok_or("el launcher no está listo")?;
    let report = core.diagnose().await.map_err(|error| error.to_string())?;
    Ok(report.render_text())
}

/// Abre un directorio del launcher en el Explorador de Windows.
///
/// Los argumentos van por valor porque Tauri los deserializa desde el IPC; no
/// se pueden tomar por referencia.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn abrir_carpeta(state: State<'_, AppState>, cual: String) -> Result<String, String> {
    let guard = state
        .core
        .try_lock()
        .map_err(|_| "ocupado; inténtalo otra vez")?;
    let core = guard.as_ref().ok_or("el launcher no está listo")?;
    let ruta = match cual.as_str() {
        "logs" => core.paths().logs_dir().to_path_buf(),
        "app" => core.paths().app_dir().to_path_buf(),
        _ => return Err(format!("carpeta desconocida: {cual}")),
    };
    std::process::Command::new("explorer.exe")
        .arg(&ruta)
        .spawn()
        .map_err(|error| format!("no pude abrir {}: {error}", ruta.display()))?;
    Ok(ruta.display().to_string())
}

fn main() -> std::process::ExitCode {
    // Un solo `.exe` para todo: si los argumentos piden CLI, se atiende sin
    // abrir ventana. La consola del padre se engancha porque en release el
    // binario es de subsistema Windows y no tiene una propia.
    if lo_cli::es_invocacion_cli() {
        plataforma::adjuntar_consola();
        if let Some(codigo) = lo_cli::run_from_env_blocking() {
            return codigo;
        }
    }

    // Sin WebView2 no hay ventana: aviso nativo con enlace antes de intentarlo.
    if !plataforma::webview2_listo() {
        return std::process::ExitCode::FAILURE;
    }

    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            estado,
            guardar,
            lanzar,
            restaurar,
            perfiles_listar,
            perfil_crear,
            perfil_duplicar,
            perfil_borrar,
            perfil_activar,
            perfil_renombrar,
            servidores_listar,
            servidor_guardar,
            servidor_borrar,
            servidores_importar_dat,
            servidores_exportar_dat,
            servidores_importar_json,
            servidores_exportar_json,
            jvm_presets,
            jvm_guardar,
            jvm_perfil_guardar,
            jvm_preview,
            jvm_probar,
            diagnostico,
            abrir_carpeta
        ])
        .run(tauri::generate_context!())
        .expect("error al arrancar la ventana");

    std::process::ExitCode::SUCCESS
}
