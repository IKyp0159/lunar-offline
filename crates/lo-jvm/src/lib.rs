//! Flags de la JVM: presets, validación y prueba real de flags.
//!
//! Esta crate no lanza Minecraft. Decide **qué argumentos de máquina virtual**
//! acompañan al comando (`-Xms`, `-Xmx` y los `-XX:`/`-D` del usuario), dice
//! cuáles no debe tocar y comprueba con el Java instalado si un flag se acepta.
//!
//! ## Presets
//!
//! | preset | `-Xms` | `-Xmx` | flags extra |
//! |---|---|---|---|
//! | `lunar` | RAM del perfil | RAM del perfil | ninguno — el comando de hoy |
//! | `low` | 512 | `min(2048, ram)` | `-XX:+UseSerialGC` |
//! | `high` | `ram / 4` | `ram` | G1 con los ajustes de PvP |
//! | `custom` | lo que diga el usuario | lo que diga el usuario | lo que escriba, validado |
//!
//! `lunar` usa los flags del launcher oficial, `-Xms`/`-Xmx` del perfil
//! incluidos. `xms_mb` y `xmx_mb` de [`JvmSettings`] mandan sobre el preset
//! cuando están puestos: quien escribe 4096 quiere 4096, no lo que diga el
//! preset.
//!
//! ## Qué no se emite nunca
//!
//! El launcher es dueño de una parte del comando. Un `-cp`, un `-jar` o un
//! `-javaagent` cambiarían qué se ejecuta, y un `-Djava.library.path`, un
//! `-Dlunar.*`/`-Dichor.*`/`-Dwebosr.*` o un `-Xbootclasspath` romperían el
//! arranque que ya funciona. Los tres primeros se pueden usar si el usuario lo
//! confirma ([`ResolvedJvm::arguments_confirming`]); los demás se descartan con
//! aviso. Un token suelto sin `-` parece una clase principal y también se
//! descarta: se colaría como argumento del juego.
//!
//! ## Prueba de flags
//!
//! [`probe_flags`] ejecuta `java <flags> -version` con timeout y devuelve la
//! versión y qué flags aceptó. `java` falla en el primer flag que no entiende,
//! así que cuando la ejecución falla se leen de `stderr` los nombres
//! (`Unrecognized VM option '-XX:LoQueSea'`, `Unrecognized option: -Xfoo`) y se
//! marcan como rechazados; el resto se consideran aceptados. Es una
//! comprobación de mejor esfuerzo, y así se documenta en el informe.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use lo_config::{JvmPreset, JvmSettings, MAX_FLAG_LEN};
use thiserror::Error;

/// Flags del preset `high`: G1 con los ajustes que pide el PvP.
pub const HIGH_FLAGS: &[&str] = &[
    "-XX:+UseG1GC",
    "-XX:+ParallelRefProcEnabled",
    "-XX:MaxGCPauseMillis=200",
    "-XX:+AlwaysPreTouch",
];
/// Flags del preset `low`.
pub const LOW_FLAGS: &[&str] = &["-XX:+UseSerialGC"];
/// Suelo de `-Xms`/`-Xmx` que produce un preset, en MB. Un perfil no baja de
/// `RAM_MIN_MB`, pero un `ram / 4` sí podría dar un número absurdo.
pub const MIN_PRESET_MB: u32 = 256;
/// Prefijos `-D` que son del launcher y no del usuario.
const OWNED_PREFIXES: &[&str] = &["-Dlunar.", "-Dlunar=", "-Dichor.", "-Dwebosr"];
/// Claves de token que nunca se pintan en una vista previa.
const TOKEN_KEYS: &[&str] = &["--accessToken", "--uuid", "--clientId", "--xuid"];
/// Tope de salida que se guarda de `java -version` (8 KiB).
const MAX_PROBE_OUTPUT: usize = 8 * 1024;

