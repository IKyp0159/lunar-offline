//! `servers.dat` de Minecraft: lectura, escritura atómica y fusión con
//! la lista de servidores de `config.json`.
//!
//! El fichero del juego es NBT comprimido con gzip:
//!
//! ```text
//! TAG_Compound("") {
//!   TAG_List("servers") [
//!     TAG_Compound {
//!       TAG_String("name")       nombre visible
//!       TAG_String("ip")         host o host:puerto
//!       TAG_Byte("hidden")       0/1 (opcional)
//!       TAG_String("icon")       data URI del icono (opcional, puede ser grande)
//!       TAG_Byte("acceptTextures")
//!     }
//!   ]
//! }
//! ```
//!
//! Reglas de esta crate:
//!
//! - **Nunca se borra una entrada.** Ni del `servers.dat` ni de `config.json`.
//!   Los dos sentidos de la fusión solo añaden o actualizan.
//! - **Un fichero que no se entiende no se toca.** Si el gzip o el NBT fallan,
//!   o la estructura no es la esperada, [`ServersDat::read`] devuelve una lista
//!   vacía con `readable = false`, y [`ServersDat::write_atomic`] se niega a
//!   escribir. Es un fichero del juego con datos del jugador: mejor no tocarlo
//!   que reescribirlo a medias.
//! - **Etiquetas desconocidas se conservan.** Cada entrada guarda sus claves
//!   NBT no interpretadas en [`DatServer::extra`] (incluido el icono, que puede
//!   pesar más que el resto del fichero) y se vuelven a escribir tal cual.
//! - **Antes de escribir, copia `servers.dat.bak`.** Una sola copia, no
//!   rotativa: es la red de seguridad para volver atrás a mano.
//! - **Escritura atómica** con los mismos reintentos que [`lo_paths`]: en
//!   Windows el rename falla un instante si el juego o el antivirus tienen el
//!   fichero abierto.
//!
//! ## Sincronización con el juego
//!
//! `ServerList.save()` reescribe el fichero entero al añadir, editar o borrar
//! una entrada y en `Minecraft.shutdownMinecraftApplet`. Si el jugador toca la
//! lista dentro del juego mientras el launcher escribe, gana el último que
//! escriba. Por eso la UI sincroniza antes de lanzar y no durante la partida, y
//! la fusión vuelve a leer el fichero justo antes de escribir.

use std::collections::HashMap;
use std::fs;
use std::io::{ErrorKind, Read as _, Write as _};
use std::path::{Path, PathBuf};

use fastnbt::Value;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use lo_config::{DEFAULT_PORT, MAX_SERVER_NAME, MAX_SERVERS, Server, ServerSource};
use lo_paths::LunarPaths;
use serde_json::Value as Json;
use thiserror::Error;

/// Nombre del fichero del juego dentro de `.minecraft`.
pub const SERVERS_DAT_NAME: &str = "servers.dat";
/// Copia de seguridad que se deja antes de reescribir.
pub const SERVERS_DAT_BACKUP_NAME: &str = "servers.dat.bak";
/// Clave del NBT que contiene la lista.
const ROOT_KEY: &str = "servers";
/// Tope de entradas que se aceptan de un fichero ajeno.
pub const MAX_DAT_ENTRIES: usize = 4096;
/// Tope de bytes que se leen de un `servers.dat` (16 MiB: los iconos pesan).
pub const MAX_DAT_BYTES: u64 = 16 * 1024 * 1024;

/// Ruta del `servers.dat` que lee el juego.
#[must_use]
pub fn dat_path(paths: &LunarPaths) -> PathBuf {
    paths.minecraft_dir().join(SERVERS_DAT_NAME)
}

/// Una entrada de `servers.dat`, con lo que no se interpreta guardado tal cual.
#[derive(Debug, Clone, PartialEq)]
pub struct DatServer {
    /// Nombre visible. Es la clave con la que se empareja la entrada.
    pub name: String,
    /// `ip` tal cual está en el fichero (puede llevar `:puerto`).
    pub address: String,
    /// `hidden = 1`.
    pub hidden: bool,
    /// La entrada trae icono (se conserva en [`DatServer::extra`]).
    pub icon_present: bool,
    /// Claves NBT no interpretadas, conservadas literalmente.
    pub extra: HashMap<String, Value>,
}

impl DatServer {
    /// Entrada nueva con lo mínimo que exige el juego.
    #[must_use]
    pub fn new(name: &str, address: &str) -> Self {
        Self {
            name: name.trim().to_owned(),
            address: address.trim().to_owned(),
            hidden: false,
            icon_present: false,
            extra: HashMap::new(),
        }
    }

