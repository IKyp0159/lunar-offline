//! Supervisor del proceso: lanza `javaw.exe`, sigue el log en vivo y cuenta lo
//! que pasa hasta que el juego termina.
//!
//! Es el equivalente de `subprocess.Popen` y una espera de 3 s en bucle:
//!
//! ```text
//! Popen(cmd, cwd=MULTIVER, stdout=log, stderr=STDOUT, CREATE_NO_WINDOW)
//! cada 3 s: proc.poll() -> None => sigue vivo | rc => restaurar y salir
//! ```
//!
//! Lo que añade sobre esa base:
//!
//! - El log se **lee mientras se escribe** (tail desde el byte 0), no solo al
//!   cerrar: la UI puede enseñar las líneas de ichor en directo.
//! - Se detectan cuatro fallos con síntoma inequívoco: `Loaded 4389 baked
//!   classes`, falta de `resources/`, `Invalid session` y artefacto que no
//!   carga.
//! - Si a los 45 s no apareció `[LC] Setting user: <nick>`, se avisa una vez.
//!
//! El proceso **no se mata al soltar el supervisor**: si el jugador cierra la
//! ventana del launcher, el juego sigue y la restauración de `accounts.json` la
//! hace el siguiente arranque (`Core::open`).

use std::collections::BTreeSet;
use std::io::{ErrorKind, Read};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime};

use lo_accounts::Nick;
use tokio::process::Child;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{self, MissedTickBehavior};

use crate::{
    EXPECTED_BAKED_CLASSES, HealthIssue, LaunchError, LaunchEvent, LaunchPlan, LaunchRequest,
    RunningGame,
};

/// Tiempo máximo sin `[LC] Setting user:` antes de avisar de que no hay ventana.
pub const WINDOW_TIMEOUT: Duration = Duration::from_secs(45);

/// Cada cuánto se mira el log y el proceso.
pub const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Tamaño máximo de una línea del log; lo que pase de aquí se corta.
pub const MAX_LINE_BYTES: usize = 8 * 1024;

/// Ajustes del vigilante: intervalos de sondeo y plazos.
#[derive(Debug, Clone)]
pub struct WatchOptions {
    /// Margen para ver `[LC] Setting user:` antes de avisar.
    pub window_timeout: Duration,
    /// Periodo de lectura del log y de `wait`.
    pub poll: Duration,
    /// Longitud máxima de una línea.
    pub max_line_bytes: usize,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            window_timeout: WINDOW_TIMEOUT,
            poll: POLL_INTERVAL,
            max_line_bytes: MAX_LINE_BYTES,
        }
    }
}

/// Juego lanzado, con su flujo de sucesos.
///
/// `next_event` devuelve `None` cuando el proceso ya terminó y el log se vació;
/// el último suceso es siempre [`LaunchEvent::Exited`].
#[derive(Debug)]
pub struct Supervisor {
    game: RunningGame,
    events: mpsc::UnboundedReceiver<LaunchEvent>,
    worker: JoinHandle<()>,
}

impl Supervisor {
    /// Arranca el juego descrito por `plan` y empieza a vigilarlo.
    ///
    /// El primer suceso del flujo es [`LaunchEvent::Started`] con el PID real.
    ///
    /// Es síncrona a propósito: crear el proceso y la tarea de vigilancia no
    /// espera a nada; el que espera es [`Supervisor::next_event`]. Tiene que
    /// llamarse desde dentro de un runtime de Tokio (`tokio::spawn`).
    pub fn start(plan: &LaunchPlan, request: &LaunchRequest) -> Result<Self, LaunchError> {
        Self::start_with(plan, request, &WatchOptions::default())
    }

