//! LOGICA: il lato conversazionale dell'app.
//!
//! Il flusso non e' affidato interamente al modello. Le decisioni che toccano
//! il disco (quali cartelle scansionare, quando proporre, quando applicare,
//! quando annullare) vengono riconosciute da regole deterministiche in Rust;
//! al modello resta il linguaggio. Cosi' l'app funziona anche con un 7B locale
//! che sbaglia il JSON, e continua a rispondere pure senza AI.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ai_analyzer::{self, Grouping, OrganizationProposal};
use crate::scanner::FileInfo;
use crate::storage;

/// Prompt di sistema di LOGICA. Descrive personalita' e vincoli; il controllo
/// del flusso resta comunque in `classify`.
const SYSTEM_PROMPT: &str = "Tu sei LOGICA, un assistente che aiuta a organizzare i file del computer.

PERSONALITA'
- Amichevole ma professionale, mai prolissa.
- Curiosa: fai domande invece di dare per scontate le preferenze dell'utente.
- Decisa: quando hai le informazioni, proponi chiaramente.
- Rispondi sempre in italiano naturale, massimo 4 frasi.

COSA SAI FARE
- Scansionare cartelle (Download, Desktop, Documenti, o un percorso indicato).
- Proporre uno schema di organizzazione per tipo, per data o per progetto.
- Applicare lo spostamento dei file solo dopo l'approvazione esplicita.
- Annullare l'ultima operazione se l'utente scrive 'annulla'.

REGOLE
- Non promettere di aver spostato file: lo spostamento avviene solo dopo che
  l'utente approva la proposta mostrata nella chat.
- Non inventare numeri: usa solo i dati della scansione che trovi nel contesto.
- Se l'utente non ha ancora indicato una cartella, chiedigliela.
- Se l'utente parla d'altro, rispondi cordialmente e riporta il discorso ai file.
- Se hai una proposta completa da fare, includila come blocco
  <PROPOSAL_JSON>{\"schema\": {...}, \"rules\": [...], \"rationale\": \"...\"}</PROPOSAL_JSON>
  dopo una frase di introduzione.";

const CHAT_TIMEOUT: Duration = Duration::from_secs(120);
/// Turni di storico inviati al modello: piu' di cosi' e' solo latenza.
const MAX_HISTORY_TURNS: usize = 12;

/* ---------------------------------- tipi ---------------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurn {
    /// "user" | "ai"
    pub role: String,
    pub content: String,
}

/// Parametri del motore AI scelti nelle impostazioni.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default)]
    pub use_premium_ai: bool,
    #[serde(default = "default_url")]
    pub ollama_url: String,
    #[serde(default = "default_model")]
    pub ollama_model: String,
    #[serde(default)]
    pub user_preferences: String,
}

fn default_url() -> String {
    ai_analyzer::DEFAULT_OLLAMA_URL.to_string()
}

fn default_model() -> String {
    "mistral".to_string()
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            use_premium_ai: false,
            ollama_url: default_url(),
            ollama_model: default_model(),
            user_preferences: String::new(),
        }
    }
}

/// Cosa l'utente sta chiedendo, dedotto dal messaggio e dallo stato della chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Scansiona le cartelle indicate.
    Scan(Vec<String>),
    /// Prepara una proposta con questo criterio.
    Propose(Grouping),
    /// Applica la proposta corrente.
    Apply,
    /// Annulla l'ultima applicazione.
    Undo,
    /// Nessuna azione: si chiacchiera.
    Chat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiStatus {
    pub ollama: bool,
    pub claude: bool,
    pub models: Vec<String>,
}

/* ------------------------- riconoscimento intenzioni ------------------------ */

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'á' => 'a',
            'è' | 'é' => 'e',
            'ì' | 'í' => 'i',
            'ò' | 'ó' => 'o',
            'ù' | 'ú' => 'u',
            other => other,
        })
        .collect()
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

fn is_undo(msg: &str) -> bool {
    contains_any(
        msg,
        &["annulla", "undo", "ripristina", "rimetti a posto", "torna indietro", "disfa", "rollback"],
    )
}