/// Plan de JVM ya resuelto: memoria, flags y avisos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JvmPlan {
    /// Java elegido (`javaw.exe` al lanzar, `java.exe` para probar flags).
    pub program: PathBuf,
    /// `-Xms` / `-Xmx`, en ese orden.
    pub memory: Vec<String>,
    /// Flags efectivos, en orden, sin los descartados.
    pub flags: Vec<String>,
    /// Lo que la UI tiene que contar.
    pub warnings: Vec<JvmWarning>,
}

impl JvmPlan {
    /// Todos los argumentos de JVM: memoria y después los flags.
    #[must_use]
    pub fn arguments(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.memory.len() + self.flags.len());
        out.extend_from_slice(&self.memory);
        out.extend_from_slice(&self.flags);
        out
    }
}

/// Veredicto de un flag escrito por el usuario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Se emite tal cual.
    Ok,
    /// Se puede usar, pero el usuario tiene que confirmarlo.
    NeedsConfirmation {
        /// El flag, recortado.
        flag: String,
        /// Por qué pide confirmación.
        reason: FlagReason,
    },
    /// No se emite nunca.
    Blocked {
        /// El flag, recortado.
        flag: String,
        /// Por qué está bloqueado.
        reason: FlagReason,
    },
    /// No es un flag utilizable (vacío, demasiado largo, con caracteres de
    /// control).
    Invalid {
        /// Lo que escribió el usuario.
        flag: String,
        /// Qué está mal.
        reason: String,
    },
}

impl Verdict {
    /// `true` si el flag se emite sin preguntar.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }

    /// `true` si el flag no se emite nunca.
    #[must_use]
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked { .. })
    }

    /// El flag del veredicto, si lo hay.
    #[must_use]
    pub fn flag(&self) -> Option<&str> {
        match self {
            Self::Ok => None,
            Self::NeedsConfirmation { flag, .. }
            | Self::Blocked { flag, .. }
            | Self::Invalid { flag, .. } => Some(flag),
        }
    }
}

/// Motivo del veredicto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagReason {
    /// `-cp`, `-classpath`, `--class-path`.
    Classpath,
    /// `-jar`.
    Jar,
    /// `-javaagent:…`.
    Agent,
    /// `-Djava.library.path=…`.
    LibraryPath,
    /// `-Dlunar.*`, `-Dichor.*`, `-Dwebosr*`: los pone el launcher.
    LauncherOwned,
    /// Token suelto que parece la clase principal.
    MainClass,
    /// `-Xbootclasspath*`.
    ReservedPrefix,
}

impl FlagReason {
    /// Texto para la UI.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Classpath => "un -cp cambia qué se ejecuta; el classpath lo pone el launcher",
            Self::Jar => "un -jar ignora el resto del comando",
            Self::Agent => "un -javaagent mete código de otra persona en la máquina virtual",
            Self::LibraryPath => {
                "-Djava.library.path lo pone el launcher: si lo cambias, el juego no abre"
            }
            Self::LauncherOwned => {
                "ese -D es del launcher (lunar/ichor/webosr); el arranque depende de él"
            }
            Self::MainClass => "un token suelto se colaría como clase principal",
            Self::ReservedPrefix => "-Xbootclasspath sustituye las clases del propio Java",
        }
    }
}

/// Aviso para la UI, sin duplicados.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JvmWarning {
    /// Código estable (`jvm.unknown_flag`, `jvm.blocked_flag`, …).
    pub code: &'static str,
    /// Texto para el usuario.
    pub message: String,
}

/// Preset tal y como lo pinta la UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JvmPresetInfo {
    /// Identificador.
    pub id: JvmPreset,
    /// Nombre corto.
    pub name: &'static str,
    /// Qué hace, en una línea.
    pub description: &'static str,
}

