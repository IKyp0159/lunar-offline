//! Configuración persistente del launcher (`%APPDATA%\LunarOffline\config.json`).
//!
//! Las nueve claves originales viven en la raíz. Lo nuevo se añade al lado, sin
//! ficheros aparte:
//!
//! ```json
//! {
//!   "nick": "", "version": "1.8.9", "ram": 3072, "server": "",
//!   "width": 1280, "height": 720, "fullscreen": false, "java": "",
//!   "advanced": false,
//!
//!   "schema": 2,
//!   "active_profile": "p_...",
//!   "profiles": [ ... ],
//!   "servers": [ ... ],
//!   "jvm": { "preset": "lunar", "xms_mb": null, "xmx_mb": null, "extra_flags": [] },
//!   "ui": { "last_tab": "jugar", "status_bar": true }
//! }
//! ```
//!
//! Reglas del diseño:
//!
//! - **Aditivo**: las nueve claves originales siguen en la raíz con el mismo
//!   nombre y el mismo tipo, y las claves nuevas se conservan al redondear el
//!   fichero.
//! - **Nombres de clave estables**: `ram` en el JSON es [`Config::ram_mb`] en
//!   Rust; el resto se llama igual.
//! - **La raíz manda**: las nueve claves planas son la verdad y el perfil activo
//!   se sincroniza desde ellas al cargar ([`Config::sync_profile_from_root`]).
//!   [`Config::select_profile`] hace lo contrario: copia el perfil a la raíz.
//!   Así la UI puede editar siempre la raíz sin perder nada.
//! - **Nada se pierde**: las claves que esta versión no conoce caen en
//!   [`Config::extra`] (y las de dentro de un perfil o un servidor, en su
//!   propio `extra`) y se vuelven a escribir tal cual.
//! - **Formato estable**: la escritura usa `indent=2` y sin salto final, así
//!   que un guardado sin cambios produce los mismos bytes.
//! - **Migración automática**: un `config.json` sin `schema` se convierte a v2
//!   al cargarlo, con copia previa en `config.json.v1.bak`. Los ficheros de una
//!   versión más nueva **no se degradan**: se leen y se avisa.
//! - **Escritura atómica**: temporal + rename con reintentos ([`lo_paths`]). En
//!   Windows el rename puede fallar un instante si el antivirus o el indexador
//!   tienen el fichero abierto.
//! - **Copia de seguridad**: antes de cada guardado correcto, el fichero
//!   anterior queda en `config.json.bak`.
//! - **Un `config.json` corrupto no impide arrancar**: se aparta a
//!   `config.json.corrupto` y se sigue con los valores por defecto.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use lo_paths::LunarPaths;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};
use thiserror::Error;

/// Versión del esquema que escribe esta compilación.
pub const SCHEMA_VERSION: u32 = 2;
/// Puerto por defecto de Minecraft.
pub const DEFAULT_PORT: u16 = 25565;

/// RAM mínima del slider (MB). Igual que el `Slider(..., 1024, 8192, step=256)`.
pub const RAM_MIN_MB: u32 = 1024;
/// RAM máxima del slider (MB).
pub const RAM_MAX_MB: u32 = 8192;
/// Paso del slider (MB).
pub const RAM_STEP_MB: u32 = 256;
/// RAM por defecto (MB).
pub const RAM_DEFAULT_MB: u32 = 3072;
/// Versión por defecto.
pub const DEFAULT_VERSION: &str = "1.8.9";
/// Ancho por defecto de la ventana del juego.
pub const DEFAULT_WIDTH: u32 = 1280;
/// Alto por defecto de la ventana del juego.
pub const DEFAULT_HEIGHT: u32 = 720;
/// Ancho mínimo aceptado para la ventana del juego.
///
/// Estos límites evitan que el juego arranque con 0×0.
pub const MIN_WIDTH: u32 = 320;
/// Alto mínimo aceptado para la ventana del juego.
pub const MIN_HEIGHT: u32 = 240;
/// Ancho máximo aceptado para la ventana del juego.
pub const MAX_WIDTH: u32 = 7680;
/// Alto máximo aceptado para la ventana del juego.
pub const MAX_HEIGHT: u32 = 4320;

/// Perfiles que caben en el fichero.
pub const MAX_PROFILES: usize = 64;
/// Servidores que caben en el fichero.
pub const MAX_SERVERS: usize = 512;
/// Flags JVM extra por configuración.
pub const MAX_FLAGS: usize = 64;
/// Longitud máxima de un flag JVM.
pub const MAX_FLAG_LEN: usize = 512;
/// Longitud máxima del nombre de un perfil.
pub const MAX_PROFILE_NAME: usize = 32;
/// Longitud máxima del nombre de un servidor.
pub const MAX_SERVER_NAME: usize = 48;
/// Nombre del perfil que se crea al migrar o al no haber ninguno.
pub const DEFAULT_PROFILE_NAME: &str = "Predeterminado";
/// Nombre de relleno de un servidor sin nombre.
pub const DEFAULT_SERVER_NAME: &str = "Servidor";
/// Prefijo de los identificadores de perfil.
pub const PROFILE_PREFIX: &str = "p_";
/// Prefijo de los identificadores de servidor.
pub const SERVER_PREFIX: &str = "s_";
/// Pestaña inicial de la UI.
pub const DEFAULT_TAB: &str = "jugar";