fn is_apply(msg: &str) -> bool {
    // Frasi corte di conferma: "vai", "ok", "perfetto". Su un messaggio lungo
    // queste parole sono spesso incidentali, quindi chiediamo l'intento esplicito.
    let short = msg.split_whitespace().count() <= 4;
    contains_any(msg, &["applica", "procedi", "esegui", "conferma", "fallo", "muovi i file"])
        || (short && contains_any(msg, &["vai", "ok", "perfetto", "si", "certo", "d'accordo", "avanti"]))
}

/// Criterio di organizzazione citato nel messaggio, se c'e'.
pub fn detect_grouping(msg: &str) -> Option<Grouping> {
    if contains_any(msg, &["progetto", "progetti", "cartella di origine", "provenienza"]) {
        return Some(Grouping::SourceFolder);
    }
    if contains_any(msg, &["mese", "mensile", "data", "cronologic", "quando"]) {
        // "solo per anno" e' un caso diverso da "per data".
        if contains_any(msg, &["solo anno", "per anno", "annuale"]) && !msg.contains("mese") {
            return Some(Grouping::Year);
        }
        return Some(Grouping::Month);
    }
    if contains_any(msg, &["anno", "annuale"]) {
        return Some(Grouping::Year);
    }
    if contains_any(msg, &["tipo", "tipologia", "estension", "categoria", "formato"]) {
        return Some(Grouping::Extension);
    }
    // Risposta secca a "1) tipo 2) data 3) progetto".
    match msg.trim() {
        "1" | "1)" | "uno" => Some(Grouping::Extension),
        "2" | "2)" | "due" => Some(Grouping::Month),
        "3" | "3)" | "tre" => Some(Grouping::SourceFolder),
        _ => None,
    }
}

/// Cartelle note per nome, piu' i percorsi assoluti scritti a mano.
fn resolve_folders(msg: &str) -> Vec<String> {
    let home = match storage::home_dir() {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };

    // (parole chiave, sottocartella della home)
    const ALIASES: &[(&[&str], &[&str])] = &[
        (&["download", "downloads", "scaricat"], &["Downloads", "Download"]),
        (&["desktop", "scrivania"], &["Desktop"]),
        (&["documenti", "documents"], &["Documents", "Documenti"]),
        (&["immagini", "foto", "pictures"], &["Pictures", "Immagini"]),
        (&["video", "filmati"], &["Videos", "Video"]),
        (&["musica", "music"], &["Music", "Musica"]),
    ];

    let mut out: Vec<String> = Vec::new();
    let wants_everything = contains_any(msg, &["tutto il pc", "tutto il computer", "tutte le cartelle"]);

    for (keys, candidates) in ALIASES {
        let mentioned = wants_everything || contains_any(msg, keys);
        if !mentioned {
            continue;
        }
        // "tutto il PC" resta comunque limitato alle cartelle utente: fuori da
        // li' ci sono cartelle di sistema che non vanno toccate.
        let is_user_folder = keys.iter().any(|k| ["download", "desktop", "documenti"].contains(k));
        if wants_everything && !is_user_folder {
            continue;
        }
        if let Some(dir) = candidates.iter().map(|c| home.join(c)).find(|p| p.is_dir()) {
            push_unique(&mut out, dir);
        }
    }

    for path in explicit_paths(msg) {
        push_unique(&mut out, path);
    }
    out
}

fn push_unique(out: &mut Vec<String>, path: PathBuf) {
    let s = path.to_string_lossy().to_string();
    if !out.iter().any(|p| p.eq_ignore_ascii_case(&s)) {
        out.push(s);
    }
}

