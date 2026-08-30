//! Gli strumenti che l'assistente puo' usare per guardare davvero sul disco.
//!
//! Fino a qui il modello poteva solo dire "non ho accesso ai tuoi file". Il
//! recinto per farlo davvero esisteva gia' (`server::guard`, scritto per
//! l'accesso da browser): qui lo si riusa tale e quale, cosi' la finestra
//! desktop e la rete hanno un solo confine e non due implementazioni che
//! divergono.
//!
//! Tutti gli strumenti sono in **sola lettura**: elencano, leggono, cercano.
//! Niente sposta, rinomina o cancella. Una scrittura decisa da un modello
//! locale da 7B senza una conferma davanti agli occhi dell'utente e' un
//! rischio che non vale la comodita'.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::server::guard;

/// Voci mostrate per una cartella. Oltre, il modello non legge comunque e il
/// contesto si riempie di nomi.
const MAX_ENTRIES: usize = 200;

/// Risultati riportati da una ricerca (per nome o per contenuto).
const MAX_RESULTS: usize = 40;

/// Quanto si scende sotto la cartella di partenza in una ricerca ricorsiva.
const MAX_DEPTH: usize = 6;

/// File aperti in tutto da una singola ricerca: un recinto grande con dentro
/// `node_modules` bloccherebbe la risposta per minuti.
const MAX_VISITED: usize = 4_000;

/// Byte letti da un file per cercarci dentro. Oltre e' quasi certamente un
/// binario o un log enorme, e in entrambi i casi non serve.
const MAX_SCAN_BYTES: u64 = 2 * 1024 * 1024;

/// Cartelle che in una ricerca sono solo rumore e costano moltissimo tempo.
const SKIP_DIRS: &[&str] =
    &["node_modules", ".git", "target", "dist", "build", ".venv", "__pycache__", ".next", ".cache"];

/// Passi di strumento in un solo turno. Serve un tetto: un modello confuso
/// rifarebbe la stessa `list_dir` all'infinito.
pub const MAX_STEPS: usize = 6;

/// Una chiamata a uno strumento, come esce dal modello.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// Id assegnato da Claude (`toolu_...`). Ollama non ne manda: resta vuoto.
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

impl ToolCall {
    fn arg(&self, key: &str) -> Option<String> {
        self.input[key].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }
}

/* ------------------------------ dichiarazioni ----------------------------- */

/// Le definizioni degli strumenti nel formato di Claude. Ollama usa lo stesso
/// JSON Schema avvolto in `{"type":"function","function":{...}}`, quindi si
/// scrive una volta sola e si converte.
fn definitions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "list_dir",
            "description": "Elenca il contenuto di una cartella: nomi, se sono file o cartelle, dimensione e data di modifica. Usalo per esplorare prima di leggere.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Percorso assoluto della cartella." }
                },
                "required": ["path"]
            }
        }),
        serde_json::json!({
            "name": "read_file",
            "description": "Legge il contenuto testuale di un file. Funziona su testo, sorgenti, PDF e DOCX. Il testo lungo viene tagliato.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Percorso assoluto del file." }
                },
                "required": ["path"]
            }
        }),
        serde_json::json!({
            "name": "find_files",
            "description": "Cerca file e cartelle il cui nome contiene un testo, scendendo nelle sottocartelle.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Pezzo di nome da cercare, senza asterischi." },
                    "path": { "type": "string", "description": "Cartella da cui partire. Se manca, tutte le cartelle autorizzate." }
                },
                "required": ["name"]
            }
        }),
        serde_json::json!({
            "name": "search_in_files",
            "description": "Cerca un testo dentro i file e riporta il file e la riga in cui compare.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Testo da cercare dentro i file." },
                    "path": { "type": "string", "description": "Cartella da cui partire. Se manca, tutte le cartelle autorizzate." }
                },
                "required": ["text"]
            }
        }),
    ]
}

/// Formato Claude: le definizioni cosi' come sono.
pub fn claude_tools() -> Vec<serde_json::Value> {
    definitions()
}

/// Formato Ollama: `input_schema` si chiama `parameters` e sta dentro
/// `function`.
pub fn ollama_tools() -> Vec<serde_json::Value> {
    definitions()
        .into_iter()
        .map(|d| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": d["name"],
                    "description": d["description"],
                    "parameters": d["input_schema"],
                }
            })
        })
        .collect()
}

