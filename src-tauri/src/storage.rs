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
    /// Cartelle e file guardati sul disco per scrivere questa risposta.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<crate::tools::ToolUse>>,
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
    /// Motore scelto per questa conversazione: `claude`, oppure
    /// `ollama:<modello>`. Assente = quello delle impostazioni.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
}

/// Una conversazione in cui il testo cercato compare, con il pezzo di
/// messaggio da mostrare sotto il titolo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMatch {
    #[serde(flatten)]
    pub metadata: ChatMetadata,
    /// Il primo punto in cui compare, con qualche parola intorno. Vuoto
    /// quando la corrispondenza sta solo nel titolo.
    pub snippet: String,
    /// In quanti messaggi compare: distingue un accenno da un discorso.
    pub hits: usize,
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
    /// L'assistente puo' guardare dentro le cartelle autorizzate. Spento
    /// finche' non lo si accende: e' l'unica preferenza che apre una porta sui
    /// file dell'utente.
    #[serde(default)]
    pub file_tools: bool,
    /// L'assistente ricorda fra una chat e l'altra. Acceso: quello che tiene
    /// e' visibile e cancellabile dalle impostazioni.
    #[serde(default = "enabled")]
    pub memory: bool,
    /// L'assistente consulta l'indice dei documenti. Spento finche' un indice
    /// non esiste: senza, ogni domanda pagherebbe una chiamata a vuoto.
    #[serde(default)]
    pub documents: bool,
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
            file_tools: false,
            memory: true,
            documents: false,
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

/// Cambia il titolo senza toccare altro. La data di aggiornamento resta
/// quella dell'ultimo messaggio: rinominare una conversazione vecchia non
/// deve farla risalire in cima alla lista.
pub fn rename_chat(chat_id: &str, title: &str) -> Result<ChatMetadata, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("Il titolo non puo' essere vuoto.".to_string());
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("Il titolo non puo' superare {MAX_TITLE_CHARS} caratteri."));
    }

    let mut chat = load_chat(chat_id)?;
    chat.metadata.title = title.to_string();
    chat.metadata.message_count = chat.messages.len();
    write_json(&chat_path(chat_id)?, &chat)?;
    Ok(chat.metadata)
}

/// Titolo lungo quanto una frase, non quanto un messaggio.
const MAX_TITLE_CHARS: usize = 120;

/// Copia una conversazione in una nuova, con i suoi messaggi. Serve per
/// ripartire da un punto senza perdere il filo di prima: si duplica, e nella
/// copia si cambia strada.
pub fn duplicate_chat(chat_id: &str) -> Result<ChatMetadata, String> {
    let source = load_chat(chat_id)?;
    let now = Utc::now();

    let copy = StoredChat {
        metadata: ChatMetadata {
            id: new_chat_id(),
            title: format!("{} (copia)", source.metadata.title),
            created: now.to_rfc3339(),
            updated: now.to_rfc3339(),
            message_count: source.messages.len(),
            // La copia continua col motore su cui si stava parlando.
            engine: source.metadata.engine.clone(),
        },
        messages: source.messages,
    };
    save_chat(&copy)?;
    load_chat(&copy.metadata.id).map(|c| c.metadata)
}

/// Un id come quello che genera il frontend: data leggibile piu' quattro
/// caratteri a caso, perche' due copie fatte nello stesso secondo non si
/// sovrascrivano.
fn new_chat_id() -> String {
    use rand::Rng as _;
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    let tail: String =
        (0..4).map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char).collect();
    format!("{}_{tail}", Utc::now().format("%Y%m%d_%H%M%S"))
}

/// La conversazione in un file da tenere fuori dall'app: Markdown per
/// leggerla, JSON per riaverla intera. Torna il testo, non lo scrive: dove
/// salvarlo lo decide chi ha chiesto l'esportazione.
pub fn export_chat(chat_id: &str, format: &str) -> Result<String, String> {
    let chat = load_chat(chat_id)?;
    match format {
        "json" => serde_json::to_string_pretty(&chat).map_err(|e| format!("Esportazione fallita: {e}")),
        "markdown" => Ok(as_markdown(&chat)),
        other => Err(format!("Formato non riconosciuto: '{other}'")),
    }
}

fn as_markdown(chat: &StoredChat) -> String {
    let mut out = format!("# {}\n\n", chat.metadata.title);
    out.push_str(&format!("*Conversazione con Logica, {}*\n", human_date(&chat.metadata.created)));

    for message in &chat.messages {
        let who = match message.role.as_str() {
            "user" => "Io",
            _ if message.message_type == "notification" => "Nota dell'app",
            _ => "Logica",
        };
        out.push_str(&format!("\n## {who}\n\n"));

        if let Some(files) = &message.attachments {
            for file in files {
                let note = match (&file.error, file.kind) {
                    (Some(e), _) => format!(" (non letto: {e})"),
                    (None, crate::attachments::Kind::Image) => " (immagine)".to_string(),
                    _ => String::new(),
                };
                out.push_str(&format!("*Allegato: {}{note}*\n", file.name));
            }
            if !files.is_empty() {
                out.push('\n');
            }
        }

        out.push_str(message.content.trim());
        out.push('\n');

        if let Some(sources) = &message.sources {
            if !sources.is_empty() {
                out.push_str("\nFonti:\n");
                for source in sources {
                    let title = if source.title.trim().is_empty() { &source.url } else { &source.title };
                    out.push_str(&format!("- [{title}]({})\n", source.url));
                }
            }
        }
    }
    out
}