/// Los presets que la UI ofrece (todos menos `inherit`, que es por perfil).
#[must_use]
pub fn presets() -> &'static [JvmPresetInfo] {
    &[
        JvmPresetInfo {
            id: JvmPreset::Lunar,
            name: "Lunar por defecto",
            description: "Xms y Xmx con la RAM del perfil, sin flags extra: el comando de hoy",
        },
        JvmPresetInfo {
            id: JvmPreset::Low,
            name: "Consumo bajo",
            description: "512 MB de arranque, tope 2048 MB y recolector serie",
        },
        JvmPresetInfo {
            id: JvmPreset::High,
            name: "Rendimiento alto",
            description: "G1 con pausas cortas y memoria precargada",
        },
        JvmPresetInfo {
            id: JvmPreset::Custom,
            name: "Personalizado",
            description: "Memoria y flags escritos a mano, validados antes de usar",
        },
    ]
}

/// Ajustes efectivos de un perfil, ya resueltos contra el preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedJvm {
    xms_mb: Option<u32>,
    xmx_mb: u32,
    /// Flags que se emiten sin preguntar, en orden (preset primero).
    flags_ok: Vec<String>,
    /// Flags del usuario que necesitan decisión (confirmar, bloqueados, inválidos).
    flags: Vec<Verdict>,
    warnings: Vec<JvmWarning>,
}

impl ResolvedJvm {
    /// Resuelve un preset con la RAM del perfil.
    ///
    /// `fallback` es la configuración global: solo se usa cuando el preset es
    /// `inherit` (`effective_jvm` de un perfil). Si el global también fuera
    /// `inherit`, se resuelve como `lunar`, que es el valor de hoy.
    #[must_use]
    #[allow(clippy::similar_names)] // xms/xmx son los nombres del protocolo
    pub fn resolve(settings: &JvmSettings, fallback: &JvmSettings, ram_mb: u32) -> Self {
        let ram = ram_mb.max(MIN_PRESET_MB);
        let effective = if settings.preset == JvmPreset::Inherit {
            if fallback.preset == JvmPreset::Inherit {
                JvmPreset::Lunar
            } else {
                fallback.preset
            }
        } else {
            settings.preset
        };

        let (preset_xms, preset_xmx, preset_flags): (Option<u32>, u32, &[&str]) = match effective {
            // `lunar` es exactamente el comando de hoy: `-Xms` y `-Xmx` con la
            // RAM del perfil y ningún flag extra.
            JvmPreset::Lunar | JvmPreset::Inherit => (Some(ram), ram, &[]),
            JvmPreset::Low => (Some(512), ram.min(2048), LOW_FLAGS),
            JvmPreset::High => (Some((ram / 4).max(MIN_PRESET_MB)), ram, HIGH_FLAGS),
            JvmPreset::Custom => (None, ram, &[]),
        };

        // Lo que el usuario escribió manda sobre lo que diga el preset.
        let xms_mb = settings
            .xms_mb
            .or(preset_xms)
            .map(|mb| mb.max(MIN_PRESET_MB));
        let xmx_mb = settings.xmx_mb.unwrap_or(preset_xmx).max(MIN_PRESET_MB);

        let mut warnings = Vec::new();
        // Los flags del preset se emiten siempre; los del usuario pasan por el
        // validador.
        let mut emitted: Vec<String> = preset_flags.iter().map(|f| (*f).to_owned()).collect();
        let mut pending: Vec<Verdict> = Vec::new();

        for raw in &settings.extra_flags {
            match validate(raw) {
                Verdict::Ok => emitted.push(raw.trim().to_owned()),
                verdict => {
                    let code = match &verdict {
                        Verdict::NeedsConfirmation { .. } => "jvm.confirm_flag",
                        Verdict::Blocked { .. } => "jvm.blocked_flag",
                        Verdict::Invalid { .. } => "jvm.invalid_flag",
                        Verdict::Ok => unreachable!("Ok ya se emitió"),
                    };
                    let flag = verdict.flag().unwrap_or_default().to_owned();
                    warnings.push(JvmWarning {
                        code,
                        message: format!("«{flag}»: {}", describe(&verdict)),
                    });
                    pending.push(verdict);
                }
            }
        }

        if emitted.len() > lo_config::MAX_FLAGS {
            warnings.push(JvmWarning {
                code: "jvm.too_many_flags",
                message: format!(
                    "hay más de {} flags; se usan los primeros",
                    lo_config::MAX_FLAGS
                ),
            });
            emitted.truncate(lo_config::MAX_FLAGS);
        }

        Self {
            xms_mb,
            xmx_mb,
            flags_ok: emitted,
            flags: pending,
            warnings,
        }
    }