/// Percorsi assoluti citati nel messaggio: tra virgolette, oppure token che
/// somigliano a un path ed esistono davvero sul disco.
fn explicit_paths(msg: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();

    // Prima le virgolette: sono l'unico modo per indicare path con spazi.
    let mut rest = msg;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        match after.find('"') {
            Some(end) => {
                let candidate = Path::new(&after[..end]);
                if candidate.is_dir() {
                    out.push(candidate.to_path_buf());
                }
                rest = &after[end + 1..];
            }
            None => break,
        }
    }

    for token in msg.split_whitespace() {
        let token = token.trim_matches(|c: char| c == ',' || c == '.' || c == ';' || c == '"');
        let looks_like_path = token.contains(":\\") || token.contains(":/") || token.starts_with('/');
        if looks_like_path && Path::new(token).is_dir() {
            out.push(PathBuf::from(token));
        }
    }
    out
}

/// Decide cosa fare col messaggio dell'utente. `has_files` e `has_proposal`
/// descrivono lo stato della conversazione corrente.
pub fn classify(message: &str, has_files: bool, has_proposal: bool) -> Intent {
    let msg = normalize(message);

    if is_undo(&msg) {
        return Intent::Undo;
    }
    if has_proposal && is_apply(&msg) {
        return Intent::Apply;
    }

    let folders = resolve_folders(&msg);
    // Una cartella nominata insieme a un criterio ("organizza Downloads per
    // tipo") va comunque scansionata prima: la proposta arriva al giro dopo.
    if !folders.is_empty() {
        return Intent::Scan(folders);
    }

    if has_files {
        if let Some(grouping) = detect_grouping(&msg) {
            return Intent::Propose(grouping);
        }
        // "organizza" senza criterio, ma i file ci sono gia': si riparte da li'.
        if contains_any(&msg, &["organizza", "sistema", "ordina", "riordina"]) {
            return Intent::Chat;
        }
    }

    Intent::Chat
}

/* ------------------------------ testi generati ----------------------------- */

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// Riepilogo della scansione + domanda sul criterio.
pub fn scan_summary_reply(folders: &[String], files: &[FileInfo]) -> String {
    if files.is_empty() {
        return format!(
            "Ho guardato in {} ma non ho trovato file da organizzare. Vuoi indicarmi un'altra cartella?",
            folders.join(", ")
        );
    }

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for f in files {
        let ext = if f.extension.is_empty() { "senza estensione".to_string() } else { format!(".{}", f.extension) };
        *counts.entry(ext).or_insert(0) += 1;
    }
    let mut top: Vec<(String, usize)> = counts.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    let top_line = top
        .iter()
        .take(5)
        .map(|(ext, n)| format!("{ext} ({n})"))
        .collect::<Vec<_>>()
        .join(", ");

    let names: Vec<String> = folders
        .iter()
        .map(|f| Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.clone()))
        .collect();
    let size_mb: u64 = files.iter().map(|f| f.size).sum::<u64>() / 1_048_576;

    format!(
        "Ho scansionato {}: {} per {} MB.\nTipi piu' frequenti: {}.\n\nCome preferisci organizzarli?\n1) Per tipo (PDF, immagini, video, ...)\n2) Per data (anno e mese)\n3) Per progetto (cartella di provenienza)",
        names.join(", "),
        plural(files.len(), "file trovato", "file trovati"),
        size_mb,
        top_line
    )
}

/// Frase che accompagna il blocco proposta.
pub fn proposal_reply(proposal: &OrganizationProposal, files: usize) -> String {
    let criterio = match proposal.grouping {
        Grouping::Extension => "per tipo di file",
        Grouping::Year => "per anno",
        Grouping::Month => "per anno e mese",
        Grouping::SourceFolder => "per cartella di provenienza",
    };
    let motore = match proposal.source.as_str() {
        "ollama" => " (schema suggerito dal modello locale)",
        "claude" => " (schema suggerito da Claude)",
        "fallback" => " (l'AI non era raggiungibile, ho usato lo schema di riserva)",
        _ => "",
    };
    format!(
        "Ecco lo schema {criterio}{motore}. Riguarda {}: controlla l'anteprima e dimmi se va bene.\n\nSe ti convince scrivi \"applica\" oppure premi Approva. Puoi anche rinominare o unire le cartelle prima di procedere.",
        plural(files, "file", "file")
    )
}