    /// Igual que [`Supervisor::start`], con tiempos a medida (tests).
    pub fn start_with(
        plan: &LaunchPlan,
        request: &LaunchRequest,
        options: &WatchOptions,
    ) -> Result<Self, LaunchError> {
        let mut child = spawn_child(plan)?;
        let pid = child
            .id()
            .ok_or_else(|| LaunchError::Spawn(std::io::Error::other("el proceso no tiene PID")))?;

        let game = RunningGame {
            pid,
            started_at: SystemTime::now(),
            log_path: plan.log_path.clone(),
            version: plan.version.to_owned(),
            nick: request.nick.as_str().to_owned(),
        };

        let (tx, events) = mpsc::unbounded_channel();
        // Si nadie escucha todavía, el suceso se pierde: es un canal de avisos,
        // no un diario. `RunningGame` se devuelve también por valor.
        let _ = tx.send(LaunchEvent::Started(game.clone()));

        let nick = Nick::parse(request.nick.as_str()).map_err(LaunchError::Nick)?;
        let log_path = plan.log_path.clone();
        let options = options.clone();
        let worker = tokio::spawn(async move {
            watch(&mut child, &log_path, &nick, &options, &tx).await;
        });

        Ok(Self {
            game,
            events,
            worker,
        })
    }

    /// Datos del proceso lanzado.
    #[must_use]
    pub fn game(&self) -> &RunningGame {
        &self.game
    }

    /// Siguiente suceso; `None` cuando el flujo se cierra.
    pub async fn next_event(&mut self) -> Option<LaunchEvent> {
        self.events.recv().await
    }

    /// Cierra el flujo de sucesos y deja que el proceso viva.
    pub fn detach(self) {
        drop(self.events);
        drop(self.worker);
    }
}

/// Crea el fichero de log y lanza el proceso con su salida redirigida.
fn spawn_child(plan: &LaunchPlan) -> Result<Child, LaunchError> {
    if let Some(parent) = plan.log_path.parent() {
        std::fs::create_dir_all(parent).map_err(LaunchError::Io)?;
    }
    std::fs::create_dir_all(&plan.workdir).map_err(LaunchError::Io)?;

    let stdout = std::fs::File::create(&plan.log_path).map_err(LaunchError::Io)?;
    let stderr = stdout.try_clone().map_err(LaunchError::Io)?;

    let mut command = tokio::process::Command::new(&plan.program);
    command
        .args(&plan.args)
        .current_dir(&plan.workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .kill_on_drop(false);

    #[cfg(windows)]
    {
        // `CREATE_NO_WINDOW`: nada de consola parpadeando.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command.spawn().map_err(LaunchError::Spawn)
}

/// Bucle del vigilante: log en vivo, salud y salida del proceso.
async fn watch(
    child: &mut Child,
    log_path: &Path,
    nick: &Nick,
    options: &WatchOptions,
    tx: &mpsc::UnboundedSender<LaunchEvent>,
) {
    let started = Instant::now();
    let mut ticker = time::interval(options.poll);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut tail = Tail::open(log_path);
    let mut health = HealthState::default();

    loop {
        tokio::select! {
            status = child.wait() => {
                // Últimas líneas que quedaran en el fichero antes de morir.
                for _ in 0..4 {
                    let lines = tail.read(options.max_line_bytes);
                    if lines.is_empty() {
                        break;
                    }
                    for line in lines {
                        if let Some(issue) = health.observe(&line, nick.as_str()) {
                            let _ = tx.send(LaunchEvent::Warning(issue));
                        }
                        let _ = tx.send(LaunchEvent::LogLine(line));
                    }
                }
                let code = status.ok().and_then(|status| status.code());
                let _ = tx.send(LaunchEvent::Exited {
                    code,
                    uptime_secs: started.elapsed().as_secs(),
                });
                return;
            }
            _ = ticker.tick() => {
                for line in tail.read(options.max_line_bytes) {
                    if let Some(issue) = health.observe(&line, nick.as_str()) {
                        let _ = tx.send(LaunchEvent::Warning(issue));
                    }
                    let _ = tx.send(LaunchEvent::LogLine(line));
                }
                if health.user.is_none()
                    && !health.window_warned
                    && started.elapsed() >= options.window_timeout
                {
                    health.window_warned = true;
                    let _ = tx.send(LaunchEvent::Warning(HealthIssue::NoWindowAfter {
                        secs: options.window_timeout.as_secs(),
                    }));
                }
            }
        }
    }
}

/// Cola de lectura del log: bytes sueltos, líneas ya decodificadas.
#[derive(Debug)]
struct Tail {
    file: Option<std::fs::File>,
    pending: Vec<u8>,
}

impl Tail {
    fn open(path: &Path) -> Self {
        Self {
            file: std::fs::File::open(path).ok(),
            pending: Vec::new(),
        }
    }

    /// Lee lo que haya nuevo y devuelve las líneas completas.
    fn read(&mut self, max_line: usize) -> Vec<String> {
        let mut fresh = Vec::new();
        if let Some(file) = self.file.as_mut() {
            let mut chunk = [0_u8; 8192];
            loop {
                match file.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(len) => fresh.extend_from_slice(&chunk[..len]),
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        }
        self.pending.extend_from_slice(&fresh);
        self.split(max_line)
    }

    fn split(&mut self, max_line: usize) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(position) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=position).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            out.push(decode(&line, max_line));
        }
        if self.pending.len() > max_line {
            // Línea que no termina: se corta para no crecer sin freno.
            let line = std::mem::take(&mut self.pending);
            out.push(decode(&line, max_line));
        }
        out
    }
}

