//! `lunar-offline`: la misma fachada que usa la app, sin ventana.
//!
//! La biblioteca la comparten dos binarios: `lunar-offline-cli` (uso headless)
//! y `lunar-offline` (la app Tauri, que decide entre GUI y CLI según los
//! argumentos). Comandos:
//!
//! ```text
//! lunar-offline selftest [--out <ruta>]        informe de soporte, sin UI
//! lunar-offline launch --nick <nick> [--dry-run] arranque headless
//! lunar-offline restore                        recupera accounts.json
//! lunar-offline versions                       versiones instaladas
//! lunar-offline diagnose [--out <ruta>]        informe en texto
//! ```
//!
//! `--selftest` se acepta como alias de `selftest`. Todo pasa por
//! [`lo_core::Core`], igual que la app: la CLI no reimplementa nada.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use lo_accounts::Nick;
use lo_core::{Core, CoreEvent, GameState};
use lo_diag::{DiagnosticReport, Status};
use lo_launch::{LaunchEvent, LaunchRequest};
use thiserror::Error;

/// Launcher de Lunar Client para jugadores no premium.
#[derive(Debug, Parser)]
#[command(
    name = "lunar-offline",
    version,
    about = "Launcher de Lunar Client para jugadores no premium",
    long_about = None
)]
struct Cli {
    /// Subcomando a ejecutar.
    #[command(subcommand)]
    command: Command,
}

/// Subcomandos disponibles.
#[derive(Debug, Subcommand)]
enum Command {
    /// Modo soporte: escribe el informe y sale sin abrir ventana.
    Selftest {
        /// Dónde escribir el informe (por defecto `%APPDATA%\LunarOffline\selftest.txt`).
        #[arg(long, value_name = "RUTA")]
        out: Option<PathBuf>,
    },
    /// Arranca el juego sin interfaz.
    Launch {
        /// Nick no premium (`A-Z`, `a-z`, `0-9`, `_`; 1-16).
        #[arg(long, value_name = "NICK")]
        nick: String,
        /// Versión instalada.
        #[arg(long, default_value = "1.8.9", value_name = "VERSION")]
        version: String,
        /// Servidor al que entrar al arrancar (`host[:puerto]`).
        #[arg(long, value_name = "SERVIDOR")]
        server: Option<String>,
        /// RAM asignada al juego, en MB.
        #[arg(long, default_value_t = 3072, value_name = "MB")]
        ram: u32,
        /// Solo valida y pinta el comando; no lanza el proceso.
        #[arg(long)]
        dry_run: bool,
    },
    /// Restaura `accounts.json` si quedó un swap a medias.
    Restore,
    /// Lista las versiones instaladas.
    Versions,
    /// Genera el informe de diagnóstico en texto.
    Diagnose {
        /// Dónde escribir el informe (por defecto, solo por pantalla).
        #[arg(long, value_name = "RUTA")]
        out: Option<PathBuf>,
    },
}

/// ¿Los argumentos actuales piden la CLI en vez de la ventana?
///
/// Solo los subcomandos conocidos (y `--selftest`) abren la consola. Un doble
/// clic no trae argumentos y arranca la interfaz; cualquier otro argumento se
/// ignora para no romper lanzamientos desde el Explorador.
#[must_use]
pub fn es_invocacion_cli() -> bool {
    let args: Vec<OsString> = std::env::args_os().collect();
    es_invocacion_cli_en(&args)
}

/// Ejecuta la CLI si los argumentos la piden. `None` = hay que abrir la GUI.
///
/// Crea su propio runtime de Tokio porque la app llama a esta función antes de
/// arrancar Tauri, y `#[tokio::main]` no se puede combinar con el bucle de la
/// ventana.
#[must_use]
pub fn run_from_env_blocking() -> Option<ExitCode> {
    let args = normalizar(std::env::args_os().collect());
    if !es_invocacion_cli_en(&args) {
        return None;
    }

    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let _ = error.print();
            return Some(if error.use_stderr() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            });
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("lunar-offline: no pude crear el runtime de Tokio: {error}");
            return Some(ExitCode::FAILURE);
        }
    };

    Some(match runtime.block_on(run(cli)) {
        Ok(codigo) => codigo,
        Err(error) => {
            eprintln!("lunar-offline: {error}");
            ExitCode::FAILURE
        }
    })
}

