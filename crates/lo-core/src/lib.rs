//! Fachada del launcher: una sola API, asíncrona, para la UI y la CLI.
//!
//! Reglas:
//!
//! - Este crate no sabe de Tauri, de clap ni de HTTP. Solo compone los crates
//!   puros.
//! - **Lo primero que hace [`Core::open`] es recuperar un cierre sucio**: si hay
//!   `swap.pending` o `accounts.original.json`, restaura antes de enseñar nada.
//! - El estado de la partida es explícito ([`GameState`]) y se emite como
//!   [`CoreEvent`]; la UI no consulta el proceso, escucha.
//! - Ante un fallo de restauración, el launcher **no arranca**: [`CoreError`]
//!   se propaga con el fichero afectado, porque un `accounts.json` a medias es
//!   peor que un launcher parado.
//!
//! El ciclo de una partida:
//!
//! ```text
//! comprobar oficial cerrado -> escribir logs/cmd-<marca>.txt -> swap de
//! accounts.json -> Popen(javaw, cwd=multiver, stdout=log) -> vigilar el log
//! -> al salir, restaurar accounts.json y volver a Idle
//! ```
//!
//! Si el proceso no llega a arrancar, se restaura `accounts.json` antes de
//! devolver el error: no queda una cuenta temporal viva en el disco.

// Las firmas son `async` desde el primer día: la UI las llama igual y algunas
// harán E/S de verdad cuando el plan se construya en segundo plano.
#![allow(clippy::unused_async, clippy::unused_async_trait_impl)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lo_accounts::{AccountsManager, RestoreOutcome};
use lo_config::{Config, ConfigLoad};
use lo_detect::{Environment, VersionSpec};
use lo_diag::{DiagError, DiagnosticReport};
use lo_launch::{
    HealthIssue, LaunchEvent, LaunchPlan, LaunchRequest, RunningGame, Supervisor, WatchOptions,
};
use lo_paths::{LunarPaths, PathsError};
use thiserror::Error;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Versión de esta compilación, la que va al informe de soporte.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Cuántos sucesos guarda el canal para quien se conecte tarde.
const EVENT_BUFFER: usize = 256;

/// Cada cuánto se comprueba si el oficial se ha abierto mientras se juega.
const LAUNCHER_CHECK: Duration = Duration::from_secs(3);

/// Estado de la partida, tal cual la pinta la UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameState {
    /// Sin nada en marcha.
    Idle,
    /// Guardando la cuenta offline y armando el comando.
    Preparing,
    /// El juego está corriendo con la cuenta offline activa.
    Running(RunningGame),
    /// Hay varias instancias abiertas a la vez (cambio rápido sin cerrar).
    RunningMultiple {
        /// Cuántas siguen vivas.
        count: usize,
        /// Nicks de las instancias, en orden de apertura.
        nicks: Vec<String>,
    },
    /// Cerrando el juego y restaurando `accounts.json`.
    Restoring,
    /// Falló algo; el mensaje va en el idioma de la UI.
    Error {
        /// Mensaje ya traducido (es/en).
        message: String,
    },
}

impl GameState {
    /// ¿Hay una partida viva (o a punto de estarlo)?
    #[must_use]
    pub fn busy(&self) -> bool {
        !matches!(self, Self::Idle | Self::Error { .. })
    }
}

/// Launcher listo para usarse.
#[derive(Debug)]
pub struct Core {
    paths: LunarPaths,
    config: Config,
    notice: Option<String>,
    environment: Environment,
    state: Arc<Mutex<GameState>>,
    running: Arc<Mutex<Vec<RunningGame>>>,
    accounts: AccountsManager,
    probe: LauncherProbe,
    events: broadcast::Sender<CoreEvent>,
    recovery: Option<RestoreOutcome>,
    workers: Vec<JoinHandle<()>>,
}

/// Cómo se pregunta si el launcher oficial está abierto.
///
/// Se inyecta para poder lanzar aunque el equipo tenga Lunar Client abierto,
/// igual que [`lo_paths::EnvVars`] evita tocar el entorno.
#[derive(Clone)]
pub struct LauncherProbe(Arc<dyn Fn() -> bool + Send + Sync>);

impl LauncherProbe {
    /// Sonda a partir de una función cualquiera.
    pub fn new(check: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(check))
    }

    /// ¿Está abierto ahora mismo?
    #[must_use]
    pub fn running(&self) -> bool {
        (self.0)()
    }
}

impl std::fmt::Debug for LauncherProbe {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LauncherProbe")
    }
}