    /// Host y puerto separados, con las mismas reglas que el juego: `host`,
    /// `host:puerto`, `[v6]:puerto` y `v6:sin:puerto` (se queda el host entero).
    #[must_use]
    pub fn host_port(&self) -> (String, u16) {
        split_host_port(&self.address)
    }

    /// Reconstruye la entrada como compuesto NBT.
    ///
    /// `name` e `ip` se escriben primero; después las claves conservadas y, si
    /// toca, `hidden`. El orden dentro de un compuesto NBT no significa nada
    /// para el juego (el mapa interno es un `HashMap`), así que no se garantiza
    /// que los bytes de dos escrituras coincidan.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut map: HashMap<String, Value> = HashMap::new();
        map.insert("name".to_owned(), Value::String(self.name.clone()));
        map.insert("ip".to_owned(), Value::String(self.address.clone()));
        for (key, value) in &self.extra {
            if key == "name" || key == "ip" || key == "hidden" {
                continue;
            }
            map.insert(key.clone(), value.clone());
        }
        if self.hidden {
            map.insert("hidden".to_owned(), Value::Byte(1));
        }
        Value::Compound(map)
    }

    /// Lee una entrada. Devuelve `None` si no es un compuesto o no tiene un
    /// `ip` de texto: eso invalida el fichero entero, no la entrada.
    #[must_use]
    pub fn from_value(value: &Value) -> Option<Self> {
        let Value::Compound(map) = value else {
            return None;
        };
        let name = map.get("name").and_then(as_string).unwrap_or_default();
        let address = map.get("ip").and_then(as_string)?;
        let hidden = match map.get("hidden") {
            Some(Value::Byte(flag)) => *flag != 0,
            Some(Value::Short(flag)) => *flag != 0,
            Some(Value::Int(flag)) => *flag != 0,
            Some(Value::String(text)) => text.trim() == "1" || text.eq_ignore_ascii_case("true"),
            _ => false,
        };
        let icon_present = map.contains_key("icon");
        let mut extra = HashMap::new();
        for (key, value) in map {
            if key == "name" || key == "ip" || key == "hidden" {
                continue;
            }
            extra.insert(key.clone(), value.clone());
        }
        Some(Self {
            name: name.trim().to_owned(),
            address: address.trim().to_owned(),
            hidden,
            icon_present,
            extra,
        })
    }
}

/// `servers.dat` leído.
#[derive(Debug, Clone)]
pub struct ServersDat {
    /// Entradas, en el orden del fichero.
    pub entries: Vec<DatServer>,
    /// De dónde se leyó.
    pub path: PathBuf,
    /// Copia de seguridad, si se hizo una.
    pub backup: Option<PathBuf>,
    readable: bool,
    existed: bool,
    notice: Option<String>,
}

impl ServersDat {
    /// Lee el fichero. **No falla por contenido raro**: un fichero ilegible se
    /// devuelve vacío con [`ServersDat::is_readable`] en `false` y un aviso,
    /// para que la capa de arriba lo cuente y no se escriba encima.
    pub fn read(path: &Path) -> Result<Self, ServersError> {
        let mut dat = Self {
            entries: Vec::new(),
            path: path.to_path_buf(),
            backup: None,
            readable: true,
            existed: false,
            notice: None,
        };
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(dat),
            Err(err) => return Err(ServersError::Io(err)),
        };
        dat.existed = true;
        if bytes.len() as u64 > MAX_DAT_BYTES {
            dat.mark_unreadable(format!(
                "{} pasa de {MAX_DAT_BYTES} bytes; no lo toco",
                path.display()
            ));
            return Ok(dat);
        }

        let root = match decode_nbt(&bytes) {
            Ok(root) => root,
            Err(reason) => {
                dat.mark_unreadable(format!("{} no es NBT válido: {reason}", path.display()));
                return Ok(dat);
            }
        };
        let Value::Compound(root_map) = &root else {
            dat.mark_unreadable(format!(
                "{} no tiene un compuesto en la raíz",
                path.display()
            ));
            return Ok(dat);
        };
        let Some(Value::List(items)) = root_map.get(ROOT_KEY) else {
            dat.mark_unreadable(format!("{} no tiene la lista `{ROOT_KEY}`", path.display()));
            return Ok(dat);
        };
        if items.len() > MAX_DAT_ENTRIES {
            dat.mark_unreadable(format!(
                "{} trae más de {MAX_DAT_ENTRIES} servidores; no lo toco",
                path.display()
            ));
            return Ok(dat);
        }

