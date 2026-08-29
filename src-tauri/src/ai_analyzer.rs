//! Costruzione del prompt, chiamata all'AI (Ollama locale o Claude API) e
//! validazione della proposta. Se qualcosa va storto si ricade sempre su uno
//! schema deterministico per categoria di estensione: l'app non si blocca mai.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Datelike, Utc};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::scanner::FileInfo;

pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
pub(crate) const CLAUDE_URL: &str = "https://api.anthropic.com/v1/messages";
pub(crate) const CLAUDE_MODEL: &str = "claude-opus-5";
/// Generoso di proposito: alla prima richiesta Ollama deve caricare il
/// modello in VRAM (per un 7B sono decine di secondi), e con 30 s si
/// finiva sempre nel fallback proprio alla prima analisi.
const AI_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_SAMPLES: usize = 100;
/// Profondita' massima consentita nello schema: evita alberi assurdi.
const MAX_SCHEMA_DEPTH: usize = 4;

/// Albero di cartelle annidate: { "Documenti": { "PDF": {} } }
#[derive(Debug, Clone, Default, Serialize)]
pub struct SchemaNode(pub BTreeMap<String, SchemaNode>);

/// Un elemento dentro una lista di cartelle: o il nome nudo, o un sottoalbero.
#[derive(Deserialize)]
#[serde(untagged)]
enum SchemaEntry {
    Name(String),
    Node(SchemaNode),
}

/// I modelli piccoli scrivono lo schema in forme diverse da quella chiesta:
/// `[]` o `null` per una foglia, `["PDF", "Word"]` o `[{"PDF": {}}]` per un
/// ramo, `"PDF"` per un figlio unico. Le accettiamo tutte invece di cadere sul
/// fallback per un dettaglio di sintassi.
impl<'de> Deserialize<'de> for SchemaNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SchemaNodeVisitor;

        impl<'de> Visitor<'de> for SchemaNodeVisitor {
            type Value = SchemaNode;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("un albero di cartelle: oggetto, lista di nomi, stringa o null")
            }

            fn visit_map<M>(self, mut access: M) -> Result<SchemaNode, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut out = BTreeMap::new();
                while let Some((key, value)) = access.next_entry::<String, SchemaNode>()? {
                    let key = key.trim().to_string();
                    if !key.is_empty() {
                        out.insert(key, value);
                    }
                }
                Ok(SchemaNode(out))
            }

            fn visit_seq<S>(self, mut access: S) -> Result<SchemaNode, S::Error>
            where
                S: SeqAccess<'de>,
            {
                let mut out = BTreeMap::new();
                while let Some(entry) = access.next_element::<SchemaEntry>()? {
                    match entry {
                        SchemaEntry::Name(name) => {
                            let name = name.trim().to_string();
                            if !name.is_empty() {
                                out.entry(name).or_default();
                            }
                        }
                        // Una lista di sottoalberi: si fondono in un unico livello.
                        SchemaEntry::Node(node) => out.extend(node.0),
                    }
                }
                Ok(SchemaNode(out))
            }

            fn visit_str<E>(self, value: &str) -> Result<SchemaNode, E>
            where
                E: de::Error,
            {
                let mut out = BTreeMap::new();
                let name = value.trim();
                if !name.is_empty() {
                    out.insert(name.to_string(), SchemaNode::default());
                }
                Ok(SchemaNode(out))
            }

            fn visit_unit<E>(self) -> Result<SchemaNode, E>
            where
                E: de::Error,
            {
                Ok(SchemaNode::default())
            }

            fn visit_none<E>(self) -> Result<SchemaNode, E>
            where
                E: de::Error,
            {
                Ok(SchemaNode::default())
            }

            fn visit_some<D>(self, deserializer: D) -> Result<SchemaNode, D::Error>
            where
                D: Deserializer<'de>,
            {
                deserializer.deserialize_any(SchemaNodeVisitor)
            }
        }

        deserializer.deserialize_any(SchemaNodeVisitor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub pattern: String,
    pub destination: String,
}