impl Core {
    /// Abre el launcher con las rutas del sistema.
    pub async fn open() -> Result<Self, CoreError> {
        Self::open_with(LunarPaths::detect()?).await
    }

    /// Abre el launcher sobre un conjunto de rutas dado.
    ///
    /// Antes de leer nada, restaura `accounts.json` si la ejecución anterior
    /// murió con el swap vivo.
    pub async fn open_with(paths: LunarPaths) -> Result<Self, CoreError> {
        let probe_for_check = AccountsManager::new(paths.clone());
        let probe = LauncherProbe::new(move || probe_for_check.official_launcher_running());
        Self::open_with_probe(paths, probe).await
    }

    /// Abre el launcher con una sonda del oficial a medida.
    pub async fn open_with_probe(
        paths: LunarPaths,
        probe: LauncherProbe,
    ) -> Result<Self, CoreError> {
        paths.ensure_dirs()?;
        let accounts = AccountsManager::new(paths.clone());
        let recovery = if accounts.pending() {
            Some(accounts.restore()?)
        } else {
            None
        };

        let load = Config::load(&paths)?;
        let config = load.config().clone();
        let notice = load.notice();
        let environment = lo_detect::detect(&paths);
        let (events, _) = broadcast::channel(EVENT_BUFFER);

        Ok(Self {
            paths,
            config,
            notice,
            environment,
            state: Arc::new(Mutex::new(GameState::Idle)),
            running: Arc::new(Mutex::new(Vec::new())),
            accounts,
            probe,
            events,
            recovery,
            workers: Vec::new(),
        })
    }

    /// Rutas resueltas.
    #[must_use]
    pub fn paths(&self) -> &LunarPaths {
        &self.paths
    }

    /// Configuración en vigor.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Aviso de la última carga de configuración (fichero corrupto, etc.).
    #[must_use]
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Restauración hecha al abrir por un cierre sucio, si la hubo.
    #[must_use]
    pub fn recovery(&self) -> Option<&RestoreOutcome> {
        self.recovery.as_ref()
    }

    /// Entorno detectado (JRE, natives, versiones).
    #[must_use]
    pub fn environment(&self) -> &Environment {
        &self.environment
    }