/// Sufijo del fichero apartado cuando el `config.json` no se puede leer.
const CORRUPT_SUFFIX: &str = ".corrupto";
/// Sufijo de la copia de seguridad rotativa.
const BACKUP_SUFFIX: &str = ".bak";
/// Sufijo de la copia de la v1, hecha una sola vez al migrar.
const LEGACY_SUFFIX: &str = ".v1.bak";

/// Claves canónicas de la raíz; el resto se considera «extra» y se preserva.
const KNOWN_KEYS: [&str; 15] = [
    "nick",
    "version",
    "ram",
    "server",
    "width",
    "height",
    "fullscreen",
    "java",
    "advanced",
    "schema",
    "active_profile",
    "profiles",
    "servers",
    "jvm",
    "ui",
];

/// Preset de flags JVM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JvmPreset {
    /// Los flags del launcher oficial (sin `-Xms`).
    #[default]
    Lunar,
    /// Menos memoria y GC serie.
    Low,
    /// G1 con los ajustes habituales de PvP.
    High,
    /// Lo que escriba el usuario.
    Custom,
    /// Solo dentro de un perfil: usar la configuración global.
    Inherit,
}

impl JvmPreset {
    /// Nombre estable para la UI y los logs.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lunar => "lunar",
            Self::Low => "low",
            Self::High => "high",
            Self::Custom => "custom",
            Self::Inherit => "inherit",
        }
    }
}

/// Ajustes de JVM.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JvmSettings {
    /// Preset elegido.
    pub preset: JvmPreset,
    /// `-Xms` en MB; `None` = omitirlo (como hace hoy el launcher).
    pub xms_mb: Option<u32>,
    /// `-Xmx` en MB; `None` = seguir la RAM del perfil.
    pub xmx_mb: Option<u32>,
    /// Flags adicionales, ya validados por `lo-jvm`.
    pub extra_flags: Vec<String>,
    /// Claves desconocidas, preservadas literalmente.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Perfil de juego: un nick, una versión y su entorno.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// Identificador estable (`p_` + 16 hex).
    pub id: String,
    /// Nombre visible.
    pub name: String,
    /// Nick no premium.
    pub nick: String,
    /// Versión instalada.
    pub version: String,
    /// RAM en MB. En el JSON se llama `ram`, igual que en la raíz.
    #[serde(rename = "ram")]
    pub ram_mb: u32,
    /// Servidor al que entrar al arrancar (vacío = menú).
    pub server: String,
    /// Ancho de la ventana del juego.
    pub width: u32,
    /// Alto de la ventana del juego.
    pub height: u32,
    /// Arrancar en pantalla completa.
    pub fullscreen: bool,
    /// Ruta de Java elegida a mano (vacío = detección automática).
    pub java: String,
    /// JVM de este perfil; `None` o `inherit` = la global.
    pub jvm: Option<JvmSettings>,
    /// Marca ISO-8601 de creación; vacía si viene de la v1.
    pub created_at: String,
    /// Marca ISO-8601 del último arranque.
    pub last_used_at: Option<String>,
    /// Claves desconocidas, preservadas literalmente.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: DEFAULT_PROFILE_NAME.to_owned(),
            nick: String::new(),
            version: DEFAULT_VERSION.to_owned(),
            ram_mb: RAM_DEFAULT_MB,
            server: String::new(),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            fullscreen: false,
            java: String::new(),
            jvm: None,
            created_at: String::new(),
            last_used_at: None,
            extra: Map::new(),
        }
    }
}

impl Profile {
    /// Perfil con los valores de la raíz y un identificador vacío.
    fn from_root(config: &Config) -> Self {
        Self {
            id: String::new(),
            name: DEFAULT_PROFILE_NAME.to_owned(),
            nick: config.nick.clone(),
            version: config.version.clone(),
            ram_mb: config.ram_mb,
            server: config.server.clone(),
            width: config.width,
            height: config.height,
            fullscreen: config.fullscreen,
            java: config.java.clone(),
            jvm: None,
            created_at: String::new(),
            last_used_at: None,
            extra: Map::new(),
        }
    }
}

/// Origen de una entrada de la lista de servidores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerSource {
    /// Añadido a mano en el launcher.
    #[default]
    Manual,
    /// Importado de `servers.dat`.
    ServersDat,
}

/// Servidor guardado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Server {
    /// Identificador estable (`s_` + 16 hex).
    pub id: String,
    /// Nombre visible.
    pub name: String,
    /// Host sin puerto.
    pub address: String,
    /// Puerto.
    pub port: u16,
    /// Versión para la que se usa (`""` = indiferente).
    pub version: String,
    /// Marcado como favorito.
    pub favourite: bool,
    /// Marca ISO-8601 de la última conexión.
    pub last_connected: Option<String>,
    /// De dónde salió la entrada.
    pub source: ServerSource,
    /// Nombre original en `servers.dat`, solo si `source = servers_dat`.
    pub dat_key: Option<String>,
    /// Claves desconocidas, preservadas literalmente.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            address: String::new(),
            port: DEFAULT_PORT,
            version: String::new(),
            favourite: false,
            last_connected: None,
            source: ServerSource::Manual,
            dat_key: None,
            extra: Map::new(),
        }
    }
}

