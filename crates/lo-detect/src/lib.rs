//! Detección de todo lo que hay en disco: JRE, natives, artefactos y versiones.
//!
//! Resuelve el JRE, los natives, el classpath y las versiones instaladas con las
//! prioridades y el orden de artefactos que espera Genesis.
//!
//! Nada de manifiestos hardcodeados: lo que no está en disco no existe. Las
//! únicas tablas fijas son las de Lunar ([`VERSIONS`], [`COMMON_JARS`],
//! [`JAVA_FLAGS`]), que forman parte del contrato del launcher oficial.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

use lo_paths::LunarPaths;
use thiserror::Error;

/// Profundidad máxima al buscar `javaw.exe` dentro de `~/.lunarclient/jre`.
///
/// La estructura real es `jre/<hash>/<jdk>/bin/javaw.exe`, cuatro niveles. El
/// límite evita que un enlace o un árbol raro cuelgue el arranque.
const MAX_JRE_DEPTH: usize = 8;

/// Tabla de versiones soportadas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionSpec {
    /// Nombre largo que ve el usuario (`1.8.9`, `1.7.10`).
    pub name: &'static str,
    /// Subversión que espera `--version` (`1.8`, `1.7`).
    pub subversion: &'static str,
    /// Jar de mappings propio de la versión.
    pub mapping: &'static str,
    /// OptiFine externo que recibe `--ichorExternalFiles`.
    pub external: &'static str,
    /// Índice de assets que espera `--assetIndex`.
    pub asset_index: &'static str,
}

/// Versiones soportadas, en orden ascendente.
pub const VERSIONS: [VersionSpec; 2] = [
    VersionSpec {
        name: "1.8.9",
        subversion: "1.8",
        mapping: "lunar-platform-mappings-v1_8.jar",
        external: "OptiFine_v1_8.jar",
        asset_index: "1.8",
    },
    VersionSpec {
        name: "1.7.10",
        subversion: "1.7",
        mapping: "lunar-platform-mappings-v1_7.jar",
        external: "OptiFine_v1_7.jar",
        asset_index: "1.7.10",
    },
];

/// Artefactos comunes del classpath, en el orden exacto que usa el launcher
/// oficial. `lunar.jar` va al final, siempre.
pub const COMMON_JARS: [&str; 8] = [
    "lunar-platform-mappings-v26_2.jar",
    "optifine-0.1.0-SNAPSHOT-all.jar",
    "common-0.1.0-SNAPSHOT-all-nomappings.jar",
    "legacy-0.1.0-SNAPSHOT-all-nomappings.jar",
    "modern-0.1.0-SNAPSHOT-all-nomappings.jar",
    "lunar-lang.jar",
    "genesis-0.1.0-SNAPSHOT-all.jar",
    "lunar.jar",
];

/// Flags JVM base, tal como los pasa el launcher oficial.
pub const JAVA_FLAGS: [&str; 10] = [
    "-XX:+UnlockExperimentalVMOptions",
    "-XX:+UseG1GC",
    "-XX:G1NewSizePercent=20",
    "-XX:G1ReservePercent=20",
    "-XX:MaxGCPauseMillis=50",
    "-XX:G1HeapRegionSize=32M",
    "-XX:+DisableExplicitGC",
    "-XX:+ParallelRefProcEnabled",
    "-XX:+UseStringDeduplication",
    "-XX:+OptimizeStringConcat",
];

/// Clase de entrada del bootstrapper de Lunar.
pub const MAIN_CLASS: &str = "com.moonsworth.lunar.genesis.Genesis";

/// Subversión que Lunar asume para los perfiles (`profiles/1.8`).
///
/// El launcher oficial escribe `profiles/1.8` **también** para 1.7.10,
/// tanto en `-Dichor.fabric.localModPath` como en `-Dichor.logsFile`. Esta
/// versión reproduce ese comportamiento a propósito (ver
/// `docs/ARQUITECTURA.md`, «Discrepancias»).
pub const PROFILE_SUBVERSION: &str = "1.8";

/// Busca la especificación de una versión por su nombre largo.
#[must_use]
pub fn version(name: &str) -> Option<&'static VersionSpec> {
    VERSIONS.iter().find(|spec| spec.name == name)
}

/// Nombres largos de las versiones soportadas, en orden.
#[must_use]
pub fn version_names() -> Vec<&'static str> {
    VERSIONS.iter().map(|spec| spec.name).collect()
}