/// Definición de argumentos, para que el binario headless pueda pintar la ayuda.
#[must_use]
pub fn comando() -> clap::Command {
    use clap::CommandFactory;
    Cli::command()
}

/// `--selftest` pasa a `selftest`; el resto se deja tal cual.
fn normalizar(mut args: Vec<OsString>) -> Vec<OsString> {
    if let Some(primero) = args.get_mut(1) {
        if primero == "--selftest" {
            *primero = OsString::from("selftest");
        }
    }
    args
}

/// Comprueba el primer argumento de una lista ya preparada.
fn es_invocacion_cli_en(args: &[OsString]) -> bool {
    matches!(
        args.get(1).and_then(|arg| arg.to_str()),
        Some(
            "selftest"
                | "--selftest"
                | "launch"
                | "restore"
                | "versions"
                | "diagnose"
                | "help"
                | "-h"
                | "--help"
                | "-V"
                | "--version"
        )
    )
}

async fn run(cli: Cli) -> Result<ExitCode, CliError> {
    match cli.command {
        Command::Selftest { out } => selftest(out).await,
        Command::Launch {
            nick,
            version,
            server,
            ram,
            dry_run,
        } => launch(&nick, &version, server.as_deref(), ram, dry_run).await,
        Command::Restore => restore().await,
        Command::Versions => versions().await,
        Command::Diagnose { out } => diagnose(out).await,
    }
}

/// Informe de soporte en texto plano.
async fn selftest(out: Option<PathBuf>) -> Result<ExitCode, CliError> {
    let core = Core::open().await?;
    let report = core.diagnose().await?;
    let target = out.unwrap_or_else(|| core.paths().selftest_file().to_path_buf());
    report.write_to(&target)?;

    println!("informe: {}", target.display());
    println!("estado: {}", status_label(report.worst()));
    println!("{}", summary(&report));
    Ok(exit_for(report.worst()))
}

/// Diagnóstico por stdout (y, si se pide, también en un fichero).
async fn diagnose(out: Option<PathBuf>) -> Result<ExitCode, CliError> {
    let core = Core::open().await?;
    let report = core.diagnose().await?;

    print!("{}", report.render_text());
    if let Some(target) = out {
        report.write_to(&target)?;
        eprintln!("informe: {}", target.display());
    }
    Ok(exit_for(report.worst()))
}

/// Arranque headless: con `dry_run`, valida y pinta; sin él, lanza y espera.
async fn launch(
    nick: &str,
    version: &str,
    server: Option<&str>,
    ram: u32,
    dry_run: bool,
) -> Result<ExitCode, CliError> {
    let mut core = Core::open().await?;
    if !core.versions().iter().any(|spec| spec.name == version) {
        return Err(CliError::Version(
            version.to_owned(),
            installed_names(&core),
        ));
    }
    let request = LaunchRequest {
        version: version.to_owned(),
        ram_mb: ram,
        server: server.unwrap_or_default().to_owned(),
        ..LaunchRequest::new(Nick::parse(nick)?)
    };

    if dry_run {
        let plan = core.plan_for(&request)?;
        print!("{}", plan.render());
        eprintln!(
            "dry-run: no se lanzó nada · {} argumentos · cwd {}",
            plan.arg_count(),
            plan.workdir.display()
        );
        return Ok(ExitCode::SUCCESS);
    }

    let mut events = core.subscribe();
    let game = core.launch(request).await?;
    println!("arrancado: pid {} · {} · {nick}", game.pid, game.version);

    let mut salida = ExitCode::SUCCESS;
    while let Ok(event) = events.recv().await {
        match event {
            CoreEvent::Launch(LaunchEvent::Warning(issue)) => {
                eprintln!("aviso: {} — {}", issue.message(), issue.hint());
            }
            CoreEvent::Launch(LaunchEvent::Exited {
                code: Some(exit), ..
            }) => {
                println!("el juego terminó con código {exit}");
                if exit != 0 {
                    salida = ExitCode::FAILURE;
                }
            }
            CoreEvent::State(GameState::Idle) => break,
            CoreEvent::State(GameState::Error { message }) => return Err(CliError::State(message)),
            _ => {}
        }
    }

    println!("cuenta restaurada");
    Ok(salida)
}

