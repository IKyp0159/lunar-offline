//! Diagnóstico estructurado: los mismos datos que pinta el informe de soporte,
//! pero en tipos para que la UI y la CLI los presenten cada una a su manera.
//!
//! `%APPDATA%\LunarOffline\selftest.txt` lleva estas líneas:
//!
//! ```text
//! rust: lunar-offline 2.0.0 perfil=release
//! lunar_root: ... exists=True
//! multiver: ... exists=True
//! java: ... ok=True
//! natives: ...
//! minecraft: ... exists=True
//! accounts: ... exists=True
//! launcher_oficial_abierto: True
//! swap_pending: False
//! cuentas_registradas: 3
//! versions: ['1.8.9', '1.7.10']
//! build_command[1.8.9]: OK (69 args)
//! build_command[1.7.10]: OK (69 args)
//! ```
//!
//! Regla: el informe **no** puede contener tokens, rutas de cuentas ni
//! contenidos de `accounts.json`. Solo existencia, conteos y resultado del
//! comando.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use lo_accounts::{AccountsManager, Nick};
use lo_detect::{COMMON_JARS, Environment, detect};
use lo_launch::{LaunchRequest, build_plan};
use lo_paths::LunarPaths;
use serde::Serialize;
use thiserror::Error;

/// Nick fijo con el que se valida el comando.
const SELFTEST_NICK: &str = "TestOffline";

/// Servidor fijo: fuerza el par `--server` en el comando de prueba.
const SELFTEST_SERVER: &str = "example.com:25565";

/// Sello de tiempo para los nombres de log; en el informe no se muestra.
const SELFTEST_STAMP: &str = "selftest";

/// Estado de una línea del informe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    /// Todo bien.
    Ok,
    /// Funciona, pero hay algo que revisar.
    Warn,
    /// No funciona; el launcher no podrá arrancar el juego.
    Fail,
    /// Dato informativo, ni bien ni mal.
    Info,
}

impl Status {
    /// Marca textual que se añade al informe cuando hay algo que revisar.
    ///
    /// `Ok` e `Info` no llevan marca: un informe sano queda limpio.
    #[must_use]
    pub fn tag(self) -> Option<&'static str> {
        match self {
            Self::Ok | Self::Info => None,
            Self::Warn => Some("[AVISO]"),
            Self::Fail => Some("[FALLO]"),
        }
    }

    fn severity(self) -> u8 {
        match self {
            Self::Fail => 3,
            Self::Warn => 2,
            Self::Info | Self::Ok => 0,
        }
    }
}

/// Una línea del informe: qué se comprobó y qué salió.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportItem {
    /// Clave en snake_case (`lunar_root`, `swap_pending`, ...).
    pub label: String,
    /// Valor ya formateado.
    pub value: String,
    /// Estado de la comprobación.
    pub status: Status,
}

impl ReportItem {
    /// Línea nueva del informe.
    #[must_use]
    pub fn new(label: impl Into<String>, value: impl Into<String>, status: Status) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            status,
        }
    }
}

/// Un bloque del informe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportSection {
    /// Título del bloque (`Entorno`, `Descubrimiento`, `Cuentas`, `Comandos`).
    pub title: String,
    /// Líneas del bloque.
    pub items: Vec<ReportItem>,
}

/// Informe completo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticReport {
    /// Versión del launcher que generó el informe.
    pub launcher_version: String,
    /// Momento de generación, epoch en segundos (UTC).
    pub generated_at_unix: i64,
    /// Bloques del informe, en orden de lectura.
    pub sections: Vec<ReportSection>,
}

impl DiagnosticReport {
    /// Estado global: el peor de todas las líneas.
    ///
    /// `Info` y `Ok` pesan lo mismo, así que un informe sin avisos devuelve
    /// `Ok` aunque todas sus líneas sean informativas.
    #[must_use]
    pub fn worst(&self) -> Status {
        self.sections
            .iter()
            .flat_map(|section| section.items.iter())
            .map(|item| item.status)
            .fold(Status::Ok, |worst, status| {
                if status.severity() > worst.severity() {
                    status
                } else {
                    worst
                }
            })
    }