        let mut entries = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let Some(entry) = DatServer::from_value(item) else {
                dat.mark_unreadable(format!(
                    "{}: la entrada {index} no es un compuesto con `ip`",
                    path.display()
                ));
                return Ok(dat);
            };
            if entry.address.is_empty() {
                dat.mark_unreadable(format!(
                    "{}: la entrada {index} tiene el `ip` vacío",
                    path.display()
                ));
                return Ok(dat);
            }
            entries.push(entry);
        }
        dat.entries = entries;
        Ok(dat)
    }

    /// Escribe el fichero: copia previa en `servers.dat.bak` y después
    /// [`lo_paths::write_atomic`].
    ///
    /// Se niega si el fichero de origen no se entendió ([`ServersError::NotReadable`]);
    /// un `servers.dat` corrupto no se destruye.
    pub fn write_atomic(&self, path: &Path) -> Result<(), ServersError> {
        if !self.readable {
            return Err(ServersError::NotReadable(path.to_path_buf()));
        }
        if self.entries.len() > MAX_DAT_ENTRIES {
            return Err(ServersError::TooMany(self.entries.len()));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if path.is_file() {
            let backup = path.with_file_name(SERVERS_DAT_BACKUP_NAME);
            fs::copy(path, &backup)?;
        }
        lo_paths::write_atomic(path, &encode_nbt(&self.entries)?)?;
        Ok(())
    }

    /// Escribe en [`ServersDat::path`].
    pub fn save(&self) -> Result<(), ServersError> {
        self.write_atomic(&self.path)
    }

    /// `false` si el fichero existía y no se pudo interpretar. Nunca se escribe
    /// encima de un fichero así.
    #[must_use]
    pub fn is_readable(&self) -> bool {
        self.readable
    }

    /// `true` si el fichero existía al leerlo.
    #[must_use]
    pub fn existed(&self) -> bool {
        self.existed
    }

    /// Aviso para la UI, si el fichero dio problemas.
    #[must_use]
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Entrada con ese nombre exacto (la clave de `servers.dat`).
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<&DatServer> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    /// Lleva a `servers.dat` lo que está en `config.json` y no en el fichero.
    ///
    /// - `source = manual`: se añade si no hay ya una entrada con el mismo
    ///   host y puerto.
    /// - `source = servers_dat`: se empareja por `dat_key`; si el usuario cambió
    ///   el host en `config.json`, gana `config.json` y se reescribe la entrada.
    /// - Nunca borra.
    pub fn merge_from_config(&mut self, servers: &[Server]) -> Result<MergeReport, ServersError> {
        if !self.readable {
            return Err(ServersError::NotReadable(self.path.clone()));
        }
        let mut added = 0;
        let mut updated = 0;
        let mut kept = 0;
        for server in servers {
            if server.address.trim().is_empty() {
                continue;
            }
            let (host, port) = normalize_address(&server.address, server.port);
            let dat_key = server.dat_key.as_deref().filter(|key| !key.is_empty());
            // El nombre solo no basta: `servers.dat` admite nombres repetidos.
            // Se empareja primero por nombre y dirección, después por dirección
            // y solo al final por nombre (una entrada que se mudó).
            let index = dat_key
                .and_then(|key| {
                    self.entries
                        .iter()
                        .position(|entry| entry.name == key && entry_matches(entry, &host, port))
                })
                .or_else(|| {
                    self.entries
                        .iter()
                        .position(|entry| entry_matches(entry, &host, port))
                })
                .or_else(|| {
                    dat_key.and_then(|key| {
                        // Solo si ese nombre es único en el fichero: con dos
                        // «Mi servidor» no se sabe cuál se mudó, y mover la
                        // equivocada pisaría la dirección buena.
                        if self.name_count(key) == 1 {
                            self.entries.iter().position(|entry| entry.name == key)
                        } else {
                            None
                        }
                    })
                });
            if let Some(index) = index {
                let wanted = format_address(&host, port);
                // El nombre solo se reescribe si la entrada es de este servidor
                // (mismo `dat_key`). Cuando solo coinciden host y puerto puede
                // ser otro servidor del usuario con la misma dirección: cambiar
                // el nombre haría que dos servidores se turnasen el nombre en
                // cada fusión y la segunda pasada nunca quedaría igual.
                let known = dat_key.is_some_and(|key| self.entries[index].name == key);
                let name = if known {
                    unique_name(&self.entries, &clean_name(&server.name, &host), Some(index))
                } else {
                    self.entries[index].name.clone()
                };
                let entry = &mut self.entries[index];
                if entry.address != wanted || entry.name != name {
                    entry.address = wanted;
                    entry.name = name;
                    updated += 1;
                } else {
                    kept += 1;
                }
            } else {
                if self.entries.len() >= MAX_DAT_ENTRIES {
                    return Err(ServersError::TooMany(self.entries.len() + 1));
                }
                let name = match server.dat_key.as_deref().filter(|key| !key.is_empty()) {
                    Some(key) => key.to_owned(),
                    None => unique_name(&self.entries, &clean_name(&server.name, &host), None),
                };
                let entry = DatServer::new(&name, &format_address(&host, port));
                self.entries.push(entry);
                added += 1;
            }
        }
        Ok(MergeReport::from_counts(added, updated, kept))
    }

    /// Cuántas entradas del fichero llevan ese nombre.
    ///
    /// `servers.dat` admite nombres repetidos; la fusión lo necesita para no
    /// confundir dos servidores que el usuario llamó igual.
    #[must_use]
    pub fn name_count(&self, name: &str) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.name == name)
            .count()
    }

    /// Lleva a `config.json` lo que está en `servers.dat` y no en la lista.
    ///
    /// Nunca borra. Lo que ya existe se empareja por `dat_key` y, si no hay,
    /// por host y puerto; de una entrada existente solo se corrige la dirección
    /// (el nombre, el favorito y la última conexión son del usuario).
    #[must_use]
    pub fn merge_into_config(&self, servers: &mut Vec<Server>) -> MergeReport {
        let mut added = 0;
        let mut updated = 0;
        let mut kept = 0;
        for entry in &self.entries {
            let (host, port) = entry.host_port();
            if host.is_empty() {
                continue;
            }
            let index = servers
                .iter()
                .position(|server| {
                    server.dat_key.as_deref() == Some(entry.name.as_str())
                        && server.address.eq_ignore_ascii_case(&host)
                        && server.port == port
                })
                .or_else(|| {
                    servers.iter().position(|server| {
                        server.address.eq_ignore_ascii_case(&host) && server.port == port
                    })
                })
                .or_else(|| {
                    let unique = self.name_count(&entry.name) == 1;
                    if unique {
                        servers.iter().position(|server| {
                            server.dat_key.as_deref() == Some(entry.name.as_str())
                        })
                    } else {
                        None
                    }
                });
            if let Some(index) = index {
                let server = &mut servers[index];
                if server.dat_key.is_none() {
                    server.dat_key = Some(entry.name.clone());
                }
                if server.address.eq_ignore_ascii_case(&host) && server.port == port {
                    kept += 1;
                } else {
                    server.address = host;
                    server.port = port;
                    updated += 1;
                }
            } else {
                if servers.len() >= MAX_SERVERS {
                    break;
                }
                servers.push(Server {
                    id: lo_config::server_id_for(&entry.name),
                    name: clean_name(&entry.name, &host),
                    address: host,
                    port,
                    version: String::new(),
                    favourite: false,
                    last_connected: None,
                    source: ServerSource::ServersDat,
                    dat_key: Some(entry.name.clone()),
                    extra: serde_json::Map::new(),
                });
                added += 1;
            }
        }
        MergeReport::from_counts(added, updated, kept)
    }

    fn mark_unreadable(&mut self, notice: String) {
        self.readable = false;
        self.existed = true;
        self.entries.clear();
        self.notice = Some(notice);
    }
}

