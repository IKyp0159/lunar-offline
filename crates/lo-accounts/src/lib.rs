//! Sesión offline sobre `~/.lunarclient/settings/game/accounts.json`.
//!
//! Lunar ignora `--username` y toma el nick de `accounts.json`, así que el
//! launcher escribe ahí una cuenta `legacy` temporal, la marca como activa y la
//! borra al cerrar el juego. Reglas:
//!
//! - **El swap es aditivo**: no borra ni reescribe ninguna cuenta que ya
//!   exista; solo añade la offline y cambia `activeAccountLocalId`.
//! - **`swap.pending` se escribe antes de tocar `accounts.json`**: si la app
//!   muere a mitad, el siguiente arranque sabe que hay que restaurar.
//! - **La copia original se guarda una sola vez** (`accounts.original.json`) y
//!   el `activeAccountLocalId` original en `accounts.original.active`. Nunca se
//!   sobrescribe una copia existente.
//! - **La restauración fusiona**: gana la sesión más nueva, se elimina la
//!   cuenta offline temporal y se devuelve la activa original. Si el jugador
//!   inició sesión con otra cuenta mientras jugaba, esa cuenta sobrevive.
//! - **El fichero se lee tolerando basura**: JSON roto o una raíz que no es
//!   objeto se tratan como estructura vacía, nunca provocan un pánico.
//!
//! Todo se escribe con temporal + rename ([`lo_paths::write_atomic`]) para que
//! `accounts.json` nunca quede a medias.

use std::collections::HashSet;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lo_paths::LunarPaths;
use md5::{Digest, Md5};
use serde_json::{Map, Value, json};
use sha1::Sha1;
use thiserror::Error;
use uuid::Uuid;

/// Longitud máxima de un nick de Minecraft.
pub const NICK_MAX_LEN: usize = 16;
/// Prefijo con el que se calculan el `localId` y el UUID offline.
pub const OFFLINE_PREFIX: &str = "OfflinePlayer:";
/// Prefijo del `localId` estable del launcher.
pub const LOCAL_ID_PREFIX: &str = "LunarOffline:";
/// `userType` que escribe Lunar para la cuenta temporal.
pub const OFFLINE_USER_TYPE: &str = "legacy";
/// Nombre del proceso del launcher oficial en Windows.
pub const LAUNCHER_PROCESS: &str = "Lunar Client.exe";

/// Nick validado: `^[A-Za-z0-9_]{1,16}$`.
///
/// Se construye solo con [`Nick::parse`], así que dentro del programa un `Nick`
/// siempre es válido. El `Display` devuelve el nick tal cual, sin normalizar.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Nick(String);

impl Nick {
    /// Valida y construye un nick.
    ///
    /// Acepta de 1 a 16 caracteres ASCII
    /// alfanuméricos o `_`. Nada de espacios, guiones ni Unicode.
    pub fn parse(raw: &str) -> Result<Self, NickError> {
        let bytes = raw.as_bytes();
        if bytes.is_empty() {
            return Err(NickError::Empty);
        }
        if bytes.len() > NICK_MAX_LEN {
            return Err(NickError::TooLong { len: bytes.len() });
        }
        if let Some((idx, ch)) = raw.char_indices().find(|(_, ch)| !is_nick_char(*ch)) {
            return Err(NickError::InvalidCharacter { ch, at: idx });
        }
        Ok(Self(raw.to_owned()))
    }