    /// Informe en texto plano, listo para `selftest.txt`.
    ///
    /// Formato: un bloque por sección, líneas `label: value`, y la marca de
    /// estado solo cuando hace falta. Sin cabecera de sección si solo hay una.
    #[must_use]
    pub fn render_text(&self) -> String {
        let mut out = String::with_capacity(512);
        let single = self.sections.len() == 1;
        for (idx, section) in self.sections.iter().enumerate() {
            if idx > 0 {
                out.push('\n');
            }
            if !single {
                out.push_str("# ");
                out.push_str(&section.title);
                out.push('\n');
            }
            for item in &section.items {
                out.push_str(&item.label);
                out.push_str(": ");
                out.push_str(&item.value);
                if let Some(tag) = item.status.tag() {
                    out.push(' ');
                    out.push_str(tag);
                }
                out.push('\n');
            }
        }
        out
    }

    /// Guarda el informe en texto plano.
    pub fn write_to(&self, path: &Path) -> Result<(), DiagError> {
        lo_paths::write_atomic(path, self.render_text().as_bytes())?;
        Ok(())
    }
}

/// Construye el informe completo leyendo el estado real del equipo.
///
/// Una sola sección (`selftest`) para que el texto plano quede línea a línea.
pub fn build(paths: &LunarPaths) -> Result<DiagnosticReport, DiagError> {
    let env = detect(paths);
    let manager = AccountsManager::new(paths.clone());

    let mut items = environment_items(paths, &env, &manager);
    items.extend(command_items(paths, &env));

    Ok(DiagnosticReport {
        launcher_version: env!("CARGO_PKG_VERSION").to_owned(),
        generated_at_unix: generated_at_unix(),
        sections: vec![ReportSection {
            title: "selftest".to_owned(),
            items,
        }],
    })
}

/// Líneas 1-11: entorno, rutas, java, natives, cuentas y versiones.
fn environment_items(
    paths: &LunarPaths,
    env: &Environment,
    manager: &AccountsManager,
) -> Vec<ReportItem> {
    let lunar_root = paths.lunar_root();
    let multiver = paths.multiver();
    let minecraft = paths.minecraft_dir();
    let accounts = paths.accounts_file();

    let mut items = Vec::with_capacity(11);
    items.push(ReportItem::new(
        "rust",
        format!(
            "lunar-offline {version} perfil={profile}",
            version = env!("CARGO_PKG_VERSION"),
            profile = profile_name()
        ),
        Status::Info,
    ));
    items.push(ReportItem::new(
        "lunar_root",
        format!(
            "{} exists={}",
            lunar_root.display(),
            pybool(lunar_root.is_dir())
        ),
        flag(lunar_root.is_dir(), Status::Ok, Status::Fail),
    ));
    items.push(ReportItem::new(
        "multiver",
        format!(
            "{} exists={}",
            multiver.display(),
            pybool(multiver.is_dir())
        ),
        flag(multiver.is_dir(), Status::Ok, Status::Fail),
    ));
    items.push(ReportItem::new(
        "java",
        java_value(env),
        flag(env.java.is_some(), Status::Ok, Status::Fail),
    ));
    items.push(ReportItem::new(
        "natives",
        natives_value(env),
        natives_status(env),
    ));
    items.push(ReportItem::new(
        "minecraft",
        format!(
            "{} exists={}",
            minecraft.display(),
            pybool(minecraft.is_dir())
        ),
        flag(minecraft.is_dir(), Status::Ok, Status::Warn),
    ));
    items.push(ReportItem::new(
        "accounts",
        format!(
            "{} exists={}",
            accounts.display(),
            pybool(accounts.is_file())
        ),
        flag(accounts.is_file(), Status::Ok, Status::Info),
    ));

    let launcher_open = manager.official_launcher_running();
    items.push(ReportItem::new(
        "launcher_oficial_abierto",
        pybool(launcher_open).to_owned(),
        flag(launcher_open, Status::Warn, Status::Ok),
    ));

    let swap_pending = manager.pending();
    items.push(ReportItem::new(
        "swap_pending",
        pybool(swap_pending).to_owned(),
        flag(swap_pending, Status::Warn, Status::Ok),
    ));

    let registered = manager.read().accounts().map_or(0, serde_json::Map::len);
    items.push(ReportItem::new(
        "cuentas_registradas",
        registered.to_string(),
        Status::Info,
    ));

    items.push(ReportItem::new(
        "versions",
        versions_value(env),
        flag(!env.versions.is_empty(), Status::Ok, Status::Fail),
    ));

    items
}