/// Gli strumenti hanno senso solo se c'e' almeno una cartella autorizzata:
/// senza, ogni chiamata tornerebbe lo stesso rifiuto e il modello ci
/// sbatterebbe contro per sei giri.
pub fn available() -> bool {
    !guard::roots().is_empty()
}

/// Il pezzo di prompt che dice al modello dove puo' guardare. Senza l'elenco,
/// il modello inventa percorsi plausibili e si prende un rifiuto dopo l'altro.
pub fn roots_block() -> String {
    let list: Vec<String> = guard::roots().iter().map(|r| format!("- {}", display_path(r))).collect();
    format!(
        "STRUMENTI SUI FILE\nHai accesso in sola lettura a queste cartelle e a tutto quello che contengono:\n{}\n\nQuando la domanda riguarda file, cartelle o documenti dell'utente, guarda davvero con gli strumenti invece di dire che non puoi. Parti da list_dir su una di queste cartelle. Non inventare percorsi: quelli fuori da questo elenco vengono rifiutati. Non puoi scrivere, spostare o cancellare niente: se te lo chiedono, dillo.",
        list.join("\n")
    )
}

/* -------------------------------- esecuzione ------------------------------ */

/// Riga mostrata in chat mentre lo strumento gira. Il modello ci mette
/// qualche secondo per ogni passo: senza, sembra fermo.
pub fn status_line(call: &ToolCall) -> String {
    match call.name.as_str() {
        "list_dir" => format!("Guardo in {}", call.arg("path").unwrap_or_default()),
        "read_file" => format!("Leggo {}", call.arg("path").unwrap_or_default()),
        "find_files" => format!("Cerco file: {}", call.arg("name").unwrap_or_default()),
        "search_in_files" => format!("Cerco nei file: {}", call.arg("text").unwrap_or_default()),
        other => format!("Uso {other}"),
    }
}

/// Esegue una chiamata e torna il testo da rimandare al modello.
///
/// Gli errori non sono `Err`: tornano come testo, perche' "quella cartella non
/// esiste" e' un'informazione con cui il modello puo' correggersi al passo
/// dopo, mentre un errore duro interromperebbe la risposta.
pub fn run(call: &ToolCall) -> String {
    match call.name.as_str() {
        "list_dir" => match call.arg("path") {
            Some(path) => list_dir(&path),
            None => "Errore: manca il parametro 'path'.".to_string(),
        },
        "read_file" => match call.arg("path") {
            Some(path) => read_file(&path),
            None => "Errore: manca il parametro 'path'.".to_string(),
        },
        "find_files" => match call.arg("name") {
            Some(name) => find_files(&name, call.arg("path")),
            None => "Errore: manca il parametro 'name'.".to_string(),
        },
        "search_in_files" => match call.arg("text") {
            Some(text) => search_in_files(&text, call.arg("path")),
            None => "Errore: manca il parametro 'text'.".to_string(),
        },
        other => format!("Errore: lo strumento '{other}' non esiste."),
    }
}

fn list_dir(path: &str) -> String {
    let dir = match guard::ensure_within(path) {
        Ok(d) => d,
        Err(e) => return format!("Errore: {e}"),
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => return format!("Errore: cartella non leggibile ({e})."),
    };

    let mut rows: Vec<(bool, String)> = Vec::new();
    let mut total = 0usize;
    for entry in entries.flatten() {
        total += 1;
        if rows.len() >= MAX_ENTRIES {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = entry.metadata().ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let row = if is_dir {
            format!("{name}/")
        } else {
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let when = meta.as_ref().and_then(modified_on).unwrap_or_default();
            format!("{name}  {}  {when}", human_size(size))
        };
        rows.push((is_dir, row));
    }

    if rows.is_empty() {
        return format!("{}: cartella vuota.", display_path(&dir));
    }
    // Cartelle prima: e' l'ordine in cui si esplora.
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));

    let mut out = format!("{} ({total} voci)\n", display_path(&dir));
    for (_, row) in &rows {
        out.push_str(row);
        out.push('\n');
    }
    if total > rows.len() {
        out.push_str(&format!("... altre {} voci non elencate.\n", total - rows.len()));
    }
    out
}

