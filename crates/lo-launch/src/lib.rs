//! Construcción del comando de arranque, supervisor del proceso y salud del
//! arranque.
//!
//! El comando va argumento a argumento, en el orden que espera Genesis:
//!
//! ```text
//! javaw.exe -Xms<MB>M -Xmx<MB>M <JAVA_FLAGS>
//!   -Dlog4j2.formatMsgNoLookups=true
//!   -Dlunar.webosr.url=file:index.html
//!   -Djava.library.path=<natives>
//!   -Dichor.usingIsolatedProfiles=true
//!   -Dichor.fabric.localModPath=<profiles/1.8/mods>
//!   -XX:+DisableAttachMechanism
//!   [-Dlog4j.configurationFile=<profiles/1.8/logs/config.xml>]
//!   -Dichor.logsFile=<profiles/1.8/logs/ichor-boot.log>
//!   -cp <jars absolutos unidos por ';'> com.moonsworth.lunar.genesis.Genesis
//!   --workingDirectory . --classpathDir <multiver>
//!   --ichorExternalFiles <OptiFine> --ichorClassPath <nombres unidos por ','>
//!   --webosrDir <natives> --texturesDir ... --jitDir ... --uiDir <ui/<sha1>>
//!   --version <subversion> --gameDir ... --assetsDir ... --assetIndex ...
//!   --uuid <uuid offline> --xuid "not specified" --accessToken 0
//!   --userProperties {} --username <nick> --userType legacy
//!   --versionType release --installationId "not specified"
//!   --width <w> --height <h> --ipcPort 0
//!   [--fullscreen] [--server host:puerto]
//! ```
//!
//! El plan se construye como [`Vec<OsString>`] (nunca concatenando cadenas),
//! con el directorio de trabajo en [`LaunchPlan::workdir`] y el comando exacto
//! guardado en `logs/cmd-<marca>.txt` para soporte.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::SystemTime;

use lo_accounts::{Nick, offline_uuid};
use lo_detect::{
    Environment, JAVA_FLAGS, MAIN_CLASS, PROFILE_SUBVERSION, VersionSpec, classpath_jars,
    find_ui_dir,
};
use lo_paths::LunarPaths;
use thiserror::Error;

mod supervisor;

pub use supervisor::{MAX_LINE_BYTES, POLL_INTERVAL, Supervisor, WINDOW_TIMEOUT, WatchOptions};

/// Puerto por defecto cuando el usuario escribe un host sin puerto.
pub const DEFAULT_PORT: u16 = 25565;
/// Número de clases de un arranque correcto (el *bake* de ichor).
///
/// Menos de esto significa classpath mal armado: Genesis arrancó el Minecraft
/// vanilla.
pub const EXPECTED_BAKED_CLASSES: u32 = 19_039;
/// Número de clases de un arranque vanilla (sin transformar).
pub const VANILLA_BAKED_CLASSES: u32 = 4_389;

/// Petición de arranque ya validada por la UI.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchRequest {
    /// Nick offline validado.
    pub nick: Nick,
    /// Versión instalada que se quiere arrancar.
    pub version: String,
    /// RAM en MB (`-Xms`/`-Xmx`) cuando no hay ajustes de JVM.
    pub ram_mb: u32,
    /// Servidor al que entrar, tal cual lo escribió el usuario (vacío = menú).
    pub server: String,
    /// Ancho de la ventana del juego.
    pub width: u32,
    /// Alto de la ventana del juego.
    pub height: u32,
    /// Arrancar en pantalla completa.
    pub fullscreen: bool,
    /// Java elegido a mano; `None` = el detectado.
    pub java: Option<PathBuf>,
    /// Ajustes de JVM del perfil. `None` = `-Xms`/`-Xmx` con [`Self::ram_mb`],
    /// que es el comando que se genera.
    pub jvm: Option<lo_config::JvmSettings>,
    /// Ajustes globales, para resolver el preset `inherit` de un perfil.
    pub jvm_global: Option<lo_config::JvmSettings>,
    /// Emitir también los flags que piden confirmación (`-cp`, `-jar`,
    /// `-javaagent`). Lo decide la UI con el usuario delante.
    pub jvm_confirm: bool,
}