    /// Nick válido como cadena.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_nick_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

impl fmt::Display for Nick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Nick {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Motivos por los que un nick no vale.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NickError {
    /// Cadena vacía.
    #[error("el nick no puede estar vacío")]
    Empty,
    /// Más de 16 caracteres.
    #[error("el nick no puede pasar de 16 caracteres (tiene {len})")]
    TooLong {
        /// Longitud recibida.
        len: usize,
    },
    /// Carácter no permitido.
    #[error("el nick solo admite A-Z, a-z, 0-9 y _ (encontré {ch:?} en la posición {at})")]
    InvalidCharacter {
        /// Carácter ofensivo.
        ch: char,
        /// Posición (en caracteres) dentro del nick.
        at: usize,
    },
}

/// Identidad offline completa de un nick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineSession {
    /// Nick validado.
    pub nick: Nick,
    /// UUID v3 `OfflinePlayer:<nick>` que se pasa a `--uuid`.
    pub uuid: Uuid,
    /// `localId` estable de 32 hex: `sha1("LunarOffline:" + nick.lower())[:32]`.
    pub local_id: String,
}

impl OfflineSession {
    /// Calcula UUID y `localId` para un nick.
    #[must_use]
    pub fn new(nick: Nick) -> Self {
        let uuid = offline_uuid(&nick);
        let local_id = local_id(&nick);
        Self {
            nick,
            uuid,
            local_id,
        }
    }
}

/// UUID offline con bits de versión 3 y variante RFC 4122.
///
/// Es el mismo que calcula cualquier servidor offline:
/// `md5("OfflinePlayer:" + nick)` con `digest[6] = (d[6] & 0x0F) | 0x30` y
/// `digest[8] = (d[8] & 0x3F) | 0x80`.
#[must_use]
pub fn offline_uuid(nick: &Nick) -> Uuid {
    let mut hasher = Md5::new();
    hasher.update(OFFLINE_PREFIX.as_bytes());
    hasher.update(nick.as_str().as_bytes());
    let mut digest: [u8; 16] = hasher.finalize().into();
    digest[6] = (digest[6] & 0x0F) | 0x30;
    digest[8] = (digest[8] & 0x3F) | 0x80;
    Uuid::from_bytes(digest)
}

/// `localId` estable por nick, para que dos arranques no dupliquen entrada.
///
/// `sha1("LunarOffline:" + nick.lower())[:32]`, en hexadecimal minúscula.
#[must_use]
pub fn local_id(nick: &Nick) -> String {
    let mut hasher = Sha1::new();
    hasher.update(LOCAL_ID_PREFIX.as_bytes());
    hasher.update(nick.as_str().to_ascii_lowercase().as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Escribir en un String no puede fallar.
        let _ = write!(hex, "{byte:02x}");
    }
    hex.truncate(32);
    hex
}

/// Documento `accounts.json` tal cual, con todas sus claves desconocidas
/// preservadas.
///
/// No se modela con structs cerrados a propósito: Lunar añade campos entre
/// versiones y perder uno podría romper el launcher oficial.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountsDocument {
    /// Contenido completo del fichero.
    pub root: Value,
}

impl AccountsDocument {
    /// Documento vacío, el que se usa cuando el fichero no se puede leer.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            root: json!({"activeAccountLocalId": "", "accounts": {}}),
        }
    }

    /// Normaliza un valor ya parseado: raíz objeto y `accounts` objeto.
    fn from_value(value: Value) -> Self {
        let Value::Object(mut map) = value else {
            return Self::empty();
        };
        if !map.get("accounts").is_some_and(Value::is_object) {
            map.insert("accounts".to_owned(), Value::Object(Map::new()));
        }
        Self {
            root: Value::Object(map),
        }
    }

    /// Construye un documento a partir de sus dos partes.
    fn compose(active_local_id: &str, accounts: Map<String, Value>) -> Self {
        let mut root = Map::new();
        root.insert(
            "activeAccountLocalId".to_owned(),
            Value::String(active_local_id.to_owned()),
        );
        root.insert("accounts".to_owned(), Value::Object(accounts));
        Self {
            root: Value::Object(root),
        }
    }

    /// `localId` de la cuenta activa, si la hay.
    #[must_use]
    pub fn active_local_id(&self) -> Option<&str> {
        self.root
            .get("activeAccountLocalId")?
            .as_str()
            .filter(|id| !id.is_empty())
    }

    /// Cuentas registradas, si el documento las tiene como objeto.
    #[must_use]
    pub fn accounts(&self) -> Option<&Map<String, Value>> {
        self.root.get("accounts").and_then(Value::as_object)
    }

    /// Nicks de todas las cuentas registradas, en el orden del fichero.
    #[must_use]
    pub fn nicks(&self) -> Vec<&str> {
        let Some(accounts) = self.accounts() else {
            return Vec::new();
        };
        accounts
            .values()
            .filter_map(|account| {
                account
                    .get("minecraftProfile")
                    .and_then(|profile| profile.get("name"))
                    .and_then(Value::as_str)
                    .or_else(|| account.get("username").and_then(Value::as_str))
            })
            .collect()
    }

    /// La cuenta con ese `localId` existe.
    #[must_use]
    pub fn contains(&self, local_id: &str) -> bool {
        self.accounts()
            .is_some_and(|accounts| accounts.contains_key(local_id))
    }

    /// Serializa con el mismo formato que `json.dumps(doc, indent=2)`.
    fn to_pretty(&self) -> Result<String, AccountsError> {
        Ok(serde_json::to_string_pretty(&self.root)?)
    }
}