fn read_file(path: &str) -> String {
    let file = match guard::ensure_within(path) {
        Ok(f) => f,
        Err(e) => return format!("Errore: {e}"),
    };
    if file.is_dir() {
        return format!("Errore: {} e' una cartella. Usa list_dir.", display_path(&file));
    }
    // Stessa lettura degli allegati: PDF, DOCX e testo non UTF-8 sono gia'
    // risolti li' una volta sola.
    let read = crate::attachments::read_all(&[file.to_string_lossy().to_string()]);
    let Some(attachment) = read.into_iter().next() else {
        return "Errore: lettura fallita.".to_string();
    };
    if let Some(err) = attachment.error {
        return format!("Errore: {err}");
    }
    if attachment.image.is_some() {
        return format!(
            "{} e' un'immagine: gli strumenti non la possono guardare. L'utente puo' allegarla al messaggio.",
            attachment.name
        );
    }
    match attachment.text {
        Some(text) if !text.trim().is_empty() => {
            let cut =
                if attachment.truncated { "\n[...] file tagliato: continua oltre." } else { "" };
            format!("{} ({})\n\n{text}{cut}", display_path(&file), human_size(attachment.size))
        }
        _ => format!("{}: nessun testo leggibile.", display_path(&file)),
    }
}

fn find_files(needle: &str, path: Option<String>) -> String {
    let bases = match starting_points(path) {
        Ok(b) => b,
        Err(e) => return format!("Errore: {e}"),
    };
    let needle = needle.to_lowercase();
    let mut found: Vec<String> = Vec::new();
    let mut visited = 0usize;

    for base in &bases {
        walk(base, 0, &mut visited, &mut |entry: &Path, is_dir: bool| {
            if found.len() >= MAX_RESULTS {
                return;
            }
            let name =
                entry.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            if name.contains(&needle) {
                let suffix = if is_dir { "/" } else { "" };
                found.push(format!("{}{suffix}", display_path(entry)));
            }
        });
    }

    if found.is_empty() {
        return format!("Nessun file o cartella con \"{needle}\" nel nome.");
    }
    let mut out = format!("{} risultati per \"{needle}\":\n", found.len());
    for row in &found {
        out.push_str(row);
        out.push('\n');
    }
    if found.len() >= MAX_RESULTS {
        out.push_str("(elenco tagliato: restringi la ricerca)\n");
    }
    out
}

fn search_in_files(needle: &str, path: Option<String>) -> String {
    let bases = match starting_points(path) {
        Ok(b) => b,
        Err(e) => return format!("Errore: {e}"),
    };
    let lower = needle.to_lowercase();
    let mut hits: Vec<String> = Vec::new();
    let mut visited = 0usize;

    for base in &bases {
        walk(base, 0, &mut visited, &mut |entry: &Path, is_dir: bool| {
            if is_dir || hits.len() >= MAX_RESULTS {
                return;
            }
            let Ok(meta) = entry.metadata() else { return };
            if meta.len() > MAX_SCAN_BYTES {
                return;
            }
            let Ok(content) = std::fs::read(entry) else { return };
            // I binari non si cercano: un PNG pieno di byte casuali produce
            // "righe" lunghe migliaia di caratteri.
            if content.iter().take(1024).any(|b| *b == 0) {
                return;
            }
            let text = String::from_utf8_lossy(&content);
            for (n, line) in text.lines().enumerate() {
                if hits.len() >= MAX_RESULTS {
                    return;
                }
                if line.to_lowercase().contains(&lower) {
                    hits.push(format!(
                        "{}:{}: {}",
                        display_path(entry),
                        n + 1,
                        line.trim().chars().take(200).collect::<String>()
                    ));
                }
            }
        });
    }

    if hits.is_empty() {
        return format!("\"{needle}\" non compare in nessun file letto.");
    }
    let mut out = format!("{} righe con \"{needle}\":\n", hits.len());
    for row in &hits {
        out.push_str(row);
        out.push('\n');
    }
    if hits.len() >= MAX_RESULTS {
        out.push_str("(elenco tagliato: restringi la ricerca)\n");
    }
    out
}

/* -------------------------------- utilita' -------------------------------- */

/// Da dove parte una ricerca: la cartella chiesta, verificata, oppure tutte le
/// radici autorizzate quando non e' stata indicata.
fn starting_points(path: Option<String>) -> Result<Vec<PathBuf>, String> {
    match path {
        Some(p) => guard::ensure_within(&p).map(|p| vec![p]),
        None => {
            let roots = guard::roots();
            if roots.is_empty() {
                Err("nessuna cartella autorizzata.".to_string())
            } else {
                Ok(roots)
            }
        }
    }
}