/// Resultado de una fusión.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeReport {
    /// Nada cambió.
    Unchanged,
    /// Cambió algo.
    Merged {
        /// Entradas nuevas.
        added: usize,
        /// Entradas existentes corregidas.
        updated: usize,
        /// Entradas que ya estaban al día.
        kept: usize,
    },
}

impl MergeReport {
    fn from_counts(added: usize, updated: usize, kept: usize) -> Self {
        if added == 0 && updated == 0 {
            Self::Unchanged
        } else {
            Self::Merged {
                added,
                updated,
                kept,
            }
        }
    }

    /// Texto corto para la barra de estado.
    #[must_use]
    pub fn summary(self) -> String {
        match self {
            Self::Unchanged => "sin cambios".to_owned(),
            Self::Merged {
                added,
                updated,
                kept,
            } => format!("{added} nuevos, {updated} actualizados, {kept} iguales"),
        }
    }
}

/// Fallos de `servers.dat` y de los ficheros de importación.
#[derive(Debug, Error)]
pub enum ServersError {
    /// Fallo de E/S.
    #[error("no pude acceder a servers.dat: {0}")]
    Io(#[from] std::io::Error),
    /// El JSON de importación o exportación no se pudo leer o escribir.
    #[error("JSON de servidores no válido: {0}")]
    Json(#[from] serde_json::Error),
    /// El NBT no se pudo interpretar.
    #[error("servers.dat no es NBT válido: {0}")]
    Nbt(String),
    /// El fichero existe pero no se entiende: no se escribe encima.
    #[error("{} no se pudo interpretar; no se escribe encima", .0.display())]
    NotReadable(PathBuf),
    /// Se pasa del tope de entradas.
    #[error("demasiados servidores en servers.dat ({0})")]
    TooMany(usize),
    /// Un fichero importado no tiene la forma esperada.
    #[error("el fichero de servidores no tiene la forma esperada: {0}")]
    Invalid(String),
}

/// Importa una lista de servidores de un JSON.
///
/// Acepta un array `[{"name": "...", "address": "..."}]` o un objeto
/// `{"servers": [...]}`, que es lo que exporta [`export_json`]. Los campos
/// `address`, `host` e `ip` valen igual; si falta el nombre se usa la
/// dirección. Los que no tienen dirección se descartan y los repetidos (mismo
/// host y puerto) se quedan una sola vez.
pub fn import_json(path: &Path) -> Result<Vec<Server>, ServersError> {
    let text = fs::read_to_string(path)?;
    let root: Json =
        serde_json::from_str(&text).map_err(|err| ServersError::Invalid(err.to_string()))?;
    let items = match &root {
        Json::Array(items) => items.clone(),
        Json::Object(map) => map
            .get("servers")
            .and_then(Json::as_array)
            .cloned()
            .ok_or_else(|| ServersError::Invalid("falta la clave `servers`".to_owned()))?,
        _ => {
            return Err(ServersError::Invalid(
                "se esperaba un array o un objeto con `servers`".to_owned(),
            ));
        }
    };

    let mut servers: Vec<Server> = Vec::new();
    for item in items {
        let Some(mut server) = server_from_json(&item) else {
            continue;
        };
        let (host, port) = normalize_address(&server.address, server.port);
        server.address.clone_from(&host);
        server.port = port;
        if host.is_empty() {
            continue;
        }
        if server.name.trim().is_empty() {
            server.name.clone_from(&host);
        }
        if server.id.is_empty() {
            server.id = lo_config::server_id_for(&format!("{host}:{port}"));
        }
        let repeated = servers
            .iter()
            .any(|other| other.address.eq_ignore_ascii_case(&host) && other.port == port);
        if repeated {
            continue;
        }
        servers.push(server);
    }
    Ok(servers)
}

/// Escribe la lista como JSON legible (indent 2, sin salto final, igual que
/// `config.json`).
pub fn export_json(path: &Path, servers: &[Server]) -> Result<(), ServersError> {
    let body = serde_json::to_string_pretty(servers)?;
    lo_paths::write_atomic(path, body.as_bytes())?;
    Ok(())
}

fn server_from_json(value: &Json) -> Option<Server> {
    if let Ok(server) = serde_json::from_value::<Server>(value.clone()) {
        if !server.address.trim().is_empty() {
            return Some(server);
        }
    }
    let map = value.as_object()?;
    let raw = ["address", "host", "ip"]
        .iter()
        .find_map(|key| map.get(*key).and_then(Json::as_str))
        .map(str::trim)
        .filter(|raw| !raw.is_empty())?;
    let (address, guessed) = split_host_port(raw);
    let port = map
        .get("port")
        .and_then(Json::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|port| *port != 0)
        .unwrap_or(guessed);
    let name = map
        .get("name")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map_or_else(|| address.clone(), str::to_owned);
    Some(Server {
        name,
        address,
        port,
        ..Server::default()
    })
}

// --- NBT ---------------------------------------------------------------

/// Descomprime (si hace falta) y parsea el NBT.
fn decode_nbt(bytes: &[u8]) -> Result<Value, String> {
    let mut plain = Vec::new();
    if bytes.starts_with(&[0x1F, 0x8B]) {
        let decoder = GzDecoder::new(bytes);
        decoder
            .take(MAX_DAT_BYTES)
            .read_to_end(&mut plain)
            .map_err(|err| format!("gzip: {err}"))?;
    } else {
        // Un `servers.dat` sin comprimir no es lo normal, pero se acepta: hay
        // herramientas que lo escriben así.
        plain.extend_from_slice(bytes);
    }
    fastnbt::from_bytes::<Value>(&plain).map_err(|err| err.to_string())
}

/// Serializa la lista y la comprime con gzip, como el juego.
fn encode_nbt(entries: &[DatServer]) -> Result<Vec<u8>, ServersError> {
    let list: Vec<Value> = entries.iter().map(DatServer::to_value).collect();
    let mut root: HashMap<String, Value> = HashMap::new();
    root.insert(ROOT_KEY.to_owned(), Value::List(list));
    let plain = fastnbt::to_bytes(&Value::Compound(root))
        .map_err(|err| ServersError::Nbt(err.to_string()))?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&plain)?;
    Ok(encoder.finish()?)
}

// --- direcciones -------------------------------------------------------

/// Separa `host`, `host:puerto`, `[v6]:puerto` y `v6` de la misma forma que el
/// juego. Un puerto que no es número se ignora y se queda el de por defecto.
#[must_use]
pub fn split_host_port(raw: &str) -> (String, u16) {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix('[') {
        if let Some((host, tail)) = rest.split_once(']') {
            let port = tail
                .strip_prefix(':')
                .and_then(|port| port.parse::<u16>().ok())
                .filter(|port| *port != 0)
                .unwrap_or(DEFAULT_PORT);
            return (host.to_owned(), port);
        }
    }
    match raw.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => {
            if port.is_empty() {
                return (host.to_owned(), DEFAULT_PORT);
            }
            match port.parse::<u16>() {
                Ok(parsed) if parsed != 0 => (host.to_owned(), parsed),
                _ => (raw.to_owned(), DEFAULT_PORT),
            }
        }
        _ => (raw.to_owned(), DEFAULT_PORT),
    }
}