/// Prioridad de una JRE encontrada. Menor es mejor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum JavaRank {
    /// `jre17`, la que usa el launcher oficial para 1.8.9 y 1.7.10.
    Jre17 = 0,
    /// `jre21`.
    Jre21 = 1,
    /// `jre25`.
    Jre25 = 2,
    /// Cualquier otra JRE, o un `java.exe` sin `javaw.exe`.
    Other = 3,
}

/// De dónde salió la JRE elegida.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaSource {
    /// Descargada por el launcher oficial en `~/.lunarclient/jre`.
    LunarJre,
    /// `%JAVA_HOME%\bin`.
    JavaHome,
}

/// JRE seleccionada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaRuntime {
    /// Ruta absoluta del ejecutable (`javaw.exe` preferido).
    pub path: PathBuf,
    /// Prioridad con la que se eligió.
    pub rank: JavaRank,
    /// Origen de la ruta.
    pub source: JavaSource,
}

/// Directorio de natives elegido (`lwjgl64.dll` dentro).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativesDir {
    /// Ruta absoluta del directorio `natives/<hash>`.
    pub path: PathBuf,
    /// Tiene subcarpeta `resources/` (necesaria para los recursos Ultralight).
    pub has_resources: bool,
}

/// Resultado completo de la detección.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    /// JRE elegida, si hay alguna.
    pub java: Option<JavaRuntime>,
    /// Natives elegidos, si hay alguno.
    pub natives: Option<NativesDir>,
    /// Versiones realmente instaladas (mappings + OptiFine presentes).
    pub versions: Vec<&'static VersionSpec>,
}

impl Environment {
    /// Todos los artefactos necesarios están presentes.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.java.is_some() && self.natives.is_some() && !self.versions.is_empty()
    }
}

/// Clasifica una ruta de `javaw.exe` por su prioridad.
///
/// Se mira la ruta completa en minúsculas y se buscan `jre17`, `jre21` y `jre25`
/// en ese orden; todo lo demás cae al grupo 3.
#[must_use]
pub fn rank_for(path: &Path) -> JavaRank {
    let text = path.to_string_lossy().to_lowercase();
    if text.contains("jre17") {
        JavaRank::Jre17
    } else if text.contains("jre21") {
        JavaRank::Jre21
    } else if text.contains("jre25") {
        JavaRank::Jre25
    } else {
        JavaRank::Other
    }
}

/// Busca la mejor JRE: `jre17` > `jre21` > `jre25` > otras, y si no hay
/// `javaw.exe` en la raíz de Lunar, acepta `java.exe`; de última, `%JAVA_HOME%`.
///
/// `java_home` es el valor de `JAVA_HOME` ya resuelto (o `None`).
pub fn find_java(jre_root: &Path, java_home: Option<&Path>) -> Option<JavaRuntime> {
    let mut found = Vec::new();
    collect_named_files(jre_root, "javaw.exe", 0, &mut found);
    if found.is_empty() {
        // Solo se cae a java.exe si no hay ningún javaw.exe debajo de jre/.
        collect_named_files(jre_root, "java.exe", 0, &mut found);
    }
    if !found.is_empty() {
        return pick_java(found, JavaSource::LunarJre);
    }

    let java_home = java_home?;
    ["javaw.exe", "java.exe"].into_iter().find_map(|name| {
        let candidate = java_home.join("bin").join(name);
        candidate.is_file().then_some(JavaRuntime {
            path: candidate,
            rank: JavaRank::Other,
            source: JavaSource::JavaHome,
        })
    })
}

/// Elige la JRE ganadora: menor prioridad y, a igualdad, la primera ruta en
/// orden lexicográfico de la tupla (mayor, menor).
fn pick_java(mut found: Vec<PathBuf>, source: JavaSource) -> Option<JavaRuntime> {
    found.sort_by(|left, right| {
        rank_for(left).cmp(&rank_for(right)).then_with(|| {
            left.to_string_lossy()
                .cmp(&right.to_string_lossy())
                .then(Ordering::Equal)
        })
    });
    let path = found.into_iter().next()?;
    let rank = rank_for(&path);
    Some(JavaRuntime { path, rank, source })
}

/// Recorre `root` buscando ficheros con ese nombre, sin seguir enlaces
/// simbólicos y con profundidad acotada.
fn collect_named_files(root: &Path, wanted: &str, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_JRE_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            collect_named_files(&path, wanted, depth + 1, out);
        } else if file_type.is_file() && name_eq(&path, wanted) {
            out.push(path);
        }
    }
}