/// Restaura `accounts.json` si hay un swap pendiente.
///
/// La restauración ocurre al abrir: si `Core::open` la hizo, aquí se cuenta.
async fn restore() -> Result<ExitCode, CliError> {
    let core = Core::open().await?;

    match core.recovery() {
        None => println!("nada que restaurar: accounts.json está intacto"),
        Some(outcome) => match outcome {
            lo_accounts::RestoreOutcome::NothingPending => {
                println!("nada que restaurar: accounts.json está intacto");
            }
            lo_accounts::RestoreOutcome::Restored {
                active_local_id,
                removed,
                from_backup,
            } => {
                let active = active_local_id.as_deref().unwrap_or("(ninguna)");
                println!("restaurado: cuenta activa {active}");
                if !removed.is_empty() {
                    println!("cuentas temporales retiradas: {}", removed.join(", "));
                }
                if *from_backup {
                    println!("accounts.json no se pudo leer: se usó la copia intacta");
                }
            }
            lo_accounts::RestoreOutcome::SessionEnded {
                remaining,
                active_local_id,
            } => {
                let active = active_local_id.as_deref().unwrap_or("(ninguna)");
                println!("sesión cerrada: quedan {remaining} abiertas · cuenta activa {active}");
            }
        },
    }
    Ok(ExitCode::SUCCESS)
}

/// Versiones que el árbol tiene instaladas de verdad.
async fn versions() -> Result<ExitCode, CliError> {
    let core = Core::open().await?;
    let installed = core.versions();

    if installed.is_empty() {
        println!(
            "ninguna versión instalada en {}",
            core.paths().multiver().display()
        );
        return Ok(ExitCode::FAILURE);
    }
    for spec in installed {
        println!(
            "{} · perfil {} · {}",
            spec.name, spec.subversion, spec.external
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Nombres de las versiones instaladas, para el mensaje de error.
fn installed_names(core: &Core) -> String {
    let names: Vec<&str> = core.versions().iter().map(|spec| spec.name).collect();
    if names.is_empty() {
        return "ninguna instalada".to_owned();
    }
    names.join(", ")
}

/// Resumen de una línea para el final de `selftest`.
fn summary(report: &DiagnosticReport) -> String {
    let problems: Vec<&str> = report
        .sections
        .iter()
        .flat_map(|section| section.items.iter())
        .filter(|item| matches!(item.status, Status::Warn | Status::Fail))
        .map(|item| item.label.as_str())
        .collect();
    if problems.is_empty() {
        "sin avisos".to_owned()
    } else {
        format!("revisar: {}", problems.join(", "))
    }
}

const fn exit_for(status: Status) -> ExitCode {
    match status {
        Status::Fail => ExitCode::FAILURE,
        _ => ExitCode::SUCCESS,
    }
}

const fn status_label(status: Status) -> &'static str {
    match status {
        Status::Ok => "ok",
        Status::Warn => "aviso",
        Status::Fail => "fallo",
        Status::Info => "info",
    }
}

/// Fallos de la CLI, con mensaje ya listo para el usuario.
#[derive(Debug, Error)]
enum CliError {
    /// Cualquier fallo de la fachada (rutas, configuración, cuentas, arranque).
    #[error("{0}")]
    Core(#[from] lo_core::CoreError),
    /// El informe no se pudo generar o escribir.
    #[error("{0}")]
    Diag(#[from] lo_diag::DiagError),
    /// El nick no es válido.
    #[error("{0}")]
    Nick(#[from] lo_accounts::NickError),
    /// El juego falló sin devolver un error de la fachada.
    #[error("{0}")]
    State(String),
    /// La versión pedida no está instalada.
    #[error("la versión {0} no está instalada ({1})")]
    Version(String, String),
}
