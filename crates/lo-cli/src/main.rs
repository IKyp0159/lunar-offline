//! Binario headless: delega todo en `lo_cli`.

use std::process::ExitCode;

fn main() -> ExitCode {
    lo_cli::run_from_env_blocking().unwrap_or_else(|| {
        let mut comando = lo_cli::comando();
        let _ = comando.print_help();
        ExitCode::FAILURE
    })
}