fn decode(bytes: &[u8], max_line: usize) -> String {
    let slice = if bytes.len() > max_line {
        &bytes[..max_line]
    } else {
        bytes
    };
    String::from_utf8_lossy(slice).into_owned()
}

/// Lo que el vigilante ya vio en el log, para no repetir avisos.
#[derive(Debug, Default)]
struct HealthState {
    user: Option<String>,
    window_warned: bool,
    vanilla_warned: bool,
    session_warned: bool,
    missing: BTreeSet<String>,
}

impl HealthState {
    /// Una línea, como mucho un aviso.
    fn observe(&mut self, line: &str, expected_nick: &str) -> Option<HealthIssue> {
        if let Some(user) = setting_user(line) {
            let mismatch = !user.eq_ignore_ascii_case(expected_nick);
            self.user = Some(user.clone());
            return mismatch.then(|| HealthIssue::WrongUser {
                expected: expected_nick.to_owned(),
                actual: user,
            });
        }
        if line.contains("Invalid session") {
            if self.session_warned {
                return None;
            }
            self.session_warned = true;
            return Some(HealthIssue::PremiumServerRefused);
        }
        if let Some(classes) = baked_classes(line) {
            if classes >= EXPECTED_BAKED_CLASSES || self.vanilla_warned {
                return None;
            }
            self.vanilla_warned = true;
            return Some(HealthIssue::VanillaClasspath { classes });
        }
        if let Some(name) = missing_artifact(line) {
            if self.missing.insert(name.clone()) {
                return Some(HealthIssue::MissingArtifact(name));
            }
        }
        None
    }
}

/// `[LC] Setting user: Ana` → `Ana`.
fn setting_user(line: &str) -> Option<String> {
    let (_, rest) = line.split_once("Setting user:")?;
    let raw = rest.split_whitespace().next()?;
    let name = raw.trim_matches(|c: char| c == '"' || c == '\'' || c == ',');
    if name.is_empty() {
        return None;
    }
    Some(name.to_owned())
}

/// `Loaded 4389 baked classes.` → `4389`.
fn baked_classes(line: &str) -> Option<u32> {
    if !line.contains("baked classes") {
        return None;
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    let index = words.iter().position(|word| *word == "baked")?;
    let value = words.get(index.checked_sub(1)?)?;
    value
        .trim_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .ok()
}

/// Artefacto del classpath que no carga, tal cual lo nombra la JVM.
fn missing_artifact(line: &str) -> Option<String> {
    const MARKERS: [&str; 2] = [
        "ClassNotFoundException: ",
        "Could not find or load main class ",
    ];
    for marker in MARKERS {
        if let Some((_, rest)) = line.split_once(marker) {
            let name = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_end_matches(['.', ',', ':']);
            if !name.is_empty() {
                return Some(name.to_owned());
            }
        }
    }
    None
}