impl LaunchRequest {
    /// Petición con los valores por defecto del launcher para un nick.
    #[must_use]
    pub fn new(nick: Nick) -> Self {
        Self {
            nick,
            version: "1.8.9".to_owned(),
            ram_mb: 3072,
            server: String::new(),
            width: 1280,
            height: 720,
            fullscreen: false,
            java: None,
            jvm: None,
            jvm_global: None,
            jvm_confirm: false,
        }
    }

    /// Ajustes de JVM ya resueltos para esta petición.
    ///
    /// Sin ajustes devuelve el comando de hoy: `-Xms` y `-Xmx` iguales a la RAM
    /// del perfil, sin flags extra.
    #[must_use]
    pub fn resolved_jvm(&self) -> lo_jvm::ResolvedJvm {
        if let Some(settings) = &self.jvm {
            let fallback = self.jvm_global.clone().unwrap_or_else(|| settings.clone());
            return lo_jvm::ResolvedJvm::resolve(settings, &fallback, self.ram_mb);
        }
        let settings = lo_config::JvmSettings {
            preset: lo_config::JvmPreset::Lunar,
            xms_mb: Some(self.ram_mb),
            xmx_mb: Some(self.ram_mb),
            ..lo_config::JvmSettings::default()
        };
        lo_jvm::ResolvedJvm::resolve(&settings, &lo_config::JvmSettings::default(), self.ram_mb)
    }
}

/// Dirección de servidor ya normalizada, tal cual se pasa a `--server`.
///
/// Se guarda el argumento completo y no `host`/`puerto` separados porque las
/// las reglas no siempre son separables: `a:b:c` se deja intacto y
/// `[::1]` sin puerto también. Reglas (idénticas a `build_command`):
///
/// - `mc.es` → `mc.es:25565`
/// - `mc.es:25566` → `mc.es:25566`
/// - `mc.es:` → `mc.es:25565`
/// - `[::1]:25565` → se deja tal cual (ya trae puerto)
/// - `[::1]` → **se deja tal cual**, sin añadir `25565` (discrepancia conocida
///   con el caso `host`, reproducida a propósito)
/// - `a:b:c` → se deja tal cual (se parte por el primer `:` y se conserva el
///   resto como «puerto»)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddress {
    /// Argumento normalizado que recibe `--server`.
    pub argument: String,
}

impl ServerAddress {
    /// Normaliza lo que escribió el usuario, o `None` si está vacío.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let host = raw.trim();
        if host.is_empty() {
            return None;
        }
        let argument = if host.contains(':') && !host.starts_with('[') {
            match host.split_once(':') {
                Some((name, port)) if !port.is_empty() => format!("{name}:{port}"),
                Some((name, _)) => format!("{name}:{DEFAULT_PORT}"),
                None => host.to_owned(),
            }
        } else if !host.starts_with('[') && !host.contains(':') {
            format!("{host}:{DEFAULT_PORT}")
        } else {
            // IPv6 entre corchetes o con puerto: no se toca.
            host.to_owned()
        };
        Some(Self { argument })
    }
}

/// Comando listo para ejecutar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// Ejecutable (`javaw.exe`).
    pub program: PathBuf,
    /// Argumentos, en el orden exacto del comando.
    pub args: Vec<OsString>,
    /// Directorio de trabajo del proceso (`--workingDirectory .`).
    pub workdir: PathBuf,
    /// Fichero donde se va a volcar la salida del juego.
    pub log_path: PathBuf,
    /// Fichero donde se guarda el comando exacto (para soporte).
    pub command_log: PathBuf,
    /// Versión que se va a arrancar.
    pub version: &'static str,
    /// Avisos de los flags de JVM descartados, para que la UI los cuente.
    pub jvm_warnings: Vec<lo_jvm::JvmWarning>,
}