    /// `-Xms` y `-Xmx`, en ese orden.
    #[must_use]
    pub fn memory(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(xms) = self.xms_mb {
            out.push(format!("-Xms{xms}M"));
        }
        out.push(format!("-Xmx{}M", self.xmx_mb));
        out
    }

    /// Argumentos de JVM que se emiten sin preguntar.
    ///
    /// Los que piden confirmación no salen aquí: para incluirlos hay que pedirlo
    /// con [`ResolvedJvm::arguments_confirming`].
    #[must_use]
    pub fn arguments(&self) -> Vec<String> {
        self.arguments_confirming(false)
    }

    /// Igual que [`ResolvedJvm::arguments`], pero incluye los flags que piden
    /// confirmación cuando `confirmed` es `true`.
    #[must_use]
    pub fn arguments_confirming(&self, confirmed: bool) -> Vec<String> {
        let mut out = self.memory();
        out.extend(self.flags_ok.iter().cloned());
        if confirmed {
            for verdict in &self.flags {
                if let Verdict::NeedsConfirmation { flag, .. } = verdict {
                    out.push(flag.clone());
                }
            }
        }
        out
    }

    /// Flags que esperan confirmación del usuario.
    #[must_use]
    pub fn pending(&self) -> Vec<&Verdict> {
        self.flags
            .iter()
            .filter(|verdict| matches!(verdict, Verdict::NeedsConfirmation { .. }))
            .collect()
    }

    /// Avios acumulados (preset y flags descartados).
    #[must_use]
    pub fn warnings(&self) -> &[JvmWarning] {
        &self.warnings
    }

    /// `-Xms` en MB, si se emite.
    #[must_use]
    pub fn xms_mb(&self) -> Option<u32> {
        self.xms_mb
    }

    /// `-Xmx` en MB.
    #[must_use]
    pub fn xmx_mb(&self) -> u32 {
        self.xmx_mb
    }

    /// Vista previa del comando de JVM, sin tokens.
    ///
    /// `java <flags> -version`: es exactamente lo que se prueba con
    /// [`probe_flags`], y así el usuario ve qué se va a ejecutar. Los tokens de
    /// sesión (`--accessToken`, `--uuid`, `--clientId`, `--xuid`) se sustituyen
    /// por `***` aunque aquí no haya ninguno: la función no depende de eso.
    #[must_use]
    pub fn preview(&self, java: &Path) -> String {
        let mut parts = vec![quote(&java.display().to_string())];
        parts.extend(self.arguments().into_iter().map(|flag| redact(&flag)));
        parts.push("-version".to_owned());
        parts.join(" ")
    }

    /// Plan listo para lanzar, con el programa ya puesto.
    #[must_use]
    pub fn plan(&self, program: &Path) -> JvmPlan {
        JvmPlan {
            program: program.to_path_buf(),
            memory: self.memory(),
            flags: self.flags_ok.clone(),
            warnings: self.warnings.clone(),
        }
    }
}