    /// Versiones instaladas y listas para lanzar.
    #[must_use]
    pub fn versions(&self) -> Vec<&'static VersionSpec> {
        self.environment.versions.clone()
    }

    /// Estado actual de la partida.
    #[must_use]
    pub fn state(&self) -> GameState {
        lock(&self.state).clone()
    }

    /// Canal de sucesos: estado y vida del proceso.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.events.subscribe()
    }

    /// Carga la configuración del disco y refresca el entorno.
    pub async fn reload(&mut self) -> Result<ConfigLoad, CoreError> {
        let load = Config::load(&self.paths)?;
        self.config = load.config().clone();
        self.notice = load.notice();
        self.environment = lo_detect::detect(&self.paths);
        Ok(load)
    }

    /// Guarda la configuración (escritura atómica).
    ///
    /// Lo que se guarda es la raíz, así que el perfil activo se sincroniza
    /// antes: al releer, la raíz manda y sin esto la copia en memoria diferiría
    /// de la del disco.
    pub async fn save_config(&mut self, mut config: Config) -> Result<(), CoreError> {
        config.sync_profile_from_root();
        config.save(&self.paths)?;
        self.notice = None;
        self.config = config;
        Ok(())
    }

    /// Construye el plan de arranque sin tocar cuentas ni lanzar nada.
    ///
    /// Es lo que usa el `launch` en seco de la CLI: valida versión, JRE,
    /// natives y artefactos, y devuelve el comando exacto.
    pub fn plan_for(&self, request: &LaunchRequest) -> Result<LaunchPlan, CoreError> {
        let mut request = request.clone();
        self.apply_config_jvm(&mut request);
        let spec = self.spec_for(&request.version)?;
        Ok(lo_launch::build_plan(
            &self.paths,
            &self.environment,
            spec,
            &request,
            &stamp_now(),
        )?)
    }

    /// Guarda la sesión offline y arranca el juego.
    ///
    /// Orden obligatorio: comprobar el oficial abierto → escribir
    /// `logs/cmd-<marca>.txt` → `swap.pending` + copia + swap → lanzar. Si el
    /// arranque falla, restaura antes de devolver el error.
    ///
    /// Se puede llamar con una partida ya en marcha: el swap se cuenta por
    /// referencias y `accounts.json` no se restaura hasta cerrar la última
    /// instancia. Solo se rechaza mientras se está preparando o restaurando otra.
    pub async fn launch(&mut self, mut request: LaunchRequest) -> Result<RunningGame, CoreError> {
        if matches!(self.state(), GameState::Preparing | GameState::Restoring) {
            return Err(CoreError::BadState(
                "hay otro arranque en curso; espera un momento",
            ));
        }
        // La JVM del perfil activo manda; la petición puede traerla ya resuelta.
        self.apply_config_jvm(&mut request);
        self.set_state(GameState::Preparing);

        match self.start_game(&request).await {
            Ok(game) => {
                self.set_state(state_for(&self.running));
                Ok(game)
            }
            Err(error) => {
                self.set_state(GameState::Error {
                    message: error.to_string(),
                });
                Err(error)
            }
        }
    }

    /// Instancias del juego vivas en este proceso, en orden de apertura.
    #[must_use]
    pub fn instances(&self) -> Vec<RunningGame> {
        lock(&self.running).clone()
    }

    /// Restaura `accounts.json` si había un swap pendiente.
    pub async fn restore_pending(&mut self) -> Result<Option<RestoreOutcome>, CoreError> {
        if self.state().busy() {
            return Err(CoreError::BadState(
                "el juego sigue abierto; la cuenta se restaura al salir",
            ));
        }
        if !self.accounts.pending() {
            return Ok(None);
        }
        Ok(Some(self.accounts.restore()?))
    }

    /// Genera el informe de diagnóstico (lo que hacía `--selftest`).
    pub async fn diagnose(&self) -> Result<DiagnosticReport, CoreError> {
        Ok(lo_diag::build(&self.paths)?)
    }

    /// ¿El launcher oficial está abierto ahora mismo?
    #[must_use]
    pub fn official_launcher_running(&self) -> bool {
        self.probe.running()
    }

    /// Guarda el comando exacto en `logs/cmd-<marca>.txt` (soporte).
    fn write_command_log(plan: &LaunchPlan) -> Result<(), CoreError> {
        if let Some(parent) = plan.command_log.parent() {
            std::fs::create_dir_all(parent).map_err(|source| CoreError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        lo_paths::write_atomic(&plan.command_log, plan.render().as_bytes()).map_err(|source| {
            CoreError::Io {
                path: plan.command_log.clone(),
                source,
            }
        })
    }

    fn spec_for(&self, version: &str) -> Result<&'static VersionSpec, CoreError> {
        self.environment
            .versions
            .iter()
            .find(|spec| spec.name == version)
            .copied()
            .ok_or_else(|| lo_launch::LaunchError::UnsupportedVersion(version.to_owned()).into())
    }

    /// Rellena la JVM con la del perfil activo si la petición no la trae.
    fn apply_config_jvm(&self, request: &mut LaunchRequest) {
        if request.jvm.is_none() {
            request.jvm = Some(self.config.effective_jvm());
            request.jvm_global = Some(self.config.jvm.clone());
        }
    }

    /// Todo lo que pasa entre pulsar «Jugar» y tener un PID vivo.
    async fn start_game(&mut self, request: &LaunchRequest) -> Result<RunningGame, CoreError> {
        if self.probe.running() {
            return Err(lo_launch::LaunchError::OfficialLauncherRunning.into());
        }
        let spec = self.spec_for(&request.version)?;
        let plan =
            lo_launch::build_plan(&self.paths, &self.environment, spec, request, &stamp_now())?;

        Self::write_command_log(&plan)?;
        // El guard cierra la sesión en `Drop` y descuenta la referencia: si hay
        // otra instancia viva, `accounts.json` no se restaura.
        let guard = self.accounts.swap_guard(&request.nick)?;

        let supervisor = match Supervisor::start_with(&plan, request, &WatchOptions::default()) {
            Ok(supervisor) => supervisor,
            Err(error) => {
                // El juego no arrancó: la cuenta temporal no debe quedarse.
                let _ = guard.end();
                return Err(error.into());
            }
        };

        let game = supervisor.game().clone();
        lock(&self.running).push(game.clone());
        self.spawn_forwarder(supervisor, guard);
        Ok(game)
    }

    /// Puente entre el supervisor (log/exit) y la UI, con la restauración al
    /// final. El proceso vive aparte: si la app se cierra, el juego sigue.
    fn spawn_forwarder(&mut self, mut supervisor: Supervisor, guard: lo_accounts::SwapGuard) {
        let events = self.events.clone();
        let probe = self.probe.clone();
        let state = Arc::clone(&self.state);
        let running = Arc::clone(&self.running);
        let pid = supervisor.game().pid;

        let worker = tokio::spawn(async move {
            let mut launcher_warned = false;
            let mut ticker = tokio::time::interval(LAUNCHER_CHECK);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            loop {
                let event = tokio::select! {
                    event = supervisor.next_event() => event,
                    _ = ticker.tick() => {
                        if !launcher_warned && probe.running() {
                            launcher_warned = true;
                            let _ = events.send(CoreEvent::Launch(LaunchEvent::Warning(
                                HealthIssue::OfficialLauncherAppeared,
                            )));
                        }
                        continue;
                    }
                };
                let Some(event) = event else {
                    break;
                };
                let finished = matches!(event, LaunchEvent::Exited { .. });
                let _ = events.send(CoreEvent::Launch(event));

                if finished {
                    // Solo la última sesión restaura de verdad; las demás
                    // devuelven `SessionEnded` y dejan la cuenta en su sitio.
                    let last = {
                        let list = lock(&running);
                        list.iter().filter(|game| game.pid != pid).count() == 0
                    };
                    if last {
                        set_state(&state, GameState::Restoring);
                        let _ = events.send(CoreEvent::State(GameState::Restoring));
                    }
                    let result = guard.end();
                    lock(&running).retain(|game| game.pid != pid);
                    let next = match result {
                        Ok(_) => state_for(&running),
                        Err(error) => GameState::Error {
                            message: error.to_string(),
                        },
                    };
                    set_state(&state, next.clone());
                    let _ = events.send(CoreEvent::State(next));
                    break;
                }
            }
        });

        // Se guardan para no perder el hilo; los terminados se podan al lanzar.
        self.workers.retain(|worker| !worker.is_finished());
        self.workers.push(worker);
    }

    fn set_state(&self, state: GameState) {
        let _ = self.events.send(CoreEvent::State(state.clone()));
        set_state(&self.state, state);
    }
}