/// Risposta di ripiego quando il modello non risponde: l'app deve restare
/// utilizzabile anche senza AI.
pub fn offline_reply(has_files: bool, has_proposal: bool) -> String {
    if has_proposal {
        "Il modello AI non risponde, ma la proposta qui sopra e' pronta: scrivi \"applica\" per procedere o \"annulla\" per lasciar perdere.".to_string()
    } else if has_files {
        "Il modello AI non risponde. Posso comunque procedere: vuoi organizzare per tipo, per data o per progetto?".to_string()
    } else {
        "Il modello AI non risponde (controlla che Ollama sia avviato). Nel frattempo dimmi quale cartella organizzare, per esempio \"organizza Downloads\".".to_string()
    }
}

/* ------------------------------ chiamate al modello ----------------------------- */

fn history_messages(history: &[ChatTurn], message: &str) -> Vec<serde_json::Value> {
    let start = history.len().saturating_sub(MAX_HISTORY_TURNS);
    let mut out: Vec<serde_json::Value> = history[start..]
        .iter()
        .map(|t| {
            let role = if t.role == "user" { "user" } else { "assistant" };
            serde_json::json!({ "role": role, "content": t.content })
        })
        .collect();
    out.push(serde_json::json!({ "role": "user", "content": message }));
    out
}

async fn call_ollama_chat(
    url: &str,
    model: &str,
    system: &str,
    messages: &[serde_json::Value],
) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Message {
        content: String,
    }
    #[derive(Deserialize)]
    struct ChatResponse {
        message: Message,
    }

    let mut all = vec![serde_json::json!({ "role": "system", "content": system })];
    all.extend_from_slice(messages);

    let body = serde_json::json!({
        "model": model,
        "messages": all,
        "stream": false,
        "options": { "temperature": 0.6 }
    });

    let client = reqwest::Client::builder()
        .timeout(CHAT_TIMEOUT)
        .build()
        .map_err(|e| format!("Client HTTP non inizializzabile: {e}"))?;

    let res = client
        .post(format!("{}/api/chat", url.trim_end_matches('/')))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Ollama non raggiungibile: {e}"))?;

    if !res.status().is_success() {
        return Err(format!("Ollama ha risposto {}", res.status()));
    }
    let parsed: ChatResponse = res.json().await.map_err(|e| format!("Risposta Ollama illeggibile: {e}"))?;
    Ok(parsed.message.content)
}

async fn call_claude_chat(system: &str, messages: &[serde_json::Value]) -> Result<String, String> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| "ANTHROPIC_API_KEY non impostata: imposta la chiave o usa Ollama.".to_string())?;

    let body = serde_json::json!({
        "model": ai_analyzer::CLAUDE_MODEL,
        "max_tokens": 1500,
        "system": system,
        "messages": messages
    });

    let res = ai_analyzer::http_client()?
        .post(ai_analyzer::CLAUDE_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Claude API non raggiungibile: {e}"))?;

    let status = res.status();
    let value: serde_json::Value = res.json().await.map_err(|e| format!("Risposta Claude illeggibile: {e}"))?;
    if !status.is_success() {
        return Err(format!("Claude API ha risposto {status}: {value}"));
    }
    value["content"]
        .as_array()
        .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text").and_then(|b| b["text"].as_str()))
        .map(|s| s.to_string())
        .ok_or_else(|| "Claude non ha restituito testo.".to_string())
}

/// Estrae il blocco `<PROPOSAL_JSON>...</PROPOSAL_JSON>` e ripulisce il testo
/// mostrato all'utente, che non deve mai vedere il JSON grezzo.
fn split_proposal(text: &str) -> (String, Option<String>) {
    const OPEN: &str = "<PROPOSAL_JSON>";
    const CLOSE: &str = "</PROPOSAL_JSON>";

    let start = match text.find(OPEN) {
        Some(i) => i,
        None => return (text.trim().to_string(), None),
    };
    let after = &text[start + OPEN.len()..];
    let (json, tail) = match after.find(CLOSE) {
        Some(end) => (&after[..end], &after[end + CLOSE.len()..]),
        // Blocco troncato: prendiamo comunque quello che c'e'.
        None => (after, ""),
    };

    let visible = format!("{}{}", &text[..start], tail).trim().to_string();
    (visible, Some(json.trim().to_string()))
}