/// Texto de un escalar de `accounts.json`, con la forma que escribe Lunar.
fn scalar_text(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

/// Texto de un campo opcional; ausente o nulo, cadena vacía.
fn str_or_empty(value: Option<&Value>) -> String {
    value.map_or_else(String::new, scalar_text)
}

/// Verdad de un valor JSON: nulo, falso, cero, cadena vacía, lista o mapa vacíos.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

/// Token de la cuenta como texto; ausente o falso, cadena vacía.
fn token_text(account: &Value) -> String {
    match account.get("accessToken") {
        None => String::new(),
        Some(value) if !truthy(Some(value)) => String::new(),
        Some(value) => scalar_text(value),
    }
}

/// Cuenta con token de Microsoft de verdad (no la offline que inyecta la app).
fn is_real_session(account: &Value) -> bool {
    let token = token_text(account);
    !token.is_empty() && token != "0" && !truthy(account.get("legacy"))
}

/// Entrada `legacy` sin token: basura de un arranque anterior de la app.
fn is_dead_offline(account: &Value) -> bool {
    truthy(account.get("legacy")) && str_or_empty(account.get("accessToken")) == "0"
}

/// Fusiona dos `accounts.json` sin perder nada.
///
/// - Toda cuenta de `incoming` que no esté en `base` se conserva: es un login
///   hecho mientras el swap estaba vivo.
/// - Si está en ambos, gana la que tenga sesión real más nueva, para no pisar
///   un token recién emitido con uno viejo.
/// - `drop_id` se elimina del resultado: es la cuenta offline temporal.
///
/// El documento resultante solo lleva `activeAccountLocalId` y `accounts`;
/// cualquier otra clave de nivel superior que traiga Lunar se descarta al
/// restaurar.
#[must_use]
pub fn merge(
    base: &AccountsDocument,
    incoming: &AccountsDocument,
    drop_id: &str,
) -> AccountsDocument {
    let mut result: Map<String, Value> = base
        .accounts()
        .map(|accounts| {
            accounts
                .iter()
                .filter(|(local_id, _)| local_id.as_str() != drop_id)
                .map(|(local_id, account)| (local_id.clone(), account.clone()))
                .collect()
        })
        .unwrap_or_default();

    if let Some(incoming_accounts) = incoming.accounts() {
        for (local_id, account) in incoming_accounts {
            if local_id == drop_id || !account.is_object() {
                continue;
            }
            let previous = result.get(local_id);
            let replace = match previous {
                None => true,
                Some(previous) => {
                    let incoming_real = is_real_session(account);
                    let previous_real = is_real_session(previous);
                    if incoming_real && !previous_real {
                        true
                    } else if incoming_real && previous_real {
                        str_or_empty(account.get("accessTokenExpiresAt"))
                            > str_or_empty(previous.get("accessTokenExpiresAt"))
                    } else {
                        false
                    }
                }
            };
            if replace {
                result.insert(local_id.clone(), account.clone());
            }
        }
    }

    let declared = base
        .active_local_id()
        .or_else(|| incoming.active_local_id())
        .unwrap_or_default()
        .to_owned();
    let active = if result.contains_key(&declared) {
        declared
    } else {
        result
            .iter()
            .find(|(_, account)| is_real_session(account))
            .map(|(local_id, _)| local_id.clone())
            .or_else(|| result.keys().next().cloned())
            .unwrap_or_default()
    };

    AccountsDocument::compose(&active, result)
}

/// Entrada que Lunar espera para la cuenta offline temporal.
fn offline_entry(nick: &Nick, session: &OfflineSession) -> Value {
    json!({
        "accessToken": "0",
        "accessTokenExpiresAt": "1970-01-01T00:00:00Z",
        "eligibleForMigration": false,
        "hasMultipleProfiles": false,
        "legacy": true,
        "persistent": false,
        "localId": session.local_id,
        "refreshToken": "",
        "minecraftProfile": {"id": session.uuid.to_string(), "name": nick.as_str()},
        "remoteId": "",
        "type": "Xbox",
        "username": nick.as_str(),
    })
}

/// Resultado de un swap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapOutcome {
    /// `localId` de la cuenta offline insertada.
    pub offline_local_id: String,
    /// `localId` de la cuenta activa antes del swap (se guardó para restaurar).
    pub previous_active: Option<String>,
    /// Se creó `accounts.original.json` en este swap (primera vez).
    pub created_backup: bool,
    /// Ya existía una cuenta con ese `localId` y se reutilizó.
    pub reused_entry: bool,
}