/// Últimas líneas: `build_command[<v>]` y los artefactos que falten.
fn command_items(paths: &LunarPaths, env: &Environment) -> Vec<ReportItem> {
    let mut items = Vec::with_capacity(env.versions.len() * 2);
    for spec in &env.versions {
        let request = selftest_request(env, spec.name);
        let value = match build_plan(paths, env, spec, &request, SELFTEST_STAMP) {
            // `len(cmd)` incluye el ejecutable en `cmd[0]`.
            // Se replica para que las dos líneas digan lo mismo.
            Ok(plan) => format!("OK ({} args)", plan.arg_count() + 1),
            Err(error) => format!("FAIL {error}"),
        };
        let ok = value.starts_with("OK");
        items.push(ReportItem::new(
            format!("build_command[{}]", spec.name),
            value,
            flag(ok, Status::Ok, Status::Fail),
        ));
    }

    // `classpath_jars` solo devuelve ficheros existentes, así
    // que su rama `FALTA` nunca se ejecuta. Aquí se comprueba la lista esperada.
    for spec in &env.versions {
        for name in std::iter::once(spec.mapping).chain(COMMON_JARS) {
            if !paths.multiver().join(name).is_file() {
                items.push(ReportItem::new(
                    format!("FALTA {}", spec.name),
                    name.to_owned(),
                    Status::Fail,
                ));
            }
        }
    }

    items
}

/// Petición con valores fijos (`TestOffline`, 2048 MB, 854x480).
fn selftest_request(env: &Environment, version: &str) -> LaunchRequest {
    LaunchRequest {
        version: version.to_owned(),
        ram_mb: 2048,
        server: SELFTEST_SERVER.to_owned(),
        width: 854,
        height: 480,
        fullscreen: false,
        // Sin JRE la línea `java:` ya falla; el comando se prueba igual para
        // informar del número de argumentos.
        jvm: None,
        jvm_global: None,
        jvm_confirm: false,
        java: Some(
            env.java
                .as_ref()
                .map_or_else(std::path::PathBuf::new, |java| java.path.clone()),
        ),
        ..LaunchRequest::new(Nick::parse(SELFTEST_NICK).expect("nick fijo válido"))
    }
}

/// Valor de la línea `java` (`ruta ok=True`).
fn java_value(env: &Environment) -> String {
    env.java.as_ref().map_or_else(
        || "None ok=False".to_owned(),
        |java| format!("{} ok={}", java.path.display(), pybool(java.path.is_file())),
    )
}

/// Valor de la línea `natives` (solo la ruta).
fn natives_value(env: &Environment) -> String {
    env.natives.as_ref().map_or_else(
        || "None".to_owned(),
        |natives| natives.path.display().to_string(),
    )
}

fn natives_status(env: &Environment) -> Status {
    match env.natives.as_ref() {
        None => Status::Fail,
        Some(natives) if natives.has_resources => Status::Ok,
        Some(_) => Status::Warn,
    }
}

/// Valor de la línea `versions`, como lista entre corchetes.
fn versions_value(env: &Environment) -> String {
    let names: Vec<String> = env
        .versions
        .iter()
        .map(|spec| format!("'{}'", spec.name))
        .collect();
    format!("[{}]", names.join(", "))
}

/// `True`/`False`.
fn pybool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

fn flag(value: bool, when_true: Status, when_false: Status) -> Status {
    if value { when_true } else { when_false }
}

const fn profile_name() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn generated_at_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(0)
}

/// Fallos al generar el informe.
#[derive(Debug, Error)]
pub enum DiagError {
    /// Fallo de E/S al escribir el informe.
    #[error("no pude escribir el informe: {0}")]
    Io(#[from] std::io::Error),
    /// El launcher no pudo abrir sus rutas.
    #[error("no pude resolver las rutas del launcher: {0}")]
    Paths(#[from] lo_paths::PathsError),
}