/// Contesto sintetico dello stato corrente, iniettato nella conversazione:
/// il modello non deve indovinare quanti file ci sono.
fn state_note(folders: &[String], file_count: usize, has_proposal: bool) -> String {
    let mut parts = Vec::new();
    if folders.is_empty() {
        parts.push("Nessuna cartella selezionata.".to_string());
    } else {
        parts.push(format!("Cartelle selezionate: {}.", folders.join(", ")));
        parts.push(format!("File scansionati: {file_count}."));
    }
    parts.push(if has_proposal {
        "C'e' gia' una proposta in attesa di approvazione.".to_string()
    } else {
        "Nessuna proposta pronta.".to_string()
    });
    format!("[stato attuale: {}]", parts.join(" "))
}

/// Turno conversazionale libero. Ritorna testo per l'utente e, se il modello
/// l'ha prodotta, una proposta gia' validata.
pub async fn chat_reply(
    message: &str,
    history: &[ChatTurn],
    folders: &[String],
    file_count: usize,
    has_proposal: bool,
    cfg: &AiConfig,
) -> (String, Option<OrganizationProposal>, Option<String>) {
    let system = if cfg.user_preferences.trim().is_empty() {
        SYSTEM_PROMPT.to_string()
    } else {
        format!("{SYSTEM_PROMPT}\n\nPreferenze permanenti dell'utente: {}", cfg.user_preferences.trim())
    };

    let annotated = format!("{}\n{message}", state_note(folders, file_count, has_proposal));
    let messages = history_messages(history, &annotated);

    let raw = if cfg.use_premium_ai {
        call_claude_chat(&system, &messages).await
    } else {
        call_ollama_chat(&cfg.ollama_url, &cfg.ollama_model, &system, &messages).await
    };

    let text = match raw {
        Ok(t) => t,
        Err(e) => return (offline_reply(file_count > 0, has_proposal), None, Some(e)),
    };

    let (visible, json) = split_proposal(&text);
    let source = if cfg.use_premium_ai { "claude" } else { "ollama" };
    // Una proposta che non passa la validazione non viene mostrata: meglio
    // niente che uno schema che sposta file dove non deve.
    let proposal = json
        .as_deref()
        .and_then(|j| ai_analyzer::extract_json(j).map(|s| s.to_string()))
        .and_then(|j| match ai_analyzer::proposal_from_json(&j, source) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("[logica] proposta AI scartata: {e}");
                None
            }
        });

    let visible = if visible.is_empty() {
        "Ho preparato uno schema, dimmi se ti convince.".to_string()
    } else {
        visible
    };
    (visible, proposal, None)
}

/// Costruisce la proposta per il criterio scelto. Per il criterio "per tipo"
/// passa dall'AI (con fallback deterministico); gli altri sono calcolati.
pub async fn build_proposal(files: &[FileInfo], grouping: Grouping, cfg: &AiConfig) -> OrganizationProposal {
    match grouping {
        Grouping::Extension => {
            let prefs = if cfg.user_preferences.trim().is_empty() {
                None
            } else {
                Some(cfg.user_preferences.trim())
            };
            ai_analyzer::get_organization_proposal(
                files,
                !cfg.use_premium_ai,
                &cfg.ollama_url,
                &cfg.ollama_model,
                prefs,
            )
            .await
        }
        other => ai_analyzer::grouping_proposal(files, other),
    }
}