/// Preferencias de la interfaz que no afectan al arranque.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    /// Última pestaña abierta.
    pub last_tab: String,
    /// Mostrar la barra de estado.
    pub status_bar: bool,
    /// Claves desconocidas, preservadas literalmente.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            last_tab: DEFAULT_TAB.to_owned(),
            status_bar: true,
            extra: Map::new(),
        }
    }
}

/// Configuración del launcher.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Nick no premium (se valida con el tipo `Nick` de `lo-accounts` al lanzar).
    pub nick: String,
    /// Versión seleccionada (`1.8.9` o `1.7.10`).
    pub version: String,
    /// RAM asignada al juego en MB (`-Xms`/`-Xmx`). En el JSON se llama `ram`.
    #[serde(rename = "ram")]
    pub ram_mb: u32,
    /// Servidor al que conectarse al arrancar (vacío = menú principal).
    pub server: String,
    /// Ancho de la ventana del juego.
    pub width: u32,
    /// Alto de la ventana del juego.
    pub height: u32,
    /// Arrancar en pantalla completa.
    pub fullscreen: bool,
    /// Ruta de Java elegida a mano (vacío = detección automática).
    pub java: String,
    /// El panel *Avanzado* está desplegado.
    pub advanced: bool,

    /// Versión del esquema; `1` o ausente = fichero sin migrar.
    pub schema: u32,
    /// Identificador del perfil en uso.
    pub active_profile: String,
    /// Perfiles guardados. Nunca vacío tras [`Config::normalize`].
    pub profiles: Vec<Profile>,
    /// Servidores guardados.
    pub servers: Vec<Server>,
    /// JVM global, usada por los perfiles que no la sobreescriben.
    pub jvm: JvmSettings,
    /// Preferencias de la interfaz.
    pub ui: UiState,

    /// Claves desconocidas, preservadas literalmente.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Config {
    fn default() -> Self {
        let mut config = Self {
            nick: String::new(),
            version: DEFAULT_VERSION.to_owned(),
            ram_mb: RAM_DEFAULT_MB,
            server: String::new(),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            fullscreen: false,
            java: String::new(),
            advanced: false,
            schema: SCHEMA_VERSION,
            active_profile: String::new(),
            profiles: Vec::new(),
            servers: Vec::new(),
            jvm: JvmSettings::default(),
            ui: UiState::default(),
            extra: Map::new(),
        };
        let mut profile = Profile::from_root(&config);
        profile.id = config.profile_id_from_root();
        config.active_profile.clone_from(&profile.id);
        config.profiles.push(profile);
        config
    }
}

impl Config {
    /// Ruta del `config.json` dentro de los directorios de la aplicación.
    #[must_use]
    pub fn path(paths: &LunarPaths) -> PathBuf {
        paths.config_file().to_path_buf()
    }