impl LaunchPlan {
    /// Reproducción legible del comando, una línea por argumento entrecomillado.
    ///
    /// Es lo que se escribe en `logs/cmd-<marca>.txt` y lo que se copia al
    /// informe de soporte. No se usa para ejecutar.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&quote(&self.program.to_string_lossy()));
        out.push('\n');
        for arg in &self.args {
            out.push_str(&quote(&arg.to_string_lossy()));
            out.push('\n');
        }
        out
    }

    /// Número de argumentos (sin contar el ejecutable).
    #[must_use]
    pub fn arg_count(&self) -> usize {
        self.args.len()
    }
}

fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\\\""))
}

/// Construye el plan de arranque completo.
///
/// Falla si falta la JRE, los natives, los artefactos o la versión pedida.
pub fn build_plan(
    paths: &LunarPaths,
    env: &Environment,
    spec: &VersionSpec,
    request: &LaunchRequest,
    stamp: &str,
) -> Result<LaunchPlan, LaunchError> {
    if spec.name != request.version {
        return Err(LaunchError::UnsupportedVersion(request.version.clone()));
    }

    let program = request
        .java
        .clone()
        .or_else(|| env.java.as_ref().map(|java| java.path.clone()))
        .ok_or(LaunchError::NoJava)?;
    let natives = env
        .natives
        .as_ref()
        .ok_or(LaunchError::NoNatives)?
        .path
        .clone();

    let multiver = paths.multiver().to_path_buf();
    let names = classpath_jars(&multiver, spec);
    if names.is_empty() {
        return Err(LaunchError::NoArtifacts(spec.name.to_owned()));
    }

    // La UI vive en ui/<sha1>/, no en ui/: WebOSR resuelve file:index.html como
    // File(uiDir, "index.html").
    let ui = find_ui_dir(&paths.lunar_root().join("ui")).ok_or(LaunchError::NoUiDir)?;

    let profile = paths.profile_dir(PROFILE_SUBVERSION);
    let mods = profile.join("mods");
    let ichor_log = paths.ichor_log(PROFILE_SUBVERSION);
    let log4j_config = paths.log4j_config(PROFILE_SUBVERSION);

    let mut args: Vec<OsString> = Vec::with_capacity(72);
    // Memoria y flags los decide `lo-jvm`; sin ajustes en la petición sale
    // `-Xms`/`-Xmx` con la RAM del perfil, que es el comando de hoy.
    let jvm = request.resolved_jvm();
    args.extend(
        jvm.arguments_confirming(request.jvm_confirm)
            .into_iter()
            .map(OsString::from),
    );
    args.extend(JAVA_FLAGS.iter().map(|flag| OsString::from(*flag)));
    args.push("-Dlog4j2.formatMsgNoLookups=true".into());
    args.push("-Dlunar.webosr.url=file:index.html".into());
    args.push(format!("-Djava.library.path={}", natives.display()).into());
    args.push("-Dichor.usingIsolatedProfiles=true".into());
    args.push(format!("-Dichor.fabric.localModPath={}", mods.display()).into());
    args.push("-XX:+DisableAttachMechanism".into());
    if log4j_config.is_file() {
        args.push(format!("-Dlog4j.configurationFile={}", log4j_config.display()).into());
    }
    args.push(format!("-Dichor.logsFile={}", ichor_log.display()).into());

    let classpath = names
        .iter()
        .map(|name| multiver.join(name).display().to_string())
        .collect::<Vec<_>>()
        .join(";");
    args.push("-cp".into());
    args.push(classpath.into());
    args.push(MAIN_CLASS.into());

    args.extend(game_arguments(
        paths, spec, request, &multiver, &natives, &names, &ui,
    ));

    if request.fullscreen {
        args.push("--fullscreen".into());
    }
    if let Some(server) = ServerAddress::parse(&request.server) {
        args.push("--server".into());
        args.push(server.argument.into());
    }

    Ok(LaunchPlan {
        program,
        args,
        workdir: multiver,
        log_path: paths.run_log(stamp),
        command_log: paths.command_log(stamp),
        version: spec.name,
        jvm_warnings: jvm.warnings().to_vec(),
    })
}

