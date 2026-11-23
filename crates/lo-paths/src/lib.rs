//! Rutas del launcher, de Lunar Client y de Minecraft.
//!
//! ```text
//! APP_DIR        %APPDATA%\LunarOffline
//! CONFIG         %APPDATA%\LunarOffline\config.json
//! LOGS           %APPDATA%\LunarOffline\logs
//! ORIGINAL       %APPDATA%\LunarOffline\accounts.original.json
//! SWAP_MARKER    %APPDATA%\LunarOffline\swap.pending
//! LUNAR_ROOT     %USERPROFILE%\.lunarclient
//! MULTIVER       %USERPROFILE%\.lunarclient\offline\multiver
//! MC_DIR         %APPDATA%\.minecraft
//! ACCOUNTS       %USERPROFILE%\.lunarclient\settings\game\accounts.json
//! ```
//!
//! Nada de concatenar cadenas: todo es [`PathBuf`] y las uniones se hacen con
//! [`Path::join`], que respeta el separador de la plataforma.
//!
//! La resolución del entorno está separada de la aritmética a propósito:
//! [`LunarPaths::detect_with`] es puro (recibe las variables ya leídas) y
//! [`LunarPaths::detect`] lee el proceso y crea los directorios. Así
//! `detect_with` se puede usar sin tocar el entorno global — `std::env::set_var`
//! es `unsafe` desde Rust 2024 y este crate prohíbe `unsafe`.

use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Error de resolución de rutas del sistema.
#[derive(Debug, Error)]
pub enum PathsError {
    /// No se pudo obtener `%APPDATA%`.
    #[error("no encuentro %APPDATA% en este equipo")]
    NoAppData,
    /// No se pudo obtener el directorio del usuario.
    #[error("no encuentro el directorio del usuario (%USERPROFILE%)")]
    NoHome,
    /// Fallo de E/S al crear los directorios de la aplicación.
    #[error("no pude crear los directorios de la aplicación: {0}")]
    Io(#[from] io::Error),
}

/// Nombres de fichero y carpeta que usa la app dentro de `%APPDATA%\LunarOffline`.
const CONFIG_NAME: &str = "config.json";
const LOGS_FOLDER: &str = "logs";
const ORIGINAL_NAME: &str = "accounts.original.json";
const ORIGINAL_ACTIVE_NAME: &str = "accounts.original.active";
const SWAP_MARKER_NAME: &str = "swap.pending";
const SELFTEST_NAME: &str = "selftest.txt";
const ERROR_LOG_NAME: &str = "error.log";

/// Nombre de la carpeta de datos propia dentro de `%APPDATA%`.
pub const APP_FOLDER: &str = "LunarOffline";
/// Nombre de la carpeta de Minecraft dentro de `%APPDATA%`.
pub const MINECRAFT_FOLDER: &str = ".minecraft";
/// Nombre de la carpeta del launcher oficial en el perfil del usuario.
pub const LUNAR_FOLDER: &str = ".lunarclient";

/// Variables de entorno que resuelven las raíces.
///
/// Se inyectan para poder probar [`LunarPaths::detect_with`] sin tocar el
/// entorno del proceso. Los valores vacíos cuentan como ausentes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvVars {
    /// `%APPDATA%`.
    pub appdata: Option<PathBuf>,
    /// `%USERPROFILE%`.
    pub userprofile: Option<PathBuf>,
    /// `$HOME` (solo se usa si falta todo lo anterior).
    pub home: Option<PathBuf>,
}

impl EnvVars {
    /// Lee las variables del proceso actual.
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            appdata: env_path("APPDATA"),
            userprofile: env_path("USERPROFILE"),
            home: env_path("HOME"),
        }
    }

    /// Raíz de datos: `%APPDATA%` y, si falta, el directorio del usuario.
    ///
    /// `%APPDATA%`, y si falta, el perfil del usuario.
    #[must_use]
    pub fn appdata_root(&self) -> Option<&Path> {
        present(self.appdata.as_deref())
            .or_else(|| present(self.userprofile.as_deref()))
            .or_else(|| present(self.home.as_deref()))
    }

    /// Directorio del usuario: `%USERPROFILE%` y, si falta, `$HOME`.
    ///
    /// Se usa `USERPROFILE` y, si falta, `$HOME`: mismo valor en Windows y el
    /// crate queda usable en Linux para desarrollo.
    #[must_use]
    pub fn user_dir(&self) -> Option<&Path> {
        present(self.userprofile.as_deref()).or_else(|| present(self.home.as_deref()))
    }
}