    /// Carga la configuración, migrando de la v1 si hace falta.
    ///
    /// Un `config.json` corrupto **no** rompe el arranque: se aparta a
    /// `config.json.corrupto` y se devuelve el resultado en
    /// [`ConfigLoad::Recovered`] para que la UI pueda avisar.
    ///
    /// Un fichero sin `schema` se migra a v2 en el momento: primero se copia a
    /// `config.json.v1.bak` y después se reescribe. El resultado se devuelve
    /// como [`ConfigLoad::Migrated`] para que la UI lo cuente una vez.
    pub fn load(paths: &LunarPaths) -> Result<ConfigLoad, ConfigError> {
        let path = Self::path(paths);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                return Ok(ConfigLoad::Created(Config::default()));
            }
            Err(err) => return Err(ConfigError::Io(err)),
        };

        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => Self::load_object(&path, &map),
            // JSON válido pero que no es un objeto: no hay nada que rescatar.
            Ok(_) => {
                let corrupt_copy = quarantine(&path)?;
                Ok(ConfigLoad::Recovered {
                    config: Config::default(),
                    corrupt_copy,
                })
            }
            Err(err) => {
                debug_assert!(err.is_syntax() || err.is_eof() || err.is_data());
                let corrupt_copy = quarantine(&path)?;
                Ok(ConfigLoad::Recovered {
                    config: Config::default(),
                    corrupt_copy,
                })
            }
        }
    }

    fn load_object(path: &Path, map: &Map<String, Value>) -> Result<ConfigLoad, ConfigError> {
        let from_schema = schema_of(map);
        let mut config = Self::from_object(map);
        if from_schema > SCHEMA_VERSION {
            return Ok(ConfigLoad::Future {
                config,
                schema: from_schema,
            });
        }
        if from_schema < SCHEMA_VERSION {
            config.schema = SCHEMA_VERSION;
            let backup = legacy_backup(path)?;
            write_config_file(path, &config)?;
            return Ok(ConfigLoad::Migrated {
                config,
                from_schema,
                backup,
            });
        }
        Ok(ConfigLoad::Loaded(config))
    }

    /// Guarda la configuración de forma atómica (temporal + rename con reintentos).
    ///
    /// Antes de reemplazar el fichero, el anterior queda en `config.json.bak`.
    /// Valida primero: o se escribe un fichero consistente, o no se escribe nada.
    ///
    /// El perfil activo se sincroniza desde la raíz antes de escribir: al cargar
    /// manda la raíz, así que guardar una raíz distinta de su perfil dejaría un
    /// fichero que cambia solo en la siguiente lectura. Guardar y releer tiene
    /// que dar lo mismo.
    pub fn save(&self, paths: &LunarPaths) -> Result<(), ConfigError> {
        let mut snapshot = self.clone();
        snapshot.sync_profile_from_root();
        snapshot.validate()?;
        let path = Self::path(paths);
        rotate_backup(&path)?;
        write_config_file(&path, &snapshot)
    }

    /// Migra una configuración sin `schema` a la v2.
    ///
    /// Normaliza en lugar de rechazar: un valor del tipo equivocado cae a su
    /// valor por defecto, las cadenas numéricas se aceptan y la RAM se recorta
    /// al rango del slider. Las claves desconocidas se conservan enteras y en
    /// orden.
    #[must_use]
    pub fn migrate_legacy(raw: &Value) -> Config {
        match raw.as_object() {
            Some(map) => {
                let mut config = Self::from_object(map);
                if config.schema < SCHEMA_VERSION {
                    config.schema = SCHEMA_VERSION;
                }
                config
            }
            None => Config::default(),
        }
    }

    fn from_object(map: &Map<String, Value>) -> Config {
        let defaults = Config::default();
        let mut extra = Map::new();
        for (key, value) in map {
            if !KNOWN_KEYS.contains(&key.as_str()) {
                extra.insert(key.clone(), value.clone());
            }
        }
        let mut config = Config {
            nick: text_field(map, "nick", &defaults.nick),
            version: text_field(map, "version", &defaults.version),
            ram_mb: number_field(map, "ram", defaults.ram_mb).clamp(RAM_MIN_MB, RAM_MAX_MB),
            server: text_field(map, "server", &defaults.server),
            width: number_field(map, "width", defaults.width),
            height: number_field(map, "height", defaults.height),
            fullscreen: bool_field(map, "fullscreen", defaults.fullscreen),
            java: text_field(map, "java", &defaults.java),
            advanced: bool_field(map, "advanced", defaults.advanced),
            schema: schema_of(map),
            active_profile: text_field(map, "active_profile", ""),
            profiles: parse_profiles(map),
            servers: parse_servers(map),
            jvm: parse_jvm(map),
            ui: parse_ui(map),
            extra,
        };
        config.normalize();
        config
    }

    /// Deja la configuración en un estado utilizable, sin rechazar nada.
    #[allow(clippy::too_many_lines)]
    fn normalize(&mut self) {
        self.profiles.truncate(MAX_PROFILES);
        if self.profiles.is_empty() {
            let mut profile = Profile::from_root(self);
            profile.id = self.profile_id_from_root();
            self.profiles.push(profile);
        }

        let mut seen: HashSet<String> = HashSet::new();
        for (index, profile) in self.profiles.iter_mut().enumerate() {
            profile.name = clean_name(&profile.name, DEFAULT_PROFILE_NAME, MAX_PROFILE_NAME);
            let mut id = profile.id.trim().to_owned();
            if id.is_empty() || !seen.insert(id.clone()) {
                let mut bump = index + 1;
                id = format!("{PROFILE_PREFIX}{bump:016x}");
                while !seen.insert(id.clone()) {
                    bump += 1;
                    id = format!("{PROFILE_PREFIX}{bump:016x}");
                }
                profile.id = id;
            }
            profile.ram_mb = profile.ram_mb.clamp(RAM_MIN_MB, RAM_MAX_MB);
            let server = profile.server.trim().to_owned();
            profile.server = server;
            if let Some(jvm) = profile.jvm.as_mut() {
                normalize_jvm(jvm);
            }
        }

        if !self
            .profiles
            .iter()
            .any(|profile| profile.id == self.active_profile)
        {
            self.active_profile = self.profiles[0].id.clone();
        }
        self.sync_profile_from_root();

        self.servers.truncate(MAX_SERVERS);
        let mut seen: HashSet<String> = HashSet::new();
        for (index, server) in self.servers.iter_mut().enumerate() {
            if server.port == 0 {
                server.port = DEFAULT_PORT;
            }
            let (address, port) = split_host_port(&server.address, server.port);
            server.address = address;
            server.port = port;
            server.name = clean_name(&server.name, DEFAULT_SERVER_NAME, MAX_SERVER_NAME);
            let mut id = server.id.trim().to_owned();
            if id.is_empty() || !seen.insert(id.clone()) {
                let mut bump = index + 1;
                id = format!("{SERVER_PREFIX}{bump:016x}");
                while !seen.insert(id.clone()) {
                    bump += 1;
                    id = format!("{SERVER_PREFIX}{bump:016x}");
                }
                server.id = id;
            }
        }
        self.servers.retain(|server| !server.address.is_empty());

        normalize_jvm(&mut self.jvm);
    }

    fn profile_id_from_root(&self) -> String {
        let key = format!(
            "{}|{}|{}|{}",
            self.nick, self.version, self.ram_mb, self.server
        );
        stable_id(PROFILE_PREFIX, &key)
    }

    /// RAM dentro del rango del slider.
    #[must_use]
    pub fn ram_in_range(&self) -> u32 {
        self.ram_mb.clamp(RAM_MIN_MB, RAM_MAX_MB)
    }

    /// Comprueba que la resolución es utilizable.
    #[must_use]
    pub fn resolution_ok(&self) -> bool {
        (MIN_WIDTH..=MAX_WIDTH).contains(&self.width)
            && (MIN_HEIGHT..=MAX_HEIGHT).contains(&self.height)
    }

    /// Perfil en uso. Siempre hay uno: [`Config::normalize`] y
    /// [`Config::validate`] lo garantizan.
    #[must_use]
    pub fn active_profile(&self) -> &Profile {
        self.profiles
            .iter()
            .find(|profile| profile.id == self.active_profile)
            .or_else(|| self.profiles.first())
            .expect("Config siempre tiene al menos un perfil; validate() lo exige")
    }

    /// Perfil en uso, mutable.
    pub fn active_profile_mut(&mut self) -> &mut Profile {
        let id = self.active_profile.clone();
        let index = self
            .profiles
            .iter()
            .position(|profile| profile.id == id)
            .unwrap_or(0);
        &mut self.profiles[index]
    }

    /// Activa un perfil: sus campos pasan a la raíz y se marca el uso.
    pub fn select_profile(&mut self, id: &str) -> Result<(), ConfigError> {
        let Some(index) = self.profiles.iter().position(|profile| profile.id == id) else {
            return Err(ConfigError::Validation(format!(
                "no existe el perfil «{id}»"
            )));
        };
        self.active_profile.clone_from(&self.profiles[index].id);
        self.profiles[index].last_used_at = Some(now_iso8601());
        self.sync_root_from_profile();
        Ok(())
    }

    /// Crea un perfil con los valores actuales de la raíz y lo devuelve.
    pub fn add_profile(&mut self, name: &str) -> Result<Profile, ConfigError> {
        if self.profiles.len() >= MAX_PROFILES {
            return Err(ConfigError::TooMany("perfiles"));
        }
        let mut profile = Profile::from_root(self);
        profile.id = self.next_profile_id();
        profile.name = clean_name(name, DEFAULT_PROFILE_NAME, MAX_PROFILE_NAME);
        profile.created_at = now_iso8601();
        self.profiles.push(profile.clone());
        Ok(profile)
    }

    /// Duplica un perfil con un identificador nuevo y « copia» en el nombre.
    pub fn duplicate_profile(&mut self, id: &str) -> Result<Profile, ConfigError> {
        let Some(source) = self
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
        else {
            return Err(ConfigError::Validation(format!(
                "no existe el perfil «{id}»"
            )));
        };
        if self.profiles.len() >= MAX_PROFILES {
            return Err(ConfigError::TooMany("perfiles"));
        }
        let name = format!("{} copia", source.name);
        let mut copy = source;
        copy.id = self.next_profile_id();
        copy.name = clean_name(&name, DEFAULT_PROFILE_NAME, MAX_PROFILE_NAME);
        copy.created_at = now_iso8601();
        copy.last_used_at = None;
        self.profiles.push(copy.clone());
        Ok(copy)
    }

    /// Borra un perfil, aunque sea el último o el principal.
    ///
    /// Si era el último, en su lugar queda uno nuevo y limpio: el launcher
    /// sigue teniendo siempre un perfil válido, pero el usuario puede empezar
    /// de cero sin tocar el fichero a mano. Los servidores, la JVM global y las
    /// preferencias de la interfaz no se tocan.
    pub fn remove_profile(&mut self, id: &str) -> Result<(), ConfigError> {
        let Some(index) = self.profiles.iter().position(|profile| profile.id == id) else {
            return Err(ConfigError::Validation(format!(
                "no existe el perfil «{id}»"
            )));
        };
        let era_activo = self.active_profile == id;
        self.profiles.remove(index);

        if self.profiles.is_empty() {
            let limpio = Config::default();
            self.nick.clone_from(&limpio.nick);
            self.version.clone_from(&limpio.version);
            self.ram_mb = limpio.ram_mb;
            self.server.clone_from(&limpio.server);
            self.width = limpio.width;
            self.height = limpio.height;
            self.fullscreen = limpio.fullscreen;
            self.java.clone_from(&limpio.java);
            self.profiles = limpio.profiles;
            self.active_profile = limpio.active_profile;
            return Ok(());
        }

        if era_activo {
            self.active_profile.clone_from(&self.profiles[0].id);
            self.sync_root_from_profile();
        }
        Ok(())
    }

    /// Añade un servidor o reemplaza el que tenga su mismo `id`.
    ///
    /// La dirección se guarda siempre sin puerto y el puerto en su campo: si el
    /// usuario pega `mc.es:25566` en «Dirección», no se acaba formando
    /// `mc.es:25566:25565`, que el juego rechaza.
    pub fn upsert_server(&mut self, server: Server) -> Result<(), ConfigError> {
        let mut server = server;
        if server.port == 0 {
            server.port = DEFAULT_PORT;
        }
        let (address, port) = split_host_port(&server.address, server.port);
        server.address = address;
        server.port = port;
        if server.address.is_empty() {
            return Err(ConfigError::Validation(
                "el servidor no tiene dirección".to_owned(),
            ));
        }
        server.name = clean_name(&server.name, &server.address, MAX_SERVER_NAME);
        if !server.id.is_empty() {
            if let Some(existing) = self
                .servers
                .iter_mut()
                .find(|existing| existing.id == server.id)
            {
                *existing = server;
                return Ok(());
            }
        }
        if self.servers.len() >= MAX_SERVERS {
            return Err(ConfigError::TooMany("servidores"));
        }
        if server.id.is_empty() {
            server.id = self.next_server_id();
        }
        self.servers.push(server);
        Ok(())
    }

    /// Borra un servidor por identificador.
    pub fn remove_server(&mut self, id: &str) -> Result<(), ConfigError> {
        self.servers.retain(|server| server.id != id);
        Ok(())
    }

    /// JVM efectiva: siempre el preset Lunar del launcher.
    ///
    /// La configuración antigua conserva sus claves `jvm` (no se pierde nada al
    /// guardar), pero el arranque ya no las usa: el launcher emite siempre los
    /// flags de Lunar, que es lo único que se ofrece en la interfaz.
    #[must_use]
    pub fn effective_jvm(&self) -> JvmSettings {
        JvmSettings::default()
    }

    /// Copia el perfil activo a la raíz, que es la fuente de verdad del fichero.
    pub fn sync_root_from_profile(&mut self) {
        let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.id == self.active_profile)
            .cloned()
        else {
            return;
        };
        self.nick = profile.nick;
        self.version = profile.version;
        self.ram_mb = profile.ram_mb;
        self.server = profile.server;
        self.width = profile.width;
        self.height = profile.height;
        self.fullscreen = profile.fullscreen;
        self.java = profile.java;
    }

    /// Copia la raíz al perfil activo. Es la regla al cargar: la raíz manda.
    pub fn sync_profile_from_root(&mut self) {
        let root = RootValues {
            nick: self.nick.clone(),
            version: self.version.clone(),
            ram_mb: self.ram_mb,
            server: self.server.clone(),
            width: self.width,
            height: self.height,
            fullscreen: self.fullscreen,
            java: self.java.clone(),
        };
        let Some(profile) = self
            .profiles
            .iter_mut()
            .find(|profile| profile.id == self.active_profile)
        else {
            return;
        };
        profile.nick = root.nick;
        profile.version = root.version;
        profile.ram_mb = root.ram_mb;
        profile.server = root.server;
        profile.width = root.width;
        profile.height = root.height;
        profile.fullscreen = root.fullscreen;
        profile.java = root.java;
    }

    /// Comprueba que el fichero que se va a escribir es consistente.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.profiles.is_empty() {
            return Err(ConfigError::Validation(
                "no hay ningún perfil guardado".to_owned(),
            ));
        }
        if self.profiles.len() > MAX_PROFILES {
            return Err(ConfigError::TooMany("perfiles"));
        }
        if self.servers.len() > MAX_SERVERS {
            return Err(ConfigError::TooMany("servidores"));
        }
        let mut ids: HashSet<&str> = HashSet::new();
        for profile in &self.profiles {
            let name = profile.name.trim();
            if name.is_empty() {
                return Err(ConfigError::Validation(format!(
                    "el perfil «{}» no tiene nombre",
                    profile.id
                )));
            }
            if name.chars().count() > MAX_PROFILE_NAME {
                return Err(ConfigError::Validation(format!(
                    "el nombre del perfil «{name}» pasa de {MAX_PROFILE_NAME} caracteres"
                )));
            }
            if profile.id.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "hay un perfil sin identificador".to_owned(),
                ));
            }
            if !ids.insert(profile.id.as_str()) {
                return Err(ConfigError::Validation(format!(
                    "el identificador de perfil «{}» está repetido",
                    profile.id
                )));
            }
            if let Some(jvm) = profile.jvm.as_ref() {
                validate_jvm(jvm)?;
            }
        }
        if !ids.contains(self.active_profile.as_str()) {
            return Err(ConfigError::Validation(format!(
                "el perfil activo «{}» no existe",
                self.active_profile
            )));
        }
        for server in &self.servers {
            if server.name.chars().count() > MAX_SERVER_NAME {
                return Err(ConfigError::Validation(format!(
                    "el nombre del servidor «{}» pasa de {MAX_SERVER_NAME} caracteres",
                    server.name
                )));
            }
        }
        validate_jvm(&self.jvm)
    }

    fn next_profile_id(&self) -> String {
        for index in 1..=MAX_PROFILES {
            let candidate = format!("{PROFILE_PREFIX}{index:016x}");
            if !self.profiles.iter().any(|profile| profile.id == candidate) {
                return candidate;
            }
        }
        format!("{PROFILE_PREFIX}{:016x}", MAX_PROFILES + 1)
    }

    fn next_server_id(&self) -> String {
        for index in 1..=MAX_SERVERS {
            let candidate = format!("{SERVER_PREFIX}{index:016x}");
            if !self.servers.iter().any(|server| server.id == candidate) {
                return candidate;
            }
        }
        format!("{SERVER_PREFIX}{:016x}", MAX_SERVERS + 1)
    }
}