/// Data in italiano per l'intestazione dell'esportazione. Se la data salvata
/// non si legge, si lascia com'e': meglio di una data finta.
fn human_date(iso: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(iso) {
        Ok(date) => date.format("%d/%m/%Y").to_string(),
        Err(_) => iso.to_string(),
    }
}

/* -------------------------------- ricerca --------------------------------- */

/// Parole intorno alla corrispondenza mostrate nell'anteprima.
const SNIPPET_CONTEXT: usize = 60;

/// Risultati oltre i quali la lista non aiuta piu': si restringe la ricerca.
const MAX_MATCHES: usize = 50;

/// Cerca dentro le conversazioni, non solo nei titoli: il titolo lo scrive il
/// modello in cinque parole, quello che si ricorda e' una frase detta dentro.
///
/// Confronto in minuscolo, sottostringa secca: chi cerca "fattur" trova
/// "fatturazione". Niente accenti normalizzati e niente parole staccate,
/// perche' una ricerca che indovina troppo non si capisce piu' perche' abbia
/// trovato una cosa.
pub fn search_chats(query: &str) -> Result<Vec<ChatMatch>, String> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(Vec::new());
    }

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
        let Ok(chat) = read_json::<StoredChat>(&path) else { continue };

        let mut hits = 0usize;
        let mut snippet = String::new();
        for message in &chat.messages {
            if !message.content.to_lowercase().contains(&needle) {
                continue;
            }
            hits += 1;
            if snippet.is_empty() {
                snippet = snippet_around(&message.content, &needle);
            }
        }

        let in_title = chat.metadata.title.to_lowercase().contains(&needle);
        if hits == 0 && !in_title {
            continue;
        }

        let mut meta = chat.metadata;
        meta.message_count = chat.messages.len();
        out.push(ChatMatch { metadata: meta, snippet, hits });
    }

    out.sort_by(|a, b| b.metadata.updated.cmp(&a.metadata.updated));
    out.truncate(MAX_MATCHES);
    Ok(out)
}