/// Criterio con cui si sceglie la cartella di destinazione.
///
/// `Extension` usa le `rules` (pattern sul nome file). Gli altri criteri sono
/// deterministici e ignorano le regole: la destinazione si ricava dai metadati
/// del file, cosa che i pattern sul nome non possono esprimere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grouping {
    /// Per tipo/estensione: decide `rules`.
    Extension,
    /// Per anno di modifica: "2024".
    Year,
    /// Per anno e mese: "2024/03 - Marzo".
    Month,
    /// Per cartella di provenienza: primo livello del path relativo.
    SourceFolder,
}

impl Default for Grouping {
    fn default() -> Self {
        Grouping::Extension
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationProposal {
    pub schema: SchemaNode,
    pub rules: Vec<Rule>,
    pub rationale: String,
    /// "ollama" | "claude" | "fallback"
    pub source: String,
    /// Criterio di raggruppamento. Le proposte AI non lo indicano: default per tipo.
    #[serde(default)]
    pub grouping: Grouping,
}

/// Forma grezza attesa dall'AI (senza `source`, che aggiungiamo noi).
#[derive(Debug, Deserialize)]
struct RawProposal {
    #[serde(default)]
    schema: SchemaNode,
    #[serde(default)]
    rules: Vec<Rule>,
    #[serde(default)]
    rationale: String,
}

/* ------------------------------ aggregazione ------------------------------ */

struct Aggregate {
    total: usize,
    total_size: u64,
    by_extension: Vec<(String, usize)>,
    oldest: String,
    newest: String,
    samples: Vec<String>,
}

fn aggregate(files: &[FileInfo]) -> Aggregate {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut total_size = 0u64;
    for f in files {
        let ext = if f.extension.is_empty() { "(nessuna)".to_string() } else { f.extension.clone() };
        *counts.entry(ext).or_insert(0) += 1;
        total_size += f.size;
    }
    let mut by_extension: Vec<(String, usize)> = counts.into_iter().collect();
    by_extension.sort_by(|a, b| b.1.cmp(&a.1));
    by_extension.truncate(25);

    let mut dates: Vec<&str> = files.iter().map(|f| f.modified.as_str()).collect();
    dates.sort_unstable();

    // Campione uniforme sull'intera lista, non solo i primi N file.
    let stride = (files.len() / MAX_SAMPLES).max(1);
    let samples = files
        .iter()
        .step_by(stride)
        .take(MAX_SAMPLES)
        .map(|f| {
            let preview = f
                .content_preview
                .as_deref()
                // take su chars: evita di tagliare a meta' un carattere UTF-8
                .map(|p| format!(" | {}", p.chars().take(80).collect::<String>()))
                .unwrap_or_default();
            format!("{} ({} KB){}", f.relative_path, f.size / 1024, preview)
        })
        .collect();

    Aggregate {
        total: files.len(),
        total_size,
        by_extension,
        oldest: dates.first().unwrap_or(&"-").to_string(),
        newest: dates.last().unwrap_or(&"-").to_string(),
        samples,
    }
}

fn build_prompt(files: &[FileInfo], user_preferences: Option<&str>) -> String {
    let agg = aggregate(files);
    let ext_lines = agg
        .by_extension
        .iter()
        .map(|(e, c)| format!("- .{e}: {c} file"))
        .collect::<Vec<_>>()
        .join("\n");

    let prefs = user_preferences
        .filter(|p| !p.trim().is_empty())
        .map(|p| format!("\nPreferenze dell'utente: {p}\n"))
        .unwrap_or_default();

    format!(
        "Sei un assistente che progetta strutture di cartelle. Analizza questa cartella e proponi \
un'organizzazione chiara, con nomi in italiano, massimo {MAX_SCHEMA_DEPTH} livelli di profondita'.

STATISTICHE
- File totali: {total}
- Dimensione totale: {size_mb} MB
- Modifiche dal {oldest} al {newest}

ESTENSIONI PIU' FREQUENTI
{ext_lines}

CAMPIONE DI FILE
{samples}
{prefs}
Rispondi SOLO con un oggetto JSON valido, senza testo attorno, in questa forma esatta:
{{
  \"schema\": {{ \"Documenti\": {{ \"PDF\": {{}} }}, \"Immagini\": {{}} }},
  \"rules\": [ {{ \"pattern\": \"*.pdf\", \"destination\": \"Documenti/PDF\" }} ],
  \"rationale\": \"Una o due frasi che spiegano la logica.\"
}}

Vincoli:
- \"schema\" e' un albero di cartelle annidate; le foglie sono oggetti vuoti {{}}.
- Ogni \"destination\" deve essere un percorso relativo presente in \"schema\", separato da /.
- I pattern ammessi sono: \"*.est\", \"prefisso*\", \"*suffisso\", \"*parte*\" o un nome esatto.
- Copri le estensioni piu' frequenti; i file non coperti finiranno in \"Vari\".
- Niente percorsi assoluti, niente \"..\", niente lettere di unita'.",
        total = agg.total,
        size_mb = agg.total_size / 1_048_576,
        oldest = agg.oldest,
        newest = agg.newest,
        ext_lines = ext_lines,
        samples = agg.samples.join("\n"),
        prefs = prefs,
    )
}

/* --------------------------------- chiamate -------------------------------- */

pub(crate) fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(AI_TIMEOUT)
        .build()
        .map_err(|e| format!("Client HTTP non inizializzabile: {e}"))
}

/// True se il demone Ollama risponde su localhost:11434.
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

async fn call_ollama(url: &str, model: &str, prompt: &str) -> Result<String, String> {
    #[derive(Deserialize)]
    struct OllamaResponse {
        response: String,
    }

    let body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": false,
        "format": "json",           // forza Ollama a produrre JSON puro
        "options": { "temperature": 0.2 }
    });

    let res = http_client()?
        .post(format!("{}/api/generate", url.trim_end_matches('/')))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Ollama non raggiungibile: {e}"))?;

    if !res.status().is_success() {
        return Err(format!("Ollama ha risposto {}", res.status()));
    }
    let parsed: OllamaResponse = res
        .json()
        .await
        .map_err(|e| format!("Risposta Ollama illeggibile: {e}"))?;
    Ok(parsed.response)
}