/// Fija el estado compartido, ignorando un candado envenenado: el estado es un
/// dato, no una invariante que haya que romper el proceso por ella.
fn set_state(state: &Mutex<GameState>, value: GameState) {
    *lock(state) = value;
}

/// Estado que toca según las instancias vivas.
fn state_for(running: &Mutex<Vec<RunningGame>>) -> GameState {
    let games = lock(running);
    match games.as_slice() {
        [] => GameState::Idle,
        [game] => GameState::Running(game.clone()),
        games => GameState::RunningMultiple {
            count: games.len(),
            nicks: games.iter().map(|game| game.nick.clone()).collect(),
        },
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sello de tiempo `AAAAMMDD-HHMMSS` en UTC, para los nombres de `logs/run-*`.
#[must_use]
pub fn stamp_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(0);
    let (year, month, day, hour, minute, second) = civil_from_unix(secs);
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

/// Fecha civil UTC a partir de epoch en segundos (algoritmo de Hinnant).
#[must_use]
pub fn civil_from_unix(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let (hour, minute, second) = (rest / 3_600, (rest % 3_600) / 60, rest % 60);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);

    (year, month, day, hour, minute, second)
}

/// Errores de la fachada. Siempre llevan la causa concreta.
#[derive(Debug, Error)]
pub enum CoreError {
    /// Rutas del sistema.
    #[error(transparent)]
    Paths(#[from] PathsError),
    /// Configuración.
    #[error(transparent)]
    Config(#[from] lo_config::ConfigError),
    /// Cuentas.
    #[error(transparent)]
    Accounts(#[from] lo_accounts::AccountsError),
    /// Arranque.
    #[error(transparent)]
    Launch(#[from] lo_launch::LaunchError),
    /// Diagnóstico.
    #[error(transparent)]
    Diag(#[from] DiagError),
    /// Fallo de E/S de la fachada (logs, comando, configuración).
    #[error("no pude escribir {path}: {source}")]
    Io {
        /// Fichero o directorio afectado.
        path: PathBuf,
        /// Causa del sistema.
        source: std::io::Error,
    },
    /// Operación no válida en el estado actual (por ejemplo, lanzar dos veces).
    #[error("no puedo hacer eso ahora mismo: {0}")]
    BadState(&'static str),
}

/// Sucesos que la UI consume como eventos tipados.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEvent {
    /// Cambio de estado de la partida.
    State(GameState),
    /// Suceso del proceso del juego (log, aviso, salida).
    Launch(LaunchEvent),
}