/// Visita ricorsiva dei soli file, per chi deve leggerli tutti: l'indice dei
/// documenti. Stessi salti di cartella, tetti molto piu' alti — qui non si sta
/// rispondendo a una domanda, si sta preparando un indice, e un file saltato
/// vuol dire un documento che poi non si trovera' mai.
pub fn walk_files(dir: &Path, visited: &mut usize, on_file: &mut impl FnMut(&Path)) {
    walk_with(dir, 0, MAX_INDEX_DEPTH, MAX_INDEX_VISITS, visited, &mut |path, is_dir| {
        if !is_dir {
            on_file(path);
        }
    });
}

/// Profondita' e file visitati durante l'indicizzazione.
const MAX_INDEX_DEPTH: usize = 12;
const MAX_INDEX_VISITS: usize = 50_000;

/// Visita ricorsiva con i freni: profondita', numero di file visitati e le
/// cartelle che non servono mai.
fn walk(dir: &Path, depth: usize, visited: &mut usize, on_entry: &mut impl FnMut(&Path, bool)) {
    walk_with(dir, depth, MAX_DEPTH, MAX_VISITED, visited, on_entry);
}

fn walk_with(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    max_visited: usize,
    visited: &mut usize,
    on_entry: &mut impl FnMut(&Path, bool),
) {
    if depth > max_depth || *visited >= max_visited {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if *visited >= max_visited {
            return;
        }
        *visited += 1;
        let path = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let name = entry.file_name().to_string_lossy().to_string();
        if is_dir && SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        on_entry(&path, is_dir);
        if is_dir {
            walk_with(&path, depth + 1, max_depth, max_visited, visited, on_entry);
        }
    }
}

/// Il prefisso `\\?\` che Windows mette nei percorsi canonici e' corretto ma
/// illeggibile, e il modello poi lo ricopia nelle risposte.
fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn modified_on(meta: &std::fs::Metadata) -> Option<String> {
    let modified = meta.modified().ok()?;
    let stamp: chrono::DateTime<chrono::Local> = modified.into();
    Some(stamp.format("%d/%m/%Y").to_string())
}

/* -------------------------------- risposta -------------------------------- */

/// Quello che il frontend mostra sotto la risposta: cosa ha guardato l'app.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
pub struct ToolUse {
    pub name: String,
    /// Il parametro principale, gia' pronto da mostrare (un percorso, un testo
    /// cercato).
    pub detail: String,
}

impl ToolUse {
    pub fn of(call: &ToolCall) -> Self {
        let detail = call
            .arg("path")
            .or_else(|| call.arg("name"))
            .or_else(|| call.arg("text"))
            .unwrap_or_default();
        Self { name: call.name.clone(), detail }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_tools_wrap_the_same_schema() {
        let claude = claude_tools();
        let ollama = ollama_tools();
        assert_eq!(claude.len(), ollama.len());
        assert_eq!(ollama[0]["type"], "function");
        assert_eq!(ollama[0]["function"]["name"], claude[0]["name"]);
        assert_eq!(ollama[0]["function"]["parameters"], claude[0]["input_schema"]);
    }

    #[test]
    fn unknown_tool_is_an_answer_not_a_crash() {
        let call = ToolCall {
            id: String::new(),
            name: "delete_everything".to_string(),
            input: serde_json::json!({}),
        };
        assert!(run(&call).starts_with("Errore:"));
    }

    #[test]
    fn missing_argument_is_reported_to_the_model() {
        let call = ToolCall {
            id: String::new(),
            name: "list_dir".to_string(),
            input: serde_json::json!({}),
        };
        assert!(run(&call).contains("path"));
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn windows_verbatim_prefix_is_hidden() {
        assert_eq!(display_path(Path::new(r"\\?\C:\Users\a")), r"C:\Users\a");
        assert_eq!(display_path(Path::new("/home/a")), "/home/a");
    }

    #[test]
    fn tool_use_picks_the_main_argument() {
        let call = ToolCall {
            id: "t1".to_string(),
            name: "search_in_files".to_string(),
            input: serde_json::json!({ "text": "fattura" }),
        };
        assert_eq!(
            ToolUse::of(&call),
            ToolUse { name: "search_in_files".into(), detail: "fattura".into() }
        );
    }
}