/// Una raíz vacía es una raíz ausente: `%APPDATA%=""` no es un directorio.
fn present(root: Option<&Path>) -> Option<&Path> {
    root.filter(|path| !path.as_os_str().is_empty())
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// Conjunto completo de rutas que usa el launcher.
///
/// Se construye una vez al arrancar y se pasa por referencia a los demás crates.
/// [`LunarPaths::detect`] lee el entorno; [`LunarPaths::new`] permite inyectar
/// raíces arbitrarias sin leer el entorno.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LunarPaths {
    app_dir: PathBuf,
    lunar_root: PathBuf,
    multiver: PathBuf,
    minecraft_dir: PathBuf,
    accounts_file: PathBuf,
    config_file: PathBuf,
    logs_dir: PathBuf,
    original_accounts: PathBuf,
    original_active: PathBuf,
    swap_marker: PathBuf,
    selftest_file: PathBuf,
    error_log: PathBuf,
}

impl LunarPaths {
    /// Construye el conjunto de rutas a partir de tres raíces ya resueltas.
    ///
    /// `app_dir` es el directorio de datos de la aplicación (`%APPDATA%\LunarOffline`),
    /// `lunar_root` es `~/.lunarclient` y `minecraft_dir` es `%APPDATA%\.minecraft`.
    /// No toca el disco: es aritmética de rutas.
    #[must_use]
    pub fn new(app_dir: PathBuf, lunar_root: PathBuf, minecraft_dir: PathBuf) -> Self {
        let multiver = lunar_root.join("offline").join("multiver");
        let accounts_file = lunar_root
            .join("settings")
            .join("game")
            .join("accounts.json");
        Self {
            config_file: app_dir.join(CONFIG_NAME),
            logs_dir: app_dir.join(LOGS_FOLDER),
            original_accounts: app_dir.join(ORIGINAL_NAME),
            original_active: app_dir.join(ORIGINAL_ACTIVE_NAME),
            swap_marker: app_dir.join(SWAP_MARKER_NAME),
            selftest_file: app_dir.join(SELFTEST_NAME),
            error_log: app_dir.join(ERROR_LOG_NAME),
            app_dir,
            lunar_root,
            multiver,
            minecraft_dir,
            accounts_file,
        }
    }

    /// Resuelve las rutas reales del equipo y crea `app_dir` y `logs_dir`.
    ///
    /// Los directorios se crean al detectar, no al importar,
    /// que es el único momento en que se conocen las raíces.
    pub fn detect() -> Result<Self, PathsError> {
        let paths = Self::detect_with(&EnvVars::from_process())?;
        paths.ensure_dirs()?;
        Ok(paths)
    }

    /// Resuelve las rutas a partir de variables ya leídas. No toca el disco.
    pub fn detect_with(env: &EnvVars) -> Result<Self, PathsError> {
        let appdata = env.appdata_root().ok_or(PathsError::NoAppData)?;
        let home = env.user_dir().ok_or(PathsError::NoHome)?;
        Ok(Self::new(
            appdata.join(APP_FOLDER),
            home.join(LUNAR_FOLDER),
            appdata.join(MINECRAFT_FOLDER),
        ))
    }

    /// Directorio de datos de la aplicación (`%APPDATA%\LunarOffline`).
    #[must_use]
    pub fn app_dir(&self) -> &Path {
        &self.app_dir
    }

    /// Raíz de Lunar Client (`~/.lunarclient`).
    #[must_use]
    pub fn lunar_root(&self) -> &Path {
        &self.lunar_root
    }

    /// Directorio de artefactos (`~/.lunarclient/offline/multiver`).
    #[must_use]
    pub fn multiver(&self) -> &Path {
        &self.multiver
    }

    /// Directorio de Minecraft (`%APPDATA%\.minecraft`).
    #[must_use]
    pub fn minecraft_dir(&self) -> &Path {
        &self.minecraft_dir
    }

    /// `accounts.json` del launcher oficial.
    #[must_use]
    pub fn accounts_file(&self) -> &Path {
        &self.accounts_file
    }

    /// `config.json` propio.
    #[must_use]
    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    /// Directorio de logs de arranque.
    #[must_use]
    pub fn logs_dir(&self) -> &Path {
        &self.logs_dir
    }

