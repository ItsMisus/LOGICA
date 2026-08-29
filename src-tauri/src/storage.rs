//! Persistenza su disco in `~/.logica/`: chat e preferenze. Tutto JSON
//! leggibile a mano.
//!
//! ```text
//! ~/.logica/
//! |- config.json          cartelle consentite + profilo
//! |- preferences.json     tema, motore AI, modello
//! `- chats/chat_<id>.json una conversazione per file
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::ai::DEFAULT_OLLAMA_URL;
use crate::attachments::Attachment;

/* ---------------------------------- tipi ---------------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    /// "user" | "ai"
    pub role: String,
    pub content: String,
    pub timestamp: String,
    /// "text" | "notification"
    #[serde(rename = "type", default = "default_message_type")]
    pub message_type: String,
    /// File allegati al messaggio dall'utente, con il testo gia' letto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Attachment>>,
    /// Pagine consultate su internet per scrivere questa risposta.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<crate::web::SearchHit>>,
}

fn default_message_type() -> String {
    "text".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMetadata {
    pub id: String,
    pub title: String,
    pub created: String,
    pub updated: String,
    #[serde(default)]
    pub message_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredChat {
    #[serde(flatten)]
    pub metadata: ChatMetadata,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preferences {
    /// "dark" | "light"
    pub theme: String,
    pub use_premium_ai: bool,
    pub ollama_url: String,
    pub ollama_model: String,
    /// L'assistente puo' cercare su internet. Il campo manca nei file scritti
    /// dalle versioni precedenti: li' vale acceso.
    #[serde(default = "enabled")]
    pub web_search: bool,
}

fn enabled() -> bool {
    true
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            use_premium_ai: false,
            ollama_url: DEFAULT_OLLAMA_URL.to_string(),
            ollama_model: "llama3.1:8b".to_string(),
            web_search: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub monitored_folders: Vec<String>,
    /// "developer" | "media" | "office" | "custom"
    #[serde(default)]
    pub profile: String,
    /// Preferenze scritte a mano dall'utente, passate all'AI.
    #[serde(default)]
    pub user_preferences: String,
}

/* -------------------------------- percorsi -------------------------------- */

pub fn home_dir() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    if let Some(p) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    // Ultima spiaggia su Windows: HOMEDRIVE + HOMEPATH.
    match (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH")) {
        (Some(d), Some(p)) => Ok(PathBuf::from(d).join(p)),
        _ => Err("Impossibile determinare la cartella utente.".to_string()),
    }
}

pub fn logica_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".logica"))
}

fn chats_dir() -> Result<PathBuf, String> {
    Ok(logica_dir()?.join("chats"))
}

/// Crea `~/.logica/` e i file di default al primo avvio.
pub fn ensure_layout() -> Result<(), String> {
    let chats = chats_dir()?;
    fs::create_dir_all(&chats).map_err(|e| format!("Impossibile creare {}: {e}", chats.display()))?;

    let base = logica_dir()?;
    if !base.join("preferences.json").exists() {
        save_preferences(&Preferences::default())?;
    }
    if !base.join("config.json").exists() {
        save_config(&Config::default())?;
    }
    Ok(())
}

/// Solo lettere, numeri, `_` e `-`: un id che arriva dalla UI non deve poter
/// uscire dalla cartella delle chat.
fn safe_chat_id(chat_id: &str) -> Result<String, String> {
    let ok = !chat_id.is_empty()
        && chat_id.len() <= 64
        && chat_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(chat_id.to_string())
    } else {
        Err(format!("Identificativo chat non valido: '{chat_id}'"))
    }
}

fn chat_path(chat_id: &str) -> Result<PathBuf, String> {
    let id = safe_chat_id(chat_id)?;
    Ok(chats_dir()?.join(format!("chat_{id}.json")))
}

/* ------------------------------ lettura/scrittura ----------------------------- */

/// Scrive prima su un file temporaneo e poi rinomina: se l'app muore a meta'
/// il JSON originale resta intatto.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Impossibile creare {}: {e}", parent.display()))?;
    }
    let data = serde_json::to_string_pretty(value).map_err(|e| format!("Serializzazione fallita: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, data).map_err(|e| format!("Scrittura fallita su {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("Salvataggio fallito su {}: {e}", path.display()))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let data = fs::read_to_string(path).map_err(|e| format!("Lettura fallita di {}: {e}", path.display()))?;
    serde_json::from_str(&data).map_err(|e| format!("File illeggibile {}: {e}", path.display()))
}

/* ---------------------------------- chat ---------------------------------- */

pub fn save_chat(chat: &StoredChat) -> Result<(), String> {
    ensure_layout()?;
    let path = chat_path(&chat.metadata.id)?;
    let mut chat = chat.clone();
    chat.metadata.message_count = chat.messages.len();
    chat.metadata.updated = Utc::now().to_rfc3339();
    write_json(&path, &chat)
}

pub fn load_chat(chat_id: &str) -> Result<StoredChat, String> {
    read_json(&chat_path(chat_id)?)
}

pub fn delete_chat(chat_id: &str) -> Result<(), String> {
    let path = chat_path(chat_id)?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("Eliminazione fallita: {e}"))?;
    }
    Ok(())
}