/// Il testo intorno alla prima occorrenza, su una riga sola. Gli a capo
/// diventano spazi: in due righe di anteprima contano le parole, non
/// l'impaginazione di quello che era un blocco di codice.
fn snippet_around(content: &str, needle: &str) -> String {
    let flat: Vec<char> = content.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect();
    let lower: String = flat.iter().collect::<String>().to_lowercase();

    // Posizione in caratteri, non in byte: i tagli devono cadere fra lettere
    // intere anche quando ci sono accenti o emoji.
    let start_char = match lower.find(needle) {
        Some(byte_index) => lower[..byte_index].chars().count(),
        None => 0,
    };

    let from = start_char.saturating_sub(SNIPPET_CONTEXT);
    let to = (start_char + needle.chars().count() + SNIPPET_CONTEXT).min(flat.len());

    let mut snippet = String::new();
    if from > 0 {
        snippet.push('…');
    }
    snippet.extend(flat[from..to].iter());
    if to < flat.len() {
        snippet.push('…');
    }
    snippet
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
                engine: None,
            },
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: "user".to_string(),
                content: "ciao".to_string(),
                timestamp: "2026-08-29T10:00:00Z".to_string(),
                message_type: "text".to_string(),
                attachments: None,
                sources: None,
                tools: None,
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

    /// Chat con messaggi scritti a mano, per cercarci dentro.
    fn chat_with(id: &str, title: &str, texts: &[&str]) -> StoredChat {
        let mut chat = sample_chat(id, title);
        chat.messages = texts
            .iter()
            .enumerate()
            .map(|(i, text)| ChatMessage {
                id: format!("m{i}"),
                role: if i % 2 == 0 { "user" } else { "ai" }.to_string(),
                content: (*text).to_string(),
                timestamp: "2026-08-29T10:00:00Z".to_string(),
                message_type: "text".to_string(),
                attachments: None,
                sources: None,
                tools: None,
            })
            .collect();
        chat
    }

    #[test]
    fn search_looks_inside_the_messages() {
        let _home = TempHome::new("ricerca");
        ensure_layout().expect("layout");

        save_chat(&chat_with(
            "20260829_100000_aaaa",
            "Conti di casa",
            &["quanto costa la caldaia?", "Dipende dal modello: una a condensazione parte da 1200 euro."],
        ))
        .expect("salvataggio");
        save_chat(&chat_with("20260829_110000_bbbb", "Caldaia nel titolo", &["ciao"]))
            .expect("salvataggio");
        save_chat(&chat_with("20260829_120000_cccc", "Altro", &["parliamo di vacanze"]))
            .expect("salvataggio");

        let found = search_chats("caldaia").expect("ricerca");
        let ids: Vec<&str> = found.iter().map(|m| m.metadata.id.as_str()).collect();
        assert_eq!(ids.len(), 2, "trovate: {ids:?}");
        assert!(ids.contains(&"20260829_100000_aaaa"));
        // Il titolo conta quanto il contenuto, anche senza corrispondenze dentro.
        assert!(ids.contains(&"20260829_110000_bbbb"));

        let dentro = found.iter().find(|m| m.metadata.id == "20260829_100000_aaaa").expect("chat");
        assert_eq!(dentro.hits, 1);
        assert!(dentro.snippet.contains("caldaia"), "anteprima: {}", dentro.snippet);

        let solo_titolo = found.iter().find(|m| m.metadata.id == "20260829_110000_bbbb").expect("chat");
        assert_eq!(solo_titolo.hits, 0);
        assert_eq!(solo_titolo.snippet, "");

        // Maiuscole e minuscole non contano, una parola assente non trova niente.
        assert_eq!(search_chats("CALDAIA").expect("ricerca").len(), 2);
        assert!(search_chats("bicicletta").expect("ricerca").is_empty());
        assert!(search_chats("   ").expect("ricerca").is_empty());
    }

    #[test]
    fn the_snippet_shows_the_words_around_the_match() {
        let prima = "a ".repeat(100);
        let content = format!("{prima}parolachiave e poi il resto del discorso");
        let snippet = snippet_around(&content, "parolachiave");

        assert!(snippet.starts_with('…'), "manca il taglio a sinistra: {snippet}");
        assert!(snippet.contains("parolachiave"));
        // Il contesto e' limitato: non torna tutto il messaggio.
        assert!(snippet.chars().count() < content.chars().count());
    }

    #[test]
    fn renaming_keeps_the_place_in_the_list() {
        let _home = TempHome::new("rinomina");
        ensure_layout().expect("layout");
        save_chat(&sample_chat("20260829_100000_aaaa", "senza nome")).expect("salvataggio");

        let prima = load_chat("20260829_100000_aaaa").expect("lettura").metadata.updated;
        let meta = rename_chat("20260829_100000_aaaa", "  Conti di casa  ").expect("rinomina");

        assert_eq!(meta.title, "Conti di casa");
        assert_eq!(load_chat("20260829_100000_aaaa").expect("lettura").metadata.title, "Conti di casa");
        // Rinominare non e' un aggiornamento: la chat non risale in cima.
        assert_eq!(load_chat("20260829_100000_aaaa").expect("lettura").metadata.updated, prima);

        assert!(rename_chat("20260829_100000_aaaa", "   ").is_err());
    }

    #[test]
    fn duplicating_leaves_the_original_alone() {
        let _home = TempHome::new("duplica");
        ensure_layout().expect("layout");
        save_chat(&chat_with("20260829_100000_aaaa", "Progetto", &["primo", "secondo"]))
            .expect("salvataggio");

        let copy = duplicate_chat("20260829_100000_aaaa").expect("duplicazione");
        assert_eq!(copy.title, "Progetto (copia)");
        assert_ne!(copy.id, "20260829_100000_aaaa");
        assert_eq!(copy.message_count, 2);

        let copied = load_chat(&copy.id).expect("lettura copia");
        assert_eq!(copied.messages.len(), 2);
        assert_eq!(copied.messages[0].content, "primo");
        // L'originale resta dov'era, con il suo titolo.
        assert_eq!(load_chat("20260829_100000_aaaa").expect("lettura").metadata.title, "Progetto");
    }

    #[test]
    fn export_writes_markdown_and_json() {
        let _home = TempHome::new("esporta");
        ensure_layout().expect("layout");
        save_chat(&chat_with("20260829_100000_aaaa", "Caldaia", &["quanto costa?", "1200 euro."]))
            .expect("salvataggio");

        let markdown = export_chat("20260829_100000_aaaa", "markdown").expect("markdown");
        assert!(markdown.starts_with("# Caldaia"));
        assert!(markdown.contains("## Io\n\nquanto costa?"));
        assert!(markdown.contains("## Logica\n\n1200 euro."));

        let json = export_chat("20260829_100000_aaaa", "json").expect("json");
        let back: StoredChat = serde_json::from_str(&json).expect("json rileggibile");
        assert_eq!(back.messages.len(), 2);

        assert!(export_chat("20260829_100000_aaaa", "pdf").is_err());
    }

    #[test]
    fn chat_ids_cannot_escape_the_folder() {
        assert!(safe_chat_id("chat_20240815_103000").is_ok());
        assert!(safe_chat_id("../../etc/passwd").is_err());
        assert!(safe_chat_id("nome con spazi").is_err());
        assert!(safe_chat_id("").is_err());
    }
}