/// Stato dei due motori AI, per l'indicatore in alto e le impostazioni.
pub async fn check_status(ollama_url: &str) -> AiStatus {
    let ollama = ai_analyzer::check_ollama_connection(ollama_url).await;
    let models = if ollama { ai_analyzer::list_ollama_models(ollama_url).await } else { Vec::new() };
    AiStatus {
        ollama,
        claude: std::env::var("ANTHROPIC_API_KEY").map(|k| !k.trim().is_empty()).unwrap_or(false),
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_wins_over_everything() {
        assert_eq!(classify("annulla tutto", true, true), Intent::Undo);
        assert_eq!(classify("torna indietro per favore", false, false), Intent::Undo);
    }

    #[test]
    fn apply_needs_a_pending_proposal() {
        assert_eq!(classify("applica", true, true), Intent::Apply);
        // Senza proposta "applica" e' solo una richiesta da chiarire.
        assert_eq!(classify("applica", true, false), Intent::Chat);
        assert_eq!(classify("vai", true, true), Intent::Apply);
        // In una frase lunga "ok" non e' un'approvazione.
        assert_eq!(
            classify("ok ma prima spiegami cosa succede ai file duplicati", true, true),
            Intent::Chat
        );
    }

    #[test]
    fn grouping_from_natural_language() {
        assert_eq!(detect_grouping("per tipo"), Some(Grouping::Extension));
        assert_eq!(detect_grouping("organizzali per data"), Some(Grouping::Month));
        assert_eq!(detect_grouping("solo per anno"), Some(Grouping::Year));
        assert_eq!(detect_grouping("dividi per progetto"), Some(Grouping::SourceFolder));
        assert_eq!(detect_grouping("2"), Some(Grouping::Month));
        assert_eq!(detect_grouping("boh, decidi tu"), None);
    }

    #[test]
    fn grouping_only_after_a_scan() {
        assert_eq!(classify("per tipo", true, false), Intent::Propose(Grouping::Extension));
        assert_eq!(classify("per tipo", false, false), Intent::Chat);
    }

    #[test]
    fn proposal_block_is_stripped_from_the_visible_text() {
        let text = "Ecco lo schema.\n<PROPOSAL_JSON>{\"rules\": []}</PROPOSAL_JSON>\nDimmi se va bene.";
        let (visible, json) = split_proposal(text);
        assert_eq!(visible, "Ecco lo schema.\n\nDimmi se va bene.");
        assert_eq!(json.as_deref(), Some("{\"rules\": []}"));
    }

    #[test]
    fn text_without_block_is_untouched() {
        let (visible, json) = split_proposal("  Ciao, quale cartella?  ");
        assert_eq!(visible, "Ciao, quale cartella?");
        assert!(json.is_none());
    }

    fn test_config() -> AiConfig {
        AiConfig {
            ollama_model: std::env::var("FOAI_TEST_MODEL").unwrap_or_else(|_| "mistral".to_string()),
            ..AiConfig::default()
        }
    }

    /// Turno di chat vero contro Ollama: verifica l'endpoint `/api/chat`, che i
    /// test offline non toccano. Richiede `ollama serve` e il modello scaricato:
    /// `cargo test -- --ignored live_ollama_chat`
    #[test]
    #[ignore]
    fn live_ollama_chat_replies_without_errors() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime tokio");
        let cfg = test_config();

        let status = runtime.block_on(check_status(&cfg.ollama_url));
        assert!(status.ollama, "Ollama non risponde su {}", cfg.ollama_url);
        assert!(!status.models.is_empty(), "nessun modello installato");

        let history = vec![ChatTurn { role: "ai".to_string(), content: "Ciao, sono Logica.".to_string() }];
        let (text, proposal, error) = runtime.block_on(chat_reply(
            "ciao, cosa sai fare esattamente?",
            &history,
            &[],
            0,
            false,
            &cfg,
        ));

        assert!(error.is_none(), "chiamata fallita: {error:?}");
        assert!(!text.trim().is_empty(), "risposta vuota");
        assert!(!text.contains("PROPOSAL_JSON"), "il blocco JSON non deve arrivare all'utente");
        // Senza file scansionati non ha senso proporre uno schema.
        assert!(proposal.is_none(), "proposta inattesa senza scansione");
    }
}