/// Elenco delle chat, dalla piu' recente. I file illeggibili vengono saltati
/// invece di far fallire tutta la lista.
pub fn list_chats() -> Result<Vec<ChatMetadata>, String> {
    ensure_layout()?;
    let dir = chats_dir()?;
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(Vec::new()),
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(chat) = read_json::<StoredChat>(&path) {
            let mut meta = chat.metadata;
            meta.message_count = chat.messages.len();
            out.push(meta);
        }
    }
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    Ok(out)
}

/* ----------------------------- config e preferenze ---------------------------- */

pub fn load_preferences() -> Preferences {
    logica_dir()
        .and_then(|d| read_json::<Preferences>(&d.join("preferences.json")))
        .unwrap_or_default()
}

pub fn save_preferences(prefs: &Preferences) -> Result<(), String> {
    write_json(&logica_dir()?.join("preferences.json"), prefs)
}

pub fn load_config() -> Config {
    logica_dir()
        .and_then(|d| read_json::<Config>(&d.join("config.json")))
        .unwrap_or_default()
}

pub fn save_config(config: &Config) -> Result<(), String> {
    write_json(&logica_dir()?.join("config.json"), config)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// La home e' una variabile d'ambiente di processo: i test che la
    /// sostituiscono devono girare uno alla volta.
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Sposta ~/.logica in una cartella usa-e-getta per la durata del test.
    pub(crate) struct TempHome(PathBuf, #[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

    impl TempHome {
        pub(crate) fn new(tag: &str) -> Self {
            // Un test andato in panic avvelena il lock ma non rompe gli altri.
            let guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!("logica-{tag}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&dir).expect("creazione home finta");
            std::env::set_var("USERPROFILE", &dir);
            std::env::set_var("HOME", &dir);
            Self(dir, guard)
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample_chat(id: &str, title: &str) -> StoredChat {
        StoredChat {
            metadata: ChatMetadata {
                id: id.to_string(),
                title: title.to_string(),
                created: "2026-08-29T10:00:00Z".to_string(),
                updated: "2026-08-29T10:00:00Z".to_string(),
                message_count: 0,
            },
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: "user".to_string(),
                content: "ciao".to_string(),
                timestamp: "2026-08-29T10:00:00Z".to_string(),
                message_type: "text".to_string(),
                attachments: None,
                sources: None,
            }],
        }
    }

    #[test]
    fn chats_survive_a_round_trip() {
        let _home = TempHome::new("chats");
        ensure_layout().expect("layout");

        save_chat(&sample_chat("20260829_100000_aaaa", "prima")).expect("salvataggio");
        save_chat(&sample_chat("20260829_110000_bbbb", "seconda")).expect("salvataggio");

        let loaded = load_chat("20260829_100000_aaaa").expect("lettura");
        assert_eq!(loaded.metadata.title, "prima");
        assert_eq!(loaded.messages.len(), 1);
        // message_count viene ricalcolato al salvataggio, non fidandosi della UI.
        assert_eq!(loaded.metadata.message_count, 1);

        let list = list_chats().expect("elenco");
        assert_eq!(list.len(), 2);

        delete_chat("20260829_100000_aaaa").expect("eliminazione");
        assert_eq!(list_chats().expect("elenco").len(), 1);
        // Eliminare una chat che non c'e' piu' non e' un errore.
        delete_chat("20260829_100000_aaaa").expect("eliminazione idempotente");
    }

    #[test]
    fn preferences_and_config_default_when_missing() {
        let _home = TempHome::new("prefs");
        let prefs = load_preferences();
        assert_eq!(prefs.theme, "dark");
        assert!(!prefs.use_premium_ai);

        save_preferences(&Preferences { theme: "light".to_string(), ..Preferences::default() })
            .expect("salvataggio preferenze");
        assert_eq!(load_preferences().theme, "light");

        save_config(&Config { profile: "developer".to_string(), ..Config::default() }).expect("config");
        assert_eq!(load_config().profile, "developer");
    }

    #[test]
    fn chat_ids_cannot_escape_the_folder() {
        assert!(safe_chat_id("chat_20240815_103000").is_ok());
        assert!(safe_chat_id("../../etc/passwd").is_err());
        assert!(safe_chat_id("nome con spazi").is_err());
        assert!(safe_chat_id("").is_err());
    }
}