/// Argumentos del juego (`--flag valor`), en el orden exacto.
///
/// Se separa de [`build_plan`] para que la función quepa en una pantalla y el
/// orden de los 23 pares se lea de un vistazo.
fn game_arguments(
    paths: &LunarPaths,
    spec: &VersionSpec,
    request: &LaunchRequest,
    multiver: &std::path::Path,
    natives: &std::path::Path,
    names: &[String],
    ui: &std::path::Path,
) -> Vec<OsString> {
    let game_dir = paths.minecraft_dir().to_path_buf();
    let assets_dir = game_dir.join("assets");
    let mut args: Vec<OsString> = Vec::with_capacity(46);

    args.push("--workingDirectory".into());
    args.push(".".into());
    args.push("--classpathDir".into());
    args.push(multiver.as_os_str().to_owned());
    args.push("--ichorExternalFiles".into());
    args.push(spec.external.into());
    args.push("--ichorClassPath".into());
    args.push(names.join(",").into());
    args.push("--webosrDir".into());
    args.push(natives.as_os_str().to_owned());
    args.push("--texturesDir".into());
    args.push(paths.lunar_root().join("textures").into_os_string());
    args.push("--jitDir".into());
    args.push(paths.lunar_root().join("jit").into_os_string());
    args.push("--uiDir".into());
    args.push(ui.as_os_str().to_owned());
    args.push("--version".into());
    args.push(spec.subversion.into());
    args.push("--gameDir".into());
    args.push(game_dir.into_os_string());
    args.push("--assetsDir".into());
    args.push(assets_dir.into_os_string());
    args.push("--assetIndex".into());
    args.push(spec.asset_index.into());
    args.push("--uuid".into());
    args.push(offline_uuid(&request.nick).to_string().into());
    args.extend(
        [
            "--xuid",
            "not specified",
            "--accessToken",
            "0",
            "--userProperties",
            "{}",
            "--username",
            request.nick.as_str(),
            "--userType",
            "legacy",
            "--versionType",
            "release",
            "--installationId",
            "not specified",
        ]
        .into_iter()
        .map(OsString::from),
    );
    args.push("--width".into());
    args.push(request.width.to_string().into());
    args.push("--height".into());
    args.push(request.height.to_string().into());
    args.push("--ipcPort".into());
    args.push("0".into());

    args
}

/// Juego en ejecución.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningGame {
    /// PID del proceso Java.
    pub pid: u32,
    /// Momento del arranque.
    pub started_at: SystemTime,
    /// Log que se está siguiendo.
    pub log_path: PathBuf,
    /// Versión arrancada.
    pub version: String,
    /// Nick con el que se arrancó.
    pub nick: String,
}

/// Suceso del ciclo de vida que la UI consume tal cual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchEvent {
    /// El proceso arrancó.
    Started(RunningGame),
    /// Una línea nueva del log.
    LogLine(String),
    /// Un health check se quejó.
    Warning(HealthIssue),
    /// El proceso terminó.
    Exited {
        /// Código de salida, si el sistema lo dio.
        code: Option<i32>,
        /// Duración total del proceso.
        uptime_secs: u64,
    },
}

/// Problemas detectados leyendo el log en vivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthIssue {
    /// Genesis arrancó el Minecraft vanilla: classpath mal armado.
    VanillaClasspath {
        /// Clases contadas en el log.
        classes: u32,
    },
    /// Falta el directorio de recursos Ultralight (`--webosrDir` mal).
    MissingUltralightResources,
    /// No se pudo cargar un artefacto del `--ichorClassPath`.
    MissingArtifact(String),
    /// El juego no llegó a abrir ventana en el tiempo esperado.
    NoWindowAfter {
        /// Segundos transcurridos.
        secs: u64,
    },
    /// El servidor rechazó la sesión (`Invalid session`, servidor premium).
    PremiumServerRefused,
    /// Lunar aplicó una cuenta distinta de la pedida: el swap no cuajó.
    WrongUser {
        /// Nick con el que se quería jugar.
        expected: String,
        /// Nick que Lunar puso de verdad.
        actual: String,
    },
    /// El oficial se abrió mientras se jugaba: `accounts.json` está en riesgo.
    OfficialLauncherAppeared,
}