/// Valores de la raíz que se copian al perfil activo.
struct RootValues {
    nick: String,
    version: String,
    ram_mb: u32,
    server: String,
    width: u32,
    height: u32,
    fullscreen: bool,
    java: String,
}

/// Separa `host:puerto` si la dirección ya trae puerto.
///
/// Respeta IPv6: `[::1]:25566` → `("::1", 25566)` y `[::1]` → `("::1", port)`.
/// Una dirección con varios `:` sin corchetes se deja intacta (es IPv6 sin
/// puerto). Si la parte final no es un puerto válido, no se toca nada.
fn split_host_port(address: &str, port: u16) -> (String, u16) {
    let address = address.trim();
    if let Some(resto) = address.strip_prefix('[') {
        let Some((host, cola)) = resto.split_once(']') else {
            return (address.to_owned(), port);
        };
        if let Some(puerto) = cola.strip_prefix(':') {
            if let Ok(numero) = puerto.parse::<u16>() {
                if numero > 0 {
                    return (host.to_owned(), numero);
                }
            }
        }
        return (host.to_owned(), port);
    }
    if address.matches(':').count() == 1 {
        if let Some((host, puerto)) = address.split_once(':') {
            if let Ok(numero) = puerto.parse::<u16>() {
                if numero > 0 {
                    return (host.to_owned(), numero);
                }
            }
        }
    }
    (address.to_owned(), port)
}