/// Resultado de una restauración.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// No había nada que restaurar.
    NothingPending,
    /// Se restauró; `active_local_id` es la cuenta que queda activa.
    Restored {
        /// Cuenta activa después de fusionar.
        active_local_id: Option<String>,
        /// Cuentas que desaparecieron por ser la offline temporal.
        removed: Vec<String>,
        /// El `accounts.json` actual no se pudo leer y se usó la copia intacta.
        from_backup: bool,
    },
    /// Se cerró una sesión y quedan otras vivas: el fichero **no** se restaura.
    SessionEnded {
        /// Sesiones que siguen abiertas.
        remaining: usize,
        /// Cuenta activa que queda en `accounts.json`.
        active_local_id: Option<String>,
    },
}

/// Una instancia del juego en marcha.
///
/// El marcador guarda una por proceso: dos juegos abiertos con la misma cuenta
/// offline son dos sesiones, y cerrar una no puede restaurar el fichero mientras
/// la otra sigue viva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSession {
    /// Identificador de la sesión (distinto del `localId`).
    pub id: String,
    /// Nick que se estaba lanzando.
    pub nick: String,
    /// `localId` de la cuenta temporal.
    pub offline_id: String,
    /// Momento en que se abrió la sesión (epoch en segundos).
    pub since_unix: i64,
}

/// Estado del marcador `swap.pending`.
///
/// Las claves `nick`, `local_id` y `since_unix` son las de la v1 y se siguen
/// escribiendo con la sesión más reciente, para que un lector antiguo del
/// marcador siga funcionando.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSwap {
    /// Nick que se estaba lanzando.
    pub nick: String,
    /// `localId` de la cuenta temporal.
    pub local_id: String,
    /// Momento en que se escribió el marcador (epoch en segundos).
    pub since_unix: i64,
    /// Sesiones vivas, en orden de apertura.
    pub sessions: Vec<PendingSession>,
}

/// Resultado de abrir una sesión con [`AccountsManager::swap_session`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapSession {
    /// Identificador de la sesión (lo que se pasa a
    /// [`AccountsManager::end_session`]).
    pub id: String,
    /// Nick de la sesión.
    pub nick: String,
    /// `localId` de la cuenta offline insertada.
    pub offline_local_id: String,
    /// `localId` de la cuenta activa antes del primer swap.
    pub previous_active: Option<String>,
    /// Se creó `accounts.original.json` en esta sesión (primera vez).
    pub created_backup: bool,
    /// Ya existía una cuenta con ese `localId` y se reutilizó.
    pub reused_entry: bool,
    /// Sesiones vivas después de abrir esta.
    pub active: usize,
}

