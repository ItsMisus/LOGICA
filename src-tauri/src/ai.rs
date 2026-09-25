//! Il motore AI visto da fuori: come e' configurato e se e' raggiungibile.
//!
//! La conversazione vera sta in `assistant`; qui restano i tipi che attraversano
//! la IPC (`AiConfig`, `ChatTurn`) e l'indicatore di stato delle impostazioni.

use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
pub const CLAUDE_URL: &str = "https://api.anthropic.com/v1/messages";
pub const CLAUDE_MODEL: &str = "claude-opus-5";

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
    /// L'assistente puo' cercare su internet quando serve. I link scritti
    /// dall'utente vengono aperti comunque: quelli sono una richiesta esplicita.
    #[serde(default = "default_web_search")]
    pub web_search: bool,
    /// L'assistente puo' elencare, leggere e cercare dentro le cartelle
    /// autorizzate. Spento di default: chi lo accende sa che sta dando a un
    /// modello la possibilita' di leggere i propri file.
    #[serde(default)]
    pub file_tools: bool,
    /// L'assistente si porta dietro fra una chat e l'altra i fatti che ha
    /// imparato sull'utente.
    #[serde(default = "default_memory")]
    pub memory: bool,
    /// L'assistente cerca negli estratti dei documenti indicizzati prima di
    /// rispondere. Serve un indice gia' costruito.
    #[serde(default)]
    pub documents: bool,
}

fn default_memory() -> bool {
    true
}

fn default_web_search() -> bool {
    true
}

fn default_url() -> String {
    DEFAULT_OLLAMA_URL.to_string()
}

fn default_model() -> String {
    "qwen3:14b".to_string()
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            use_premium_ai: false,
            ollama_url: default_url(),
            ollama_model: default_model(),
            user_preferences: String::new(),
            web_search: default_web_search(),
            file_tools: false,
            memory: default_memory(),
            documents: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiStatus {
    pub ollama: bool,
    pub claude: bool,
    pub models: Vec<String>,
}

/// Ollama risponde? Timeout corto: e' un controllo di cortesia, non deve
/// tenere ferma la finestra se il servizio non c'e'.
pub async fn check_ollama_connection(url: &str) -> bool {
    let client = match reqwest::Client::builder().timeout(Duration::from_secs(3)).build() {
        Ok(c) => c,
        Err(_) => return false,
    };
    client
        .get(format!("{}/api/tags", url.trim_end_matches('/')))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Un modello che serve a misurare somiglianze, non a rispondere.
///
/// Ollama tiene in un elenco solo i modelli di conversazione e quelli di
/// embedding, ma nel menu delle impostazioni si sceglie chi risponde: offrire
/// `bge-m3` fra le scelte vuol dire offrire un assistente che non parla, e chi
/// lo prova non ha modo di capire perche' non funziona.
///
/// Due controlli invece di uno, perche' da soli sbagliano tutti e due. La
/// famiglia riconosce `paraphrase-multilingual`, che nel nome non dice niente;
/// il nome riconoscerebbe un modello di embedding costruito su un'architettura
/// che non e' BERT, che oggi non c'e' ma domani si'.
fn is_embedding_model(name: &str, family: &str) -> bool {
    family.contains("bert") || name.to_lowercase().contains("embed")
}

/// I modelli gia' scaricati che sanno rispondere, per il menu a tendina delle
/// impostazioni.
pub async fn list_ollama_models(url: &str) -> Vec<String> {
    let client = match reqwest::Client::builder().timeout(Duration::from_secs(5)).build() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let res = match client.get(format!("{}/api/tags", url.trim_end_matches('/'))).send().await {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let value: serde_json::Value = match res.json().await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    value["models"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter_map(|m| {
                    let name = m["name"].as_str()?;
                    let family = m["details"]["family"].as_str().unwrap_or("");
                    (!is_embedding_model(name, family)).then(|| name.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Stato dei due motori AI, per l'indicatore in alto e le impostazioni.
pub async fn check_status(ollama_url: &str) -> AiStatus {
    let ollama = check_ollama_connection(ollama_url).await;
    let models = if ollama { list_ollama_models(ollama_url).await } else { Vec::new() };
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
    fn the_models_that_only_measure_similarity_are_not_offered() {
        // I quattro che erano davvero installati quando il difetto e' venuto
        // fuori: l'elenco delle impostazioni li offriva tutti.
        assert!(is_embedding_model("bge-m3:latest", "bert"));
        assert!(is_embedding_model("granite-embedding:278m", "bert"));
        assert!(is_embedding_model("nomic-embed-text:latest", "nomic-bert"));
        assert!(is_embedding_model("paraphrase-multilingual:latest", "bert"));
    }

    #[test]
    fn the_models_that_answer_stay() {
        assert!(!is_embedding_model("llama3.1:8b", "llama"));
        assert!(!is_embedding_model("qwen3:14b", "qwen3"));
        assert!(!is_embedding_model("qwen3:30b-a3b", "qwen3moe"));
        assert!(!is_embedding_model("mistral:latest", "llama"));
    }

    /// I due controlli servono tutti e due, e questo lo dimostra: ognuno prende
    /// un caso che l'altro si lascia scappare.
    #[test]
    fn each_of_the_two_checks_catches_what_the_other_misses() {
        // Nel nome non c'e' scritto niente: lo riconosce solo la famiglia.
        assert!(is_embedding_model("paraphrase-multilingual:latest", "bert"));
        // La famiglia non e' BERT: lo riconosce solo il nome.
        assert!(is_embedding_model("qualcosa-embed:latest", "llama"));
    }
}