/// Valida un flag escrito por el usuario.
#[must_use]
pub fn validate(raw: &str) -> Verdict {
    let flag = raw.trim();
    if flag.is_empty() {
        return Verdict::Invalid {
            flag: flag.to_owned(),
            reason: "el flag está vacío".to_owned(),
        };
    }
    if flag.chars().count() > MAX_FLAG_LEN {
        return Verdict::Invalid {
            flag: flag.to_owned(),
            reason: format!("pasa de {MAX_FLAG_LEN} caracteres"),
        };
    }
    if flag.chars().any(char::is_control) {
        return Verdict::Invalid {
            flag: flag.to_owned(),
            reason: "tiene caracteres de control".to_owned(),
        };
    }
    if !flag.starts_with('-') {
        return Verdict::Blocked {
            flag: flag.to_owned(),
            reason: FlagReason::MainClass,
        };
    }

    let lower = flag.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "-cp" | "-classpath" | "--class-path" | "-classpath:"
    ) {
        return Verdict::NeedsConfirmation {
            flag: flag.to_owned(),
            reason: FlagReason::Classpath,
        };
    }
    if lower == "-jar" {
        return Verdict::NeedsConfirmation {
            flag: flag.to_owned(),
            reason: FlagReason::Jar,
        };
    }
    if lower.starts_with("-javaagent:") || lower == "-javaagent" {
        return Verdict::NeedsConfirmation {
            flag: flag.to_owned(),
            reason: FlagReason::Agent,
        };
    }
    if lower.starts_with("-xbootclasspath") {
        return Verdict::Blocked {
            flag: flag.to_owned(),
            reason: FlagReason::ReservedPrefix,
        };
    }
    if lower == "-djava.library.path" || lower.starts_with("-djava.library.path=") {
        return Verdict::Blocked {
            flag: flag.to_owned(),
            reason: FlagReason::LibraryPath,
        };
    }
    if OWNED_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(&prefix.to_ascii_lowercase()))
    {
        return Verdict::Blocked {
            flag: flag.to_owned(),
            reason: FlagReason::LauncherOwned,
        };
    }
    Verdict::Ok
}

/// Valida una lista y devuelve los veredictos en orden.
#[must_use]
pub fn validate_all(flags: &[String]) -> Vec<Verdict> {
    flags.iter().map(|flag| validate(flag)).collect()
}

fn describe(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Ok => "correcto".to_owned(),
        Verdict::NeedsConfirmation { reason, .. } | Verdict::Blocked { reason, .. } => {
            reason.message().to_owned()
        }
        Verdict::Invalid { reason, .. } => reason.clone(),
    }
}

/// Redacta los tokens de sesión de una línea de comando.
fn redact(flag: &str) -> String {
    let (key, value) = match flag.split_once('=') {
        Some((key, value)) => (key, Some(value)),
        None => (flag, None),
    };
    if TOKEN_KEYS
        .iter()
        .any(|token| key.eq_ignore_ascii_case(token))
    {
        return format!("{key}=***");
    }
    match value {
        Some(value) => format!("{key}={value}"),
        None => key.to_owned(),
    }
}

/// Entrecomilla un argumento si lleva espacios, como haría un shell.
fn quote(argument: &str) -> String {
    if argument.contains(' ') || argument.is_empty() {
        format!("\"{argument}\"")
    } else {
        argument.to_owned()
    }
}

/// Informe de una prueba de flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    /// Versión que imprime `java -version` (primera línea útil).
    pub java_version: String,
    /// Flags que la JVM aceptó.
    pub accepted: Vec<String>,
    /// Flags rechazados, con el motivo que dio Java.
    pub rejected: Vec<(String, String)>,
    /// Tiempo que tardó la prueba.
    pub elapsed_ms: u64,
    /// Salida recortada de `stderr`, para pegarla en el informe.
    pub stderr: String,
}

impl ProbeReport {
    /// `true` si la JVM arrancó y aceptó todo.
    #[must_use]
    pub fn all_ok(&self) -> bool {
        self.rejected.is_empty()
    }
}