/// Gestor de `accounts.json` y de sus copias de seguridad.
///
/// Una instancia por proceso. El bloqueo de fichero entre dos instancias
/// (`fd-lock`) queda para cuando la UI pueda abrirse dos veces; hoy el guard es
/// `official_launcher_running`.
#[derive(Debug, Clone)]
pub struct AccountsManager {
    paths: LunarPaths,
}

impl AccountsManager {
    /// Crea el gestor sobre un conjunto de rutas.
    #[must_use]
    pub fn new(paths: LunarPaths) -> Self {
        Self { paths }
    }

    /// Rutas que usa el gestor.
    #[must_use]
    pub fn paths(&self) -> &LunarPaths {
        &self.paths
    }

    /// Hay un swap a medias (marcador o copia de seguridad presentes).
    #[must_use]
    pub fn pending(&self) -> bool {
        self.paths.swap_marker().is_file() || self.paths.original_accounts().is_file()
    }

    /// Marcador de swap, si existe y se puede leer.
    #[must_use]
    pub fn pending_swap(&self) -> Option<PendingSwap> {
        let text = fs::read_to_string(self.paths.swap_marker()).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        if !value.is_object() {
            return None;
        }
        let nick = str_or_empty(value.get("nick"));
        let local_id = str_or_empty(value.get("offline_id"));
        let since_unix = value.get("since").and_then(Value::as_i64).unwrap_or(0);
        // Un marcador antiguo no trae `sessions`: se sintetiza
        // una sesión con lo que hay para que el resto del código vea lo mismo.
        let mut sessions: Vec<PendingSession> = value
            .get("sessions")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.is_object())
                    .map(|item| PendingSession {
                        id: str_or_empty(item.get("id")),
                        nick: str_or_empty(item.get("nick")),
                        offline_id: str_or_empty(item.get("offline_id")),
                        since_unix: item.get("since").and_then(Value::as_i64).unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if sessions.is_empty() && !local_id.is_empty() {
            sessions.push(PendingSession {
                id: local_id.clone(),
                nick: nick.clone(),
                offline_id: local_id.clone(),
                since_unix,
            });
        }
        Some(PendingSwap {
            nick,
            local_id,
            since_unix,
            sessions,
        })
    }

    /// Sesiones vivas según el marcador (vacío si no hay swap pendiente).
    #[must_use]
    pub fn active_sessions(&self) -> Vec<PendingSession> {
        self.pending_swap()
            .map(|pending| pending.sessions)
            .unwrap_or_default()
    }

    /// Lee `accounts.json` tolerando basura.
    #[must_use]
    pub fn read(&self) -> AccountsDocument {
        read_document(self.paths.accounts_file()).0
    }

    /// Escribe `swap.pending` **antes** de tocar `accounts.json`, guarda la
    /// copia original si es la primera vez e inserta la cuenta offline.
    ///
    /// Es la variante de una sola sesión (la que usa la CLI): cada
    /// llamada añade una sesión viva y deja las claves de la v1 apuntando a la
    /// más reciente. Para varias instancias con restauración al cerrar, usar
    /// [`AccountsManager::swap_session`] y [`AccountsManager::end_session`].
    pub fn swap(&self, nick: &Nick) -> Result<SwapOutcome, AccountsError> {
        let session = self.swap_session(nick)?;
        Ok(SwapOutcome {
            offline_local_id: session.offline_local_id,
            previous_active: session.previous_active,
            created_backup: session.created_backup,
            reused_entry: session.reused_entry,
        })
    }

    /// Abre una sesión de juego y devuelve su identificador.
    ///
    /// Una sesión más no reescribe `accounts.original.json` ni
    /// `accounts.original.active`: la copia buena es la del primer swap y
    /// pisarla con la cuenta offline perdería las premium.
    pub fn swap_session(&self, nick: &Nick) -> Result<SwapSession, AccountsError> {
        let accounts_path = self.paths.accounts_file().to_path_buf();
        ensure_parent(&accounts_path)?;
        // Los directorios se crean aquí y no al construir el manager,
        // en el momento del swap para que la copia y el marcador tengan sitio.
        fs::create_dir_all(self.paths.app_dir())
            .map_err(|err| io_error(self.paths.app_dir(), err))?;

        let current = self.read();
        let existing = self.active_sessions();
        let previous_active = current.active_local_id().map(str::to_owned);
        let created_backup = !self.paths.swap_marker().exists();

        if created_backup {
            // Copia solo la primera vez: si ya hay marca, la copia buena existe
            // y reescribirla con la cuenta offline perdería las premium.
            if accounts_path.is_file() {
                fs::copy(&accounts_path, self.paths.original_accounts())
                    .map_err(|err| io_error(self.paths.original_accounts(), err))?;
            }
            let previous_text = previous_active.clone().unwrap_or_default();
            lo_paths::write_atomic(self.paths.original_active(), previous_text.as_bytes())
                .map_err(|err| io_error(self.paths.original_active(), err))?;
        }

        let session = OfflineSession::new(nick.clone());
        let reused_entry = current.contains(&session.local_id);

        let mut accounts = current.accounts().cloned().unwrap_or_default();
        accounts.insert(session.local_id.clone(), offline_entry(nick, &session));

        let mut sessions = existing;
        let id = next_session_id(&sessions);
        sessions.push(PendingSession {
            id: id.clone(),
            nick: nick.as_str().to_owned(),
            offline_id: session.local_id.clone(),
            since_unix: unix_now(),
        });
        self.write_marker(&sessions)?;

        let document = AccountsDocument::compose(&session.local_id, accounts);
        write_document(&accounts_path, &document)?;

        Ok(SwapSession {
            id,
            nick: nick.as_str().to_owned(),
            offline_local_id: session.local_id,
            previous_active,
            created_backup,
            reused_entry,
            active: sessions.len(),
        })
    }

    /// Cierra una sesión.
    ///
    /// Mientras quede otra viva el fichero **no** se restaura: solo se quita la
    /// sesión del marcador y, si la activa era esa, se pasa el testigo a la
    /// última que sigue abierta. Con la última sesión se restaura de verdad.
    pub fn end_session(&self, session_id: &str) -> Result<RestoreOutcome, AccountsError> {
        let Some(pending) = self.pending_swap() else {
            return Ok(RestoreOutcome::NothingPending);
        };
        let mut sessions = pending.sessions;
        let Some(index) = sessions.iter().position(|session| session.id == session_id) else {
            // Sesión desconocida (doble cierre, marcador de otra versión): no se
            // toca el fichero.
            return Ok(RestoreOutcome::SessionEnded {
                remaining: sessions.len(),
                active_local_id: self.read().active_local_id().map(str::to_owned),
            });
        };
        let ended = sessions.remove(index);

        if sessions.is_empty() {
            return self.restore();
        }

        let accounts_path = self.paths.accounts_file().to_path_buf();
        let current = self.read();
        let active = current.active_local_id().map(str::to_owned);
        let replacement = sessions
            .last()
            .map(|session| session.offline_id.clone())
            .unwrap_or_default();
        let accounts = current.accounts().cloned().unwrap_or_default();

        // Si la que se cierra era la activa, la que sigue manda. Si no, no se
        // toca el `activeAccountLocalId` del usuario.
        let new_active = if active.as_deref() == Some(ended.offline_id.as_str()) {
            replacement
        } else {
            active.unwrap_or(replacement)
        };
        let document = AccountsDocument::compose(&new_active, accounts);
        write_document(&accounts_path, &document)?;
        self.write_marker(&sessions)?;
        Ok(RestoreOutcome::SessionEnded {
            remaining: sessions.len(),
            active_local_id: (!new_active.is_empty()).then_some(new_active),
        })
    }

    /// Abre una sesión y devuelve un guard que la cierra al soltarse.
    pub fn swap_guard(&self, nick: &Nick) -> Result<SwapGuard, AccountsError> {
        let session = self.swap_session(nick)?;
        Ok(SwapGuard {
            manager: self.clone(),
            session_id: session.id,
            ended: false,
        })
    }

    fn write_marker(&self, sessions: &[PendingSession]) -> Result<(), AccountsError> {
        let last = sessions.last();
        let marker = json!({
            // Claves de la v1, para lectores que no conocen `sessions`.
            "nick": last.map(|session| session.nick.as_str()).unwrap_or_default(),
            "offline_id": last.map(|session| session.offline_id.as_str()).unwrap_or_default(),
            "since": last.map_or(0, |session| session.since_unix),
            "sessions": sessions
                .iter()
                .map(|session| json!({
                    "id": session.id,
                    "nick": session.nick,
                    "offline_id": session.offline_id,
                    "since": session.since_unix,
                }))
                .collect::<Vec<Value>>(),
        });
        lo_paths::write_atomic(self.paths.swap_marker(), marker.to_string().as_bytes())
            .map_err(|err| io_error(self.paths.swap_marker(), err))
    }

    /// Fusiona la copia original con el estado actual, elimina la cuenta offline
    /// temporal y devuelve la activa original.
    pub fn restore(&self) -> Result<RestoreOutcome, AccountsError> {
        let original_path = self.paths.original_accounts().to_path_buf();
        let accounts_path = self.paths.accounts_file().to_path_buf();
        let marker_path = self.paths.swap_marker().to_path_buf();

        let offline_id = self
            .pending_swap()
            .map(|pending| pending.local_id)
            .unwrap_or_default();

        if !original_path.is_file() {
            // Marcador huérfano: se borra y accounts.json no se toca.
            remove_if_exists(&marker_path)?;
            return Ok(RestoreOutcome::NothingPending);
        }

        let (original, _) = read_document(&original_path);
        let (current, current_readable) = read_document(&accounts_path);
        let merged = merge(&original, &current, &offline_id);

        let original_ids: HashSet<&str> = original
            .accounts()
            .map(|accounts| accounts.keys().map(String::as_str).collect())
            .unwrap_or_default();
        // Las cuentas que había en el fichero vivo: lo que falte después de
        // filtrar es lo que se ha eliminado en esta restauración.
        let before: Vec<String> = current
            .accounts()
            .map(|accounts| accounts.keys().cloned().collect())
            .unwrap_or_default();

        let mut kept = Map::new();
        if let Some(accounts) = merged.accounts() {
            for (local_id, account) in accounts {
                // Las offline muertas que no estaban en el original son basura
                // de arranques anteriores; las del usuario se conservan.
                if original_ids.contains(local_id.as_str()) || !is_dead_offline(account) {
                    kept.insert(local_id.clone(), account.clone());
                }
            }
        }
        let removed: Vec<String> = before
            .into_iter()
            .filter(|local_id| !kept.contains_key(local_id))
            .collect();

        let active = merged.active_local_id().unwrap_or_default().to_owned();
        let document = AccountsDocument::compose(&active, kept);
        write_document(&accounts_path, &document)?;

        remove_if_exists(&original_path)?;
        remove_if_exists(&marker_path)?;

        Ok(RestoreOutcome::Restored {
            active_local_id: (!active.is_empty()).then_some(active),
            removed,
            from_backup: !current_readable,
        })
    }

    /// Borra marcador y copias una vez restaurado.
    pub fn clear_pending(&self) -> Result<(), AccountsError> {
        remove_if_exists(self.paths.swap_marker())?;
        remove_if_exists(self.paths.original_accounts())?;
        remove_if_exists(self.paths.original_active())?;
        Ok(())
    }

    /// Comprueba si el launcher oficial está abierto (`Lunar Client.exe`).
    ///
    /// Vive aquí y no en la UI porque el swap depende de ello: si el oficial
    /// está abierto, puede reescribir `accounts.json` en cualquier momento.
    #[must_use]
    pub fn official_launcher_running(&self) -> bool {
        let system = sysinfo::System::new_all();
        system.processes().values().any(|process| {
            process
                .name()
                .to_string_lossy()
                .eq_ignore_ascii_case(LAUNCHER_PROCESS)
        })
    }
}

fn read_document(path: &Path) -> (AccountsDocument, bool) {
    let Ok(text) = fs::read_to_string(path) else {
        return (AccountsDocument::empty(), false);
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(value @ Value::Object(_)) => (AccountsDocument::from_value(value), true),
        Ok(_) | Err(_) => (AccountsDocument::empty(), false),
    }
}

fn write_document(path: &Path, document: &AccountsDocument) -> Result<(), AccountsError> {
    let body = document.to_pretty()?;
    lo_paths::write_atomic(path, body.as_bytes()).map_err(|err| io_error(path, err))
}

fn ensure_parent(path: &Path) -> Result<(), AccountsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| io_error(parent, err))?;
    }
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<(), AccountsError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(io_error(path, err)),
    }
}