fn normalize_jvm(jvm: &mut JvmSettings) {
    if jvm.extra_flags.len() > MAX_FLAGS {
        jvm.extra_flags.truncate(MAX_FLAGS);
    }
    for flag in &mut jvm.extra_flags {
        let trimmed = flag.trim().to_owned();
        *flag = trimmed;
    }
    jvm.extra_flags
        .retain(|flag| !flag.is_empty() && flag.chars().count() <= MAX_FLAG_LEN);
}

fn validate_jvm(jvm: &JvmSettings) -> Result<(), ConfigError> {
    if jvm.extra_flags.len() > MAX_FLAGS {
        return Err(ConfigError::TooMany("flags JVM"));
    }
    for flag in &jvm.extra_flags {
        if flag.trim().is_empty() {
            return Err(ConfigError::Validation("hay un flag JVM vacío".to_owned()));
        }
        if flag.chars().count() > MAX_FLAG_LEN {
            return Err(ConfigError::Validation(format!(
                "el flag «{}…» pasa de {MAX_FLAG_LEN} caracteres",
                flag.chars().take(24).collect::<String>()
            )));
        }
    }
    Ok(())
}

fn schema_of(map: &Map<String, Value>) -> u32 {
    map.get("schema").and_then(parse_u32).unwrap_or(1)
}