/// Compara el nombre del fichero sin distinguir mayúsculas, como hace
/// `pathlib.Path.rglob` en Windows.
fn name_eq(path: &Path, wanted: &str) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(wanted))
}

/// Busca el directorio de natives: el primero (por orden alfabético) con
/// `lwjgl64.dll` **y** `resources/`; si no hay ninguno así, el primero con
/// `lwjgl64.dll` a secas.
pub fn find_natives(natives_root: &Path) -> Option<NativesDir> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(natives_root)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();
    dirs.sort_by_cached_key(|dir| sort_name(dir));

    let has_dll = |dir: &PathBuf| dir.join("lwjgl64.dll").is_file();

    if let Some(path) = dirs
        .iter()
        .find(|dir| has_dll(dir) && dir.join("resources").is_dir())
    {
        return Some(NativesDir {
            path: path.clone(),
            has_resources: true,
        });
    }
    dirs.iter().find(|dir| has_dll(dir)).map(|path| NativesDir {
        path: path.clone(),
        has_resources: false,
    })
}

fn sort_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Directorio de la UI web que contiene `index.html` (nunca su padre).
///
/// Lunar extrae la UI en `ui/<sha1>/` con `index.html`, `static/` y `assets/`.
/// WebOSR resuelve `file:index.html` como `File(uiDir, "index.html")`, así que
/// un `--uiDir` que apunte a `ui/` deja el menú en panorama y sin botones.
pub fn find_ui_dir(ui_root: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(ui_root)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();

    let valid = |dir: &PathBuf| {
        dir.join("index.html").is_file() && dir.join("static").join("index.js").is_file()
    };

    dirs.retain(valid);
    dirs.sort_by_cached_key(|dir| {
        let mtime = dir
            .join("index.html")
            .metadata()
            .and_then(|meta| meta.modified())
            .ok();
        (mtime, sort_name(dir))
    });
    dirs.pop().or_else(|| {
        let root = ui_root.to_path_buf();
        valid(&root).then_some(root)
    })
}

/// Artefactos del classpath para una versión, filtrados por existencia y en el
/// orden del launcher oficial: mappings de la versión primero, `lunar.jar` al
/// final.
#[must_use]
pub fn classpath_jars(multiver: &Path, spec: &VersionSpec) -> Vec<String> {
    let names = std::iter::once(spec.mapping).chain(COMMON_JARS);
    names
        .filter(|name| multiver.join(name).is_file())
        .map(str::to_owned)
        .collect()
}

/// Versiones instaladas de verdad: existen el mapping de la versión y su OptiFine.
#[must_use]
pub fn available_versions(multiver: &Path) -> Vec<&'static VersionSpec> {
    VERSIONS
        .iter()
        .filter(|spec| {
            multiver.join(spec.mapping).is_file() && multiver.join(spec.external).is_file()
        })
        .collect()
}

/// Ejecuta la detección completa sobre las rutas del launcher, leyendo
/// `JAVA_HOME` del entorno.
#[must_use]
pub fn detect(paths: &LunarPaths) -> Environment {
    detect_with(paths, env_java_home().as_deref())
}

/// Detección con el `JAVA_HOME` inyectado, sin leer el proceso.
#[must_use]
pub fn detect_with(paths: &LunarPaths, java_home: Option<&Path>) -> Environment {
    Environment {
        java: find_java(&paths.jre_root(), java_home),
        natives: find_natives(&paths.natives_root()),
        versions: available_versions(paths.multiver()),
    }
}

fn env_java_home() -> Option<PathBuf> {
    std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// Fallos de la detección cuando algo imprescindible falta.
#[derive(Debug, Error)]
pub enum DetectError {
    /// No hay ninguna JRE utilizable.
    #[error("no encontré ninguna JRE de Lunar ni JAVA_HOME con javaw.exe")]
    NoJava,
    /// No hay directorio de natives.
    #[error("no encontré el directorio natives de Lunar (lwjgl64.dll)")]
    NoNatives,
    /// No hay artefactos para la versión pedida.
    #[error("no hay artefactos para {version} en {dir}")]
    NoArtifacts {
        /// Versión pedida.
        version: String,
        /// Directorio de artefactos.
        dir: PathBuf,
    },
    /// Versión no soportada por esta compilación.
    #[error("versión no soportada: {0}")]
    UnsupportedVersion(String),
}