/// Fallos de la prueba de flags.
#[derive(Debug, Error)]
pub enum JvmError {
    /// No se pudo ejecutar el Java.
    #[error("no pude ejecutar {0}: {1}")]
    Spawn(String, String),
    /// La prueba tardó más de lo permitido.
    #[error("java tardó más de {0} ms en contestar")]
    Timeout(u64),
    /// El Java no existe o no es un fichero.
    #[error("no encuentro el Java: {}", .0.display())]
    NoJava(PathBuf),
    /// Falló la lectura de la salida.
    #[error("no pude leer la salida de java: {0}")]
    Io(String),
}

/// Prueba flags con `java <flags> -version`.
///
/// Devuelve la versión del Java y qué flags aceptó. Un `java` que se cierra con
/// error por un flag desconocido se interpreta leyendo `stderr`: los nombres que
/// Java señala se marcan como rechazados y el resto queda como aceptado. Es
/// mejor esfuerzo y así se documenta: sirve para quitar un flag que el Java del
/// usuario no entiende, no para certificar una combinación.
pub async fn probe_flags(
    java: &Path,
    flags: &[String],
    timeout: Duration,
) -> Result<ProbeReport, JvmError> {
    use tokio::process::Command;

    if !java.is_file() {
        return Err(JvmError::NoJava(java.to_path_buf()));
    }
    let started = std::time::Instant::now();
    let mut command = Command::new(java);
    command
        .args(flags)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let output = match tokio::time::timeout(timeout, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => return Err(JvmError::Spawn(java.display().to_string(), err.to_string())),
        Err(_) => {
            return Err(JvmError::Timeout(
                u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            ));
        }
    };

    let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    stderr.truncate(MAX_PROBE_OUTPUT);

    let java_version = parse_version(&stderr)
        .or_else(|| parse_version(&stdout))
        .unwrap_or_default();

    let rejected: Vec<(String, String)> = if output.status.success() {
        Vec::new()
    } else {
        let mut rejected = Vec::new();
        for line in stderr.lines() {
            if let Some(name) = unrecognized_flag(line) {
                let flag = flags
                    .iter()
                    .find(|flag| flag.trim() == name)
                    .cloned()
                    .unwrap_or(name);
                rejected.push((flag, line.trim().to_owned()));
            }
        }
        rejected
    };

    let accepted = flags
        .iter()
        .filter(|flag| !rejected.iter().any(|(bad, _)| bad == *flag))
        .cloned()
        .collect();

    Ok(ProbeReport {
        java_version,
        accepted,
        rejected,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        stderr,
    })
}

/// Saca la versión de una línea de `java -version`, que la escribe en `stderr`
/// entre comillas: `java version "1.8.0_411"`.
fn parse_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if !lower.contains("version") {
            continue;
        }
        if let Some(start) = line.find('"') {
            let rest = &line[start + 1..];
            if let Some(end) = rest.find('"') {
                return Some(rest[..end].to_owned());
            }
        }
        // Java 9+ a veces lo escribe sin comillas en la primera palabra.
        if let Some(word) = line.split_whitespace().find(|word| {
            word.chars().next().is_some_and(|c| c.is_ascii_digit()) && word.contains('.')
        }) {
            return Some(word.trim_matches(|c: char| c == '"' || c == ',').to_owned());
        }
    }
    None
}

/// Nombre del flag que Java señala como desconocido, si la línea lo hace.
fn unrecognized_flag(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let markers = [
        "unrecognized vm option '",
        "unrecognized vm option \"",
        "unrecognized option: ",
        "unrecognized option '",
    ];
    for marker in markers {
        if let Some(index) = lower.find(marker) {
            let rest = &line[index + marker.len()..];
            let tail = rest
                .split(['\'', '"', ' '])
                .next()
                .unwrap_or_default()
                .trim();
            if !tail.is_empty() {
                return Some(tail.to_owned());
            }
        }
    }
    None
}