fn parse_profiles(map: &Map<String, Value>) -> Vec<Profile> {
    map.get("profiles")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<Profile>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn parse_servers(map: &Map<String, Value>) -> Vec<Server> {
    map.get("servers")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<Server>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn parse_jvm(map: &Map<String, Value>) -> JvmSettings {
    map.get("jvm")
        .and_then(|value| serde_json::from_value::<JvmSettings>(value.clone()).ok())
        .unwrap_or_default()
}

fn parse_ui(map: &Map<String, Value>) -> UiState {
    map.get("ui")
        .and_then(|value| serde_json::from_value::<UiState>(value.clone()).ok())
        .unwrap_or_default()
}

fn text_field(map: &Map<String, Value>, key: &str, fallback: &str) -> String {
    match map.get(key) {
        Some(Value::String(text)) => text.clone(),
        // Un número donde se espera texto se acepta y se convierte.
        Some(Value::Number(number)) => number.to_string(),
        _ => fallback.to_owned(),
    }
}

fn number_field(map: &Map<String, Value>, key: &str, fallback: u32) -> u32 {
    map.get(key).and_then(parse_u32).unwrap_or(fallback)
}

/// Convierte un valor JSON en entero sin signo, aceptando números, enteros en
/// coma flotante y cadenas numéricas.
fn parse_u32(value: &Value) -> Option<u32> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .or_else(|| number.as_f64().and_then(float_to_u32)),
        Value::String(text) => text
            .trim()
            .parse::<u32>()
            .ok()
            .or_else(|| text.trim().parse::<f64>().ok().and_then(float_to_u32)),
        _ => None,
    }
}

/// Redondea un `f64` a `u32` si cae dentro del rango representable.
///
/// El cast es saturante desde Rust 1.45, así que el filtro previo solo evita
/// que un valor fuera de rango se convierta en un número silenciosamente
/// distinto.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn float_to_u32(value: f64) -> Option<u32> {
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        return None;
    }
    Some(value.round() as u32)
}

fn bool_field(map: &Map<String, Value>, key: &str, fallback: bool) -> bool {
    match map.get(key) {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_i64().is_some_and(|n| n != 0),
        Some(Value::String(text)) => match text.trim().to_lowercase().as_str() {
            "1" | "true" | "yes" | "si" | "sí" => true,
            "0" | "false" | "no" => false,
            _ => fallback,
        },
        _ => fallback,
    }
}

/// Nombre limpio: sin espacios en los extremos, nunca vacío y nunca más largo
/// que `max` caracteres (contando caracteres, no bytes).
fn clean_name(raw: &str, fallback: &str, max: usize) -> String {
    let trimmed = raw.trim();
    let mut name: String = if trimmed.is_empty() {
        fallback.trim().to_owned()
    } else {
        trimmed.to_owned()
    };
    if name.chars().count() > max {
        name = name.chars().take(max).collect();
    }
    name
}

/// Identificador corto y estable: prefijo + 16 hex del sha1 de `key`.
fn stable_id(prefix: &str, key: &str) -> String {
    let digest = Sha1::digest(key.as_bytes());
    let mut head = [0u8; 8];
    head.copy_from_slice(&digest[..8]);
    format!("{prefix}{:016x}", u64::from_be_bytes(head))
}

