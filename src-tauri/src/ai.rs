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
    "llama3.1:8b".to_string()
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

/// I modelli gia' scaricati, per il menu a tendina delle impostazioni.
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
            models.iter().filter_map(|m| m["name"].as_str().map(|s| s.to_string())).collect()
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