async fn call_claude(prompt: &str) -> Result<String, String> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| "ANTHROPIC_API_KEY non impostata: imposta la chiave o usa Ollama.".to_string())?;

    let body = serde_json::json!({
        "model": CLAUDE_MODEL,
        "max_tokens": 4000,
        "messages": [{ "role": "user", "content": prompt }]
    });

    let res = http_client()?
        .post(CLAUDE_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Claude API non raggiungibile: {e}"))?;

    let status = res.status();
    let value: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Risposta Claude illeggibile: {e}"))?;
    if !status.is_success() {
        return Err(format!("Claude API ha risposto {status}: {value}"));
    }

    // La risposta puo' contenere piu' blocchi (es. thinking): prendiamo il primo testo.
    value["content"]
        .as_array()
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|b| b["type"] == "text")
                .and_then(|b| b["text"].as_str())
        })
        .map(|s| s.to_string())
        .ok_or_else(|| "Claude non ha restituito testo.".to_string())
}

/* -------------------------------- parsing --------------------------------- */

/// Estrae il primo oggetto JSON bilanciato dal testo (i modelli spesso
/// aggiungono spiegazioni o code fence attorno al JSON).
pub(crate) fn extract_json(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in text[start..].char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Ripulisce un singolo nome di cartella da caratteri illegali su Windows.
fn sanitize_component(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\' | '/'))
        .collect();
    cleaned.trim().trim_matches('.').to_string()
}

/// Normalizza una destinazione: relativa, senza "..", max profondita'.
pub fn sanitize_destination(dest: &str) -> Option<String> {
    let parts: Vec<String> = dest
        .replace('\\', "/")
        .split('/')
        .map(sanitize_component)
        .filter(|p| !p.is_empty() && p != "." && p != "..")
        .take(MAX_SCHEMA_DEPTH)
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn insert_path(schema: &mut SchemaNode, dest: &str) {
    let mut node = schema;
    for part in dest.split('/') {
        node = node.0.entry(part.to_string()).or_default();
    }
}

fn prune_schema(node: &SchemaNode, depth: usize) -> SchemaNode {
    if depth >= MAX_SCHEMA_DEPTH {
        return SchemaNode::default();
    }
    let mut out = BTreeMap::new();
    for (name, child) in &node.0 {
        let clean = sanitize_component(name);
        if clean.is_empty() {
            continue;
        }
        out.insert(clean, prune_schema(child, depth + 1));
    }
    SchemaNode(out)
}

/// Valida e ripara la proposta grezza dell'AI.
fn validate(raw: RawProposal, source: &str) -> Result<OrganizationProposal, String> {
    let mut schema = prune_schema(&raw.schema, 0);

    let mut rules = Vec::new();
    for r in raw.rules {
        let pattern = r.pattern.trim().to_lowercase();
        if pattern.is_empty() {
            continue;
        }
        if let Some(dest) = sanitize_destination(&r.destination) {
            // Ogni destinazione deve esistere nell'albero mostrato all'utente.
            insert_path(&mut schema, &dest);
            rules.push(Rule { pattern, destination: dest });
        }
    }

    if rules.is_empty() {
        return Err("La proposta AI non contiene regole utilizzabili.".to_string());
    }

    // "Vari" e' la destinazione di riserva usata da file_mapper.
    insert_path(&mut schema, "Vari");

    let rationale = if raw.rationale.trim().is_empty() {
        "Nessuna spiegazione fornita dal modello.".to_string()
    } else {
        raw.rationale.trim().to_string()
    };

    Ok(OrganizationProposal {
        schema,
        rules,
        rationale,
        source: source.to_string(),
        grouping: Grouping::Extension,
    })
}

/* -------------------------------- fallback -------------------------------- */

/// Categoria di destinazione per estensione: usata dal fallback deterministico.
fn category_for(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "svg" | "heic" | "tiff" => "Immagini",
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "flv" => "Video",
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" => "Audio",
        "pdf" => "Documenti/PDF",
        "doc" | "docx" | "odt" | "rtf" | "txt" | "md" => "Documenti/Testi",
        "xls" | "xlsx" | "csv" | "ods" => "Documenti/Fogli",
        "ppt" | "pptx" | "odp" => "Documenti/Presentazioni",
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" => "Archivi",
        "exe" | "msi" | "dmg" | "appimage" | "deb" | "apk" => "Programmi",
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "java" | "c" | "cpp" | "h" | "go" | "rb"
        | "php" | "cs" | "sh" | "ps1" => "Codice",
        "json" | "xml" | "yaml" | "yml" | "toml" | "ini" | "cfg" => "Dati",
        "iso" | "img" | "vhd" => "Immagini disco",
        _ => "Vari",
    }
}

/// Schema semplice per estensione. Non bellissimo, ma sempre valido.
pub fn fallback_proposal(files: &[FileInfo]) -> OrganizationProposal {
    let mut schema = SchemaNode::default();
    let mut seen: BTreeMap<String, String> = BTreeMap::new();

    for f in files {
        if f.extension.is_empty() {
            continue;
        }
        let dest = category_for(&f.extension).to_string();
        seen.insert(f.extension.clone(), dest);
    }

    let rules: Vec<Rule> = seen
        .into_iter()
        .map(|(ext, dest)| {
            insert_path(&mut schema, &dest);
            Rule { pattern: format!("*.{ext}"), destination: dest }
        })
        .collect();

    insert_path(&mut schema, "Vari");

    OrganizationProposal {
        schema,
        rules,
        rationale: "Schema di riserva: i file sono raggruppati per tipo di estensione.".to_string(),
        source: "fallback".to_string(),
        grouping: Grouping::Extension,
    }
}

/* ------------------------------- entry point ------------------------------- */

/// Ottiene una proposta di organizzazione. Non fallisce mai: in caso di errore
/// AI ritorna lo schema di fallback.
pub async fn get_organization_proposal(
    files: &[FileInfo],
    use_ollama: bool,
    ollama_url: &str,
    model: &str,
    user_preferences: Option<&str>,
) -> OrganizationProposal {
    if files.is_empty() {
        return fallback_proposal(files);
    }

    let prompt = build_prompt(files, user_preferences);
    let source = if use_ollama { "ollama" } else { "claude" };

    let raw_text = if use_ollama {
        call_ollama(ollama_url, model, &prompt).await
    } else {
        call_claude(&prompt).await
    };

    let parsed = raw_text.and_then(|text| {
        let json = extract_json(&text).ok_or_else(|| "Nessun JSON nella risposta AI.".to_string())?;
        serde_json::from_str::<RawProposal>(json).map_err(|e| format!("JSON AI non valido: {e}"))
    });

    match parsed.and_then(|raw| validate(raw, source)) {
        Ok(proposal) => proposal,
        Err(err) => {
            eprintln!("[ai_analyzer] fallback attivato: {err}");
            fallback_proposal(files)
        }
    }
}

/// Parsa e valida una proposta scritta dall'AI dentro un blocco JSON.
pub fn proposal_from_json(json: &str, source: &str) -> Result<OrganizationProposal, String> {
    let raw: RawProposal =
        serde_json::from_str(json).map_err(|e| format!("JSON della proposta non valido: {e}"))?;
    validate(raw, source)
}

/* --------------------- proposte deterministiche non-AI --------------------- */

const MESI: [&str; 12] = [
    "Gennaio", "Febbraio", "Marzo", "Aprile", "Maggio", "Giugno", "Luglio", "Agosto",
    "Settembre", "Ottobre", "Novembre", "Dicembre",
];

/// Cartella di destinazione per un file, dato il criterio di raggruppamento.
/// Ritorna `None` per `Extension`, che passa invece dalle `rules`.
pub fn folder_for_grouping(file: &FileInfo, grouping: Grouping) -> Option<String> {
    match grouping {
        Grouping::Extension => None,
        Grouping::Year | Grouping::Month => {
            // Se la data non e' leggibile mettiamo il file tra i non datati,
            // invece di inventare un anno sbagliato.
            let dt = DateTime::parse_from_rfc3339(&file.modified)
                .map(|d| d.with_timezone(&Utc))
                .ok()?;
            let year = dt.year();
            if grouping == Grouping::Year {
                Some(year.to_string())
            } else {
                let m = dt.month() as usize;
                Some(format!("{year}/{:02} - {}", m, MESI[m - 1]))
            }
        }
        Grouping::SourceFolder => {
            // Primo livello del path relativo: i file gia' in radice non hanno
            // una cartella di provenienza.
            let first = file.relative_path.split('/').next().unwrap_or("");
            let is_file_in_root = first == file.name;
            if first.is_empty() || is_file_in_root {
                None
            } else {
                sanitize_destination(first)
            }
        }
    }
}

/// Proposta deterministica per data o per cartella di provenienza: lo schema
/// nasce dalle cartelle che i file produrranno davvero, non da un indovinello.
pub fn grouping_proposal(files: &[FileInfo], grouping: Grouping) -> OrganizationProposal {
    if grouping == Grouping::Extension {
        return fallback_proposal(files);
    }

    let mut schema = SchemaNode::default();
    for f in files {
        match folder_for_grouping(f, grouping) {
            Some(dest) => insert_path(&mut schema, &dest),
            None => insert_path(&mut schema, "Vari"),
        }
    }
    insert_path(&mut schema, "Vari");

    let rationale = match grouping {
        Grouping::Year => "I file sono raggruppati per anno di ultima modifica.",
        Grouping::Month => "I file sono raggruppati per anno e mese di ultima modifica.",
        Grouping::SourceFolder => "I file sono raggruppati per cartella di provenienza.",
        Grouping::Extension => unreachable!("gestito sopra"),
    };

    OrganizationProposal {
        schema,
        rules: Vec::new(), // il criterio non passa dai pattern sul nome
        rationale: rationale.to_string(),
        source: "logica".to_string(),
        grouping,
    }
}

/// Modelli installati su Ollama, per il menu delle impostazioni.
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
                .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_modified(name: &str, modified: &str, relative: &str) -> FileInfo {
        FileInfo {
            name: name.to_string(),
            path: format!("C:/tmp/{relative}"),
            extension: name.rsplit('.').next().unwrap_or("").to_string(),
            size: 10,
            modified: modified.to_string(),
            relative_path: relative.to_string(),
            depth: 1,
            content_preview: None,
        }
    }

    #[test]
    fn date_grouping_uses_italian_months() {
        let file = file_modified("nota.txt", "2024-03-07T08:00:00Z", "nota.txt");
        assert_eq!(folder_for_grouping(&file, Grouping::Month), Some("2024/03 - Marzo".to_string()));
        assert_eq!(folder_for_grouping(&file, Grouping::Year), Some("2024".to_string()));
        // Per tipo la destinazione arriva dalle regole, non dai metadati.
        assert_eq!(folder_for_grouping(&file, Grouping::Extension), None);
    }

    #[test]
    fn unreadable_date_and_root_files_fall_back_to_vari() {
        let broken = file_modified("nota.txt", "non e' una data", "nota.txt");
        assert_eq!(folder_for_grouping(&broken, Grouping::Month), None);

        // File in radice: nessuna cartella di provenienza.
        let root_file = file_modified("nota.txt", "2024-03-07T08:00:00Z", "nota.txt");
        assert_eq!(folder_for_grouping(&root_file, Grouping::SourceFolder), None);

        let nested = file_modified("nota.txt", "2024-03-07T08:00:00Z", "Progetto X/nota.txt");
        assert_eq!(folder_for_grouping(&nested, Grouping::SourceFolder), Some("Progetto X".to_string()));
    }

    #[test]
    fn grouping_proposal_lists_every_folder_it_will_create() {
        let files = vec![
            file_modified("a.txt", "2024-03-07T08:00:00Z", "a.txt"),
            file_modified("b.txt", "2025-11-02T08:00:00Z", "b.txt"),
        ];
        let proposal = grouping_proposal(&files, Grouping::Month);
        assert_eq!(proposal.grouping, Grouping::Month);
        assert!(proposal.rules.is_empty());
        let leaves = paths(&proposal.schema, "");
        assert!(leaves.contains(&"2024/03 - Marzo".to_string()), "manca marzo: {leaves:?}");
        assert!(leaves.contains(&"2025/11 - Novembre".to_string()), "manca novembre: {leaves:?}");
        assert!(leaves.contains(&"Vari".to_string()), "manca la cartella di riserva");
    }

    #[test]
    fn extracts_json_from_noisy_text() {
        let text = "Ecco il risultato:\n```json\n{\"a\": {\"b\": \"}\"}}\n```\nfine";
        assert_eq!(extract_json(text), Some("{\"a\": {\"b\": \"}\"}}"));
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        assert_eq!(sanitize_destination("../../etc"), Some("etc".to_string()));
        assert_eq!(sanitize_destination("C:/Windows"), Some("C/Windows".to_string()));
        assert_eq!(sanitize_destination("   "), None);
    }

    /// Elenco ordinato dei percorsi foglia-per-foglia, per confronti compatti.
    fn paths(node: &SchemaNode, prefix: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (name, child) in &node.0 {
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if child.0.is_empty() {
                out.push(path);
            } else {
                out.extend(paths(child, &path));
            }
        }
        out
    }

    fn parse(json: &str) -> SchemaNode {
        serde_json::from_str::<SchemaNode>(json).expect("schema rifiutato")
    }

    #[test]
    fn schema_accepts_nested_objects() {
        let node = parse(r#"{"Documenti": {"PDF": {}}, "Immagini": {}}"#);
        assert_eq!(paths(&node, ""), vec!["Documenti/PDF", "Immagini"]);
    }

    #[test]
    fn schema_accepts_lists_and_nulls_as_leaves() {
        // Forme che i modelli piccoli producono al posto di {}.
        let node = parse(r#"{"Documenti": ["PDF", "Word"], "Immagini": [], "Audio": null}"#);
        assert_eq!(
            paths(&node, ""),
            vec!["Audio", "Documenti/PDF", "Documenti/Word", "Immagini"]
        );
    }

    #[test]
    fn schema_accepts_list_of_subtrees_and_bare_strings() {
        let node = parse(r#"{"Media": [{"Audio": {}}, {"Video": ["MP4"]}], "Note": "Testi"}"#);
        assert_eq!(paths(&node, ""), vec!["Media/Audio", "Media/Video/MP4", "Note/Testi"]);
    }

    #[test]
    fn schema_drops_empty_names() {
        let node = parse(r#"{"  ": {}, "Documenti": ["", "  ", "PDF"]}"#);
        assert_eq!(paths(&node, ""), vec!["Documenti/PDF"]);
    }

    #[test]
    fn proposal_with_list_schema_is_not_rejected() {
        let raw = r#"{
            "schema": {"Documenti": ["PDF"], "Immagini": []},
            "rules": [{"pattern": "*.pdf", "destination": "Documenti/PDF"}],
            "rationale": "prova"
        }"#;
        let parsed: RawProposal = serde_json::from_str(raw).expect("proposta rifiutata");
        let proposal = validate(parsed, "ollama").expect("validazione fallita");
        assert_eq!(proposal.source, "ollama");
        assert_eq!(proposal.rules.len(), 1);
        assert!(paths(&proposal.schema, "").contains(&"Documenti/PDF".to_string()));
    }
}