/// Identificador de servidor derivado de una clave estable (por ejemplo el
/// nombre que tiene la entrada en `servers.dat`).
///
/// `lo-servers` lo usa al importar: dos lecturas del mismo `servers.dat` dan el
/// mismo `id`, así que la fusión no duplica entradas ni depende del reloj.
#[must_use]
pub fn server_id_for(key: &str) -> String {
    stable_id(SERVER_PREFIX, key)
}

/// Marca ISO-8601 en UTC con segundos, sin fracciones.
fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    iso8601_from_unix(i64::try_from(secs).unwrap_or(i64::MAX))
}

/// Convierte segundos Unix en `YYYY-MM-DDTHH:MM:SSZ`.
fn iso8601_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = rest / 3_600;
    let minute = (rest % 3_600) / 60;
    let second = rest % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Días desde 1970-01-01 a fecha civil (algoritmo de Howard Hinnant).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (year, month as u32, day as u32)
}

/// Aparta un `config.json` ilegible a `config.json.corrupto`.
fn quarantine(path: &Path) -> Result<PathBuf, ConfigError> {
    let backup = path.with_file_name(format!(
        "{}{CORRUPT_SUFFIX}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    if lo_paths::rename_with_retry(path, &backup).is_ok() {
        return Ok(backup);
    }
    // El rename puede fallar si el destino está en otro volumen; copiar sirve.
    fs::copy(path, &backup)?;
    Ok(backup)
}

/// Copia la v1 a `config.json.v1.bak` una sola vez.
///
/// Si la copia ya existe se conserva: es la original, y una segunda migración
/// tras un cierre a medias no debe pisarla con un fichero ya convertido.
fn legacy_backup(path: &Path) -> Result<PathBuf, ConfigError> {
    let backup = path.with_file_name(format!(
        "{}{LEGACY_SUFFIX}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    if backup.exists() {
        return Ok(backup);
    }
    fs::copy(path, &backup)?;
    Ok(backup)
}

/// Deja el fichero anterior en `config.json.bak` antes de reemplazarlo.
fn rotate_backup(path: &Path) -> Result<(), ConfigError> {
    if !path.is_file() {
        return Ok(());
    }
    let backup = path.with_file_name(format!(
        "{}{BACKUP_SUFFIX}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    fs::copy(path, &backup)?;
    Ok(())
}

fn write_config_file(path: &Path, config: &Config) -> Result<(), ConfigError> {
    // `to_string_pretty` = indent 2 sin salto final, igual que json.dumps(indent=2).
    let body = serde_json::to_string_pretty(config)?;
    lo_paths::write_atomic(path, body.as_bytes())?;
    Ok(())
}

/// Resultado de cargar la configuración.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigLoad {
    /// El fichero existía, era v2 y se leyó.
    Loaded(Config),
    /// No existía: se han usado los valores por defecto.
    Created(Config),
    /// Venía de una versión anterior y se migró a v2 en el disco.
    Migrated {
        /// Configuración efectiva.
        config: Config,
        /// Esquema del que venía.
        from_schema: u32,
        /// Copia del fichero original.
        backup: PathBuf,
    },
    /// El fichero es de una versión más nueva: se lee lo que se entiende.
    Future {
        /// Configuración efectiva.
        config: Config,
        /// Esquema que declara el fichero.
        schema: u32,
    },
    /// Estaba corrupto: se apartó una copia y se usan los valores por defecto.
    Recovered {
        /// Configuración efectiva.
        config: Config,
        /// Dónde quedó el fichero ilegible.
        corrupt_copy: PathBuf,
    },
}

impl ConfigLoad {
    /// Configuración efectiva, sea cual sea el camino.
    #[must_use]
    pub fn config(&self) -> &Config {
        match self {
            Self::Loaded(config)
            | Self::Created(config)
            | Self::Migrated { config, .. }
            | Self::Future { config, .. }
            | Self::Recovered { config, .. } => config,
        }
    }

    /// Aviso para la UI, si hay algo que contar.
    #[must_use]
    pub fn notice(&self) -> Option<String> {
        match self {
            Self::Loaded(_) | Self::Created(_) => None,
            Self::Migrated {
                from_schema,
                backup,
                ..
            } => Some(format!(
                "configuración migrada de la versión {from_schema} a la {SCHEMA_VERSION}; copia del fichero original en {}",
                backup.display()
            )),
            Self::Future { schema, .. } => Some(format!(
                "config.json es de una versión más nueva (schema {schema}); se conserva lo que esta compilación entiende"
            )),
            Self::Recovered { corrupt_copy, .. } => Some(format!(
                "config.json no se pudo leer; se guardó una copia en {} y se usan los valores por defecto",
                corrupt_copy.display()
            )),
        }
    }
}

/// Fallos al leer o escribir `config.json`.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Fallo de E/S.
    #[error("no pude acceder a config.json: {0}")]
    Io(#[from] std::io::Error),
    /// JSON inválido.
    #[error("config.json no es JSON válido: {0}")]
    Parse(#[from] serde_json::Error),
    /// Se ha superado un límite del fichero.
    #[error("demasiados {0} en config.json")]
    TooMany(&'static str),
    /// El contenido no es utilizable tal cual.
    #[error("configuración no válida: {0}")]
    Validation(String),
}