fn io_error(path: &Path, source: std::io::Error) -> AccountsError {
    AccountsError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

/// Identificador de sesión único dentro del marcador.
///
/// No usa relojes de alta resolución a propósito: el `since` de la sesión es el
/// que se ve en la UI, y el id solo tiene que no repetirse entre las sesiones
/// vivas. Se recorre el sufijo hasta encontrar uno libre.
fn next_session_id(existing: &[PendingSession]) -> String {
    let base = format!("s{}", unix_now());
    if !existing.iter().any(|session| session.id == base) {
        return base;
    }
    for suffix in 2..=u32::MAX {
        let candidate = format!("{base}-{suffix}");
        if !existing.iter().any(|session| session.id == candidate) {
            return candidate;
        }
    }
    // Imposible en la práctica: 4 000 millones de sesiones en el mismo segundo.
    format!("{base}-{}", existing.len() + 1)
}

/// Guard que cierra la sesión al soltarse.
///
/// Pensado para el worker que vigila el proceso del juego: si la UI se cierra,
/// el hilo muere y el `Drop` deja el marcador coherente. Un `Drop` no puede
/// devolver el error, así que se ignora a propósito; para enterarse hay que
/// llamar a [`SwapGuard::end`].
#[derive(Debug)]
pub struct SwapGuard {
    manager: AccountsManager,
    session_id: String,
    ended: bool,
}

impl SwapGuard {
    /// Identificador de la sesión.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Cierra la sesión de forma explícita y devuelve el resultado.
    pub fn end(mut self) -> Result<RestoreOutcome, AccountsError> {
        self.ended = true;
        self.manager.end_session(&self.session_id)
    }
}

impl Drop for SwapGuard {
    fn drop(&mut self) {
        if !self.ended {
            let _ = self.manager.end_session(&self.session_id);
        }
    }
}

/// Fallos del ciclo de vida de `accounts.json`.
#[derive(Debug, Error)]
pub enum AccountsError {
    /// Fallo de E/S.
    #[error("fallo de E/S sobre {path}: {source}")]
    Io {
        /// Fichero afectado.
        path: PathBuf,
        /// Error original.
        #[source]
        source: std::io::Error,
    },
    /// El JSON no se pudo reinterpretar al escribir.
    #[error("accounts.json no se pudo reconstruir: {0}")]
    Json(#[from] serde_json::Error),
    /// La raíz no es un objeto y no hay forma de arreglarlo.
    #[error("accounts.json no contiene un objeto en la raíz")]
    NotAnObject,
    /// No hay copia original pero el marcador dice que sí.
    #[error("el marcador de swap existe pero falta la copia {0}")]
    MissingBackup(PathBuf),
    /// Otra instancia tiene el bloqueo.
    #[error("otra instancia del launcher está trabajando con accounts.json")]
    Locked,
}