    /// Copia de seguridad de `accounts.json` previa al swap.
    #[must_use]
    pub fn original_accounts(&self) -> &Path {
        &self.original_accounts
    }

    /// Copia del `activeAccountLocalId` previo al swap.
    #[must_use]
    pub fn original_active(&self) -> &Path {
        &self.original_active
    }

    /// Marca de swap vivo.
    #[must_use]
    pub fn swap_marker(&self) -> &Path {
        &self.swap_marker
    }

    /// Informe de `--selftest`.
    #[must_use]
    pub fn selftest_file(&self) -> &Path {
        &self.selftest_file
    }

    /// Volcado de una excepción no controlada de la interfaz.
    #[must_use]
    pub fn error_log(&self) -> &Path {
        &self.error_log
    }

    /// `~/.lunarclient/offline/multiver/natives`.
    #[must_use]
    pub fn natives_root(&self) -> PathBuf {
        self.multiver.join("natives")
    }

    /// `~/.lunarclient/jre`, raíz donde viven las JRE del launcher oficial.
    #[must_use]
    pub fn jre_root(&self) -> PathBuf {
        self.lunar_root.join("jre")
    }

    /// `~/.lunarclient/profiles`.
    #[must_use]
    pub fn profiles_dir(&self) -> PathBuf {
        self.lunar_root.join("profiles")
    }

    /// `~/.lunarclient/profiles/<subversion>` (`1.8`, `1.7`, `26`).
    #[must_use]
    pub fn profile_dir(&self, subversion: &str) -> PathBuf {
        self.profiles_dir().join(subversion)
    }

    /// Log de un arranque, con marca de tiempo `AAAAMMDD-HHMMSS` en UTC.
    ///
    /// `stamp` tiene el formato `%Y%m%d-%H%M%S`.
    #[must_use]
    pub fn run_log(&self, stamp: &str) -> PathBuf {
        self.logs_dir.join(format!("run-{stamp}.log"))
    }

    /// Volcado del comando exacto de un arranque.
    #[must_use]
    pub fn command_log(&self, stamp: &str) -> PathBuf {
        self.logs_dir.join(format!("cmd-{stamp}.txt"))
    }

    /// Fichero de log de ichor que escribe Genesis
    /// (`profiles/1.8/logs/ichor-boot.log`).
    ///
    /// Usa el perfil `1.8` siempre, incluso para 1.7.10.
    #[must_use]
    pub fn ichor_log(&self, subversion: &str) -> PathBuf {
        self.profile_dir(subversion)
            .join("logs")
            .join("ichor-boot.log")
    }

    /// Configuración de log4j del perfil (`profiles/<subversion>/logs/config.xml`).
    #[must_use]
    pub fn log4j_config(&self, subversion: &str) -> PathBuf {
        self.profile_dir(subversion).join("logs").join("config.xml")
    }

    /// Crea `app_dir` y `logs_dir` si no existen.
    pub fn ensure_dirs(&self) -> Result<(), PathsError> {
        std::fs::create_dir_all(&self.app_dir)?;
        std::fs::create_dir_all(&self.logs_dir)?;
        Ok(())
    }
}

/// Intentos de rename antes de rendirse ante un bloqueo del sistema.
///
/// En Windows el antivirus, el indexador o el propio launcher oficial pueden
/// tener el fichero abierto un instante; el rename falla con `PermissionDenied`
/// y vuelve a funcionar unos milisegundos después.
const RENAME_ATTEMPTS: u32 = 6;
/// Espera entre intentos de rename.
const RENAME_BACKOFF_MS: u64 = 30;

/// Escritura atómica: temporal en el mismo directorio, `sync_all` y rename.
///
/// Nunca deja el fichero destino a medias: o está el contenido anterior o está
/// el nuevo. La usan `config.json`, `accounts.json` y los marcadores de swap.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let name = path.file_name().map_or_else(
        || "lunar-offline.tmp".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let tmp = parent.join(format!(".{name}.tmp"));

    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);

    if let Err(err) = rename_with_retry(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

/// `fs::rename` con reintentos para los bloqueos momentáneos de Windows.
pub fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempt = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(err) if attempt + 1 < RENAME_ATTEMPTS && is_transient(&err) => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(RENAME_BACKOFF_MS));
            }
            Err(err) => return Err(err),
        }
    }
}

/// Errores que en Windows suelen ser un bloqueo momentáneo del fichero.
fn is_transient(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::WouldBlock | io::ErrorKind::Other
    )
}