/// Dirección de `config.json` normalizada: sin corchetes y sin esquema.
fn normalize_address(address: &str, port: u16) -> (String, u16) {
    let (host, guessed) = split_host_port(address);
    let port = if port == 0 { guessed } else { port };
    (host.trim().to_owned(), port)
}

/// Escribe `host[:puerto]` como lo escribe el juego.
fn format_address(host: &str, port: u16) -> String {
    let bare = host.trim();
    if bare.contains(':') {
        // IPv6: siempre con corchetes y con puerto explícito.
        return format!("[{bare}]:{port}");
    }
    if port == DEFAULT_PORT {
        bare.to_owned()
    } else {
        format!("{bare}:{port}")
    }
}

fn entry_matches(entry: &DatServer, host: &str, port: u16) -> bool {
    let (entry_host, entry_port) = entry.host_port();
    entry_host.eq_ignore_ascii_case(host) && entry_port == port
}

/// Nombre utilizable: recortado, nunca vacío y nunca más largo que el límite
/// de `config.json` (para que la fusión no cree entradas que luego no pasen la
/// validación).
fn clean_name(raw: &str, fallback: &str) -> String {
    let trimmed = raw.trim();
    let mut name: String = if trimmed.is_empty() {
        fallback.trim().to_owned()
    } else {
        trimmed.to_owned()
    };
    if name.chars().count() > MAX_SERVER_NAME {
        name = name.chars().take(MAX_SERVER_NAME).collect();
    }
    name
}

/// Nombre libre dentro del fichero. El juego permite nombres repetidos, pero la
/// fusión empareja por nombre: dos «Mi servidor» harían que la segunda entrada
/// pisara a la primera. Se añade ` (2)`, ` (3)`… como haría un humano.
///
/// `skip` es el índice de la entrada que se está renombrando: no cuenta como
/// ocupada, así que cambiarle el nombre no la compara consigo misma.
fn unique_name(entries: &[DatServer], base: &str, skip: Option<usize>) -> String {
    let taken = |candidate: &str| {
        entries
            .iter()
            .enumerate()
            .any(|(index, entry)| Some(index) != skip && entry.name == candidate)
    };
    if !taken(base) {
        return base.to_owned();
    }
    for bump in 2..=MAX_DAT_ENTRIES {
        let suffix = format!(" ({bump})");
        let room = MAX_SERVER_NAME.saturating_sub(suffix.chars().count());
        let trimmed: String = base.chars().take(room).collect();
        let candidate = format!("{trimmed}{suffix}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    base.to_owned()
}

fn as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}