impl HealthIssue {
    /// Mensaje en lenguaje claro para la barra de estado.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::VanillaClasspath { classes } => format!(
                "Genesis horneó {classes} clases (se esperaban {EXPECTED_BAKED_CLASSES}): el classpath está mal armado"
            ),
            Self::MissingUltralightResources => {
                "Falta la carpeta resources/ del directorio de natives".to_owned()
            }
            Self::MissingArtifact(name) => {
                format!("No pude cargar el artefacto {name} del classpath de Lunar")
            }
            Self::NoWindowAfter { secs } => {
                format!("El juego no abrió ventana en {secs} s")
            }
            Self::PremiumServerRefused => {
                "El servidor rechazó la sesión (Invalid session): es un servidor premium".to_owned()
            }
            Self::WrongUser { expected, actual } => format!(
                "Lunar aplicó la cuenta '{actual}' en vez de '{expected}': el swap de accounts.json no cuajó"
            ),
            Self::OfficialLauncherAppeared => {
                "Lunar Client oficial se abrió mientras jugabas: accounts.json puede haberse reescrito"
                    .to_owned()
            }
        }
    }

    /// Acción sugerida, en una línea.
    #[must_use]
    pub fn hint(&self) -> &'static str {
        match self {
            Self::VanillaClasspath { .. } => {
                "Comprueba que los jars de Lunar están completos en offline/multiver"
            }
            Self::MissingUltralightResources => {
                "Abre el launcher oficial una vez para que descargue los recursos"
            }
            Self::MissingArtifact(_) => "Vuelve a abrir Lunar Client para reparar los artefactos",
            Self::NoWindowAfter { .. } => "Mira el log de ichor: puede faltar memoria o un driver",
            Self::PremiumServerRefused => "Usa un servidor no premium (sesión offline)",
            Self::WrongUser { .. } => {
                "Comprueba que el oficial esté cerrado y que accounts.json sea escribible"
            }
            Self::OfficialLauncherAppeared => {
                "Cierra el oficial y reinicia el launcher para restaurar las cuentas"
            }
        }
    }
}

/// Fallos al preparar el arranque.
#[derive(Debug, Error)]
pub enum LaunchError {
    /// Falta la JRE.
    #[error("no encontré ninguna JRE utilizable")]
    NoJava,
    /// Faltan los natives.
    #[error("no encontré el directorio natives de Lunar")]
    NoNatives,
    /// No hay artefactos para esa versión.
    #[error("no hay artefactos para {0} en el directorio de Lunar")]
    NoArtifacts(String),
    /// No hay directorio de UI web con `index.html`.
    #[error("no encontré la UI web de Lunar (ui/<hash>/index.html); abre el oficial una vez")]
    NoUiDir,
    /// Versión desconocida.
    #[error("versión no soportada: {0}")]
    UnsupportedVersion(String),
    /// El launcher oficial está abierto.
    #[error("Lunar Client (el oficial) está abierto; ciérralo para que no reescriba accounts.json")]
    OfficialLauncherRunning,
    /// Fallo al crear el proceso.
    #[error("no pude arrancar Java: {0}")]
    Spawn(#[source] std::io::Error),
    /// Fallo de E/S al preparar logs.
    #[error("no pude preparar el log del arranque: {0}")]
    Io(#[source] std::io::Error),
    /// El nick de la petición no es válido.
    #[error("nick no válido: {0}")]
    Nick(#[from] lo_accounts::NickError),
}
