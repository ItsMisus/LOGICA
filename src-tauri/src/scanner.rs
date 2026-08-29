//! Scansione ricorsiva del file system.
//!
//! Il walk e' bloccante (walkdir) quindi viene lanciato da `main.rs` dentro
//! `spawn_blocking`: la UI resta reattiva anche su 100k+ file.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use walkdir::{DirEntry, WalkDir};

/// Cartelle saltate sempre: rumore o roba di sistema che non va toccata.
pub const DEFAULT_IGNORES: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    "__pycache__",
    "node_modules",
    ".DS_Store",
    "$RECYCLE.BIN",
    "System Volume Information",
    "Windows",
    "Program Files",
    "Program Files (x86)",
    ".Trash",
    ".cache",
    "target",
    ".venv",
    "venv",
];

/// Estensioni per cui ha senso leggere un'anteprima del contenuto.
const PREVIEWABLE: &[&str] = &["pdf", "docx", "txt", "json", "md", "csv"];

/// Soglia oltre la quale leggiamo l'anteprima (da specifica: 1 MB).
const PREVIEW_MIN_SIZE: u64 = 1024 * 1024;
const PREVIEW_BYTES: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    pub extension: String,
    pub size: u64,
    /// ISO-8601 UTC, es. "2024-08-15T10:30:00Z"
    pub modified: String,
    pub relative_path: String,
    pub depth: usize,
    pub content_preview: Option<String>,
}

/// True se la entry va ignorata (nome nella blacklist oppure nascosta).
fn is_ignored(entry: &DirEntry, ignores: &[String], root: &Path) -> bool {
    // La radice scelta dall'utente non viene mai filtrata, anche se nascosta.
    if entry.path() == root {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    if name.starts_with('.') && name.len() > 1 {
        return true;
    }
    ignores.iter().any(|i| i.eq_ignore_ascii_case(name.as_ref()))
}

/// Legge i primi byte di un file e li ripulisce dai caratteri non stampabili,
/// cosi' l'anteprima resta leggibile anche per PDF/DOCX (che sono binari).
fn read_preview(path: &Path) -> Option<String> {
    let mut buf = [0u8; PREVIEW_BYTES];
    let mut file = File::open(path).ok()?;
    let read = file.read(&mut buf).ok()?;
    let text: String = String::from_utf8_lossy(&buf[..read])
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| c.is_ascii() || c.is_alphanumeric())
        .collect();
    let trimmed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Scansiona `root` ricorsivamente. `on_progress(scansionati, path_corrente)`
/// viene chiamato periodicamente per aggiornare la UI.
pub fn scan_directory<F>(
    root: &str,
    ignore_patterns: Vec<String>,
    mut on_progress: F,
) -> Result<Vec<FileInfo>, String>
where
    F: FnMut(usize, &str),
{
    let root_path = Path::new(root);
    if !root_path.is_dir() {
        return Err(format!("'{root}' non e' una cartella valida."));
    }

    let mut ignores: Vec<String> = DEFAULT_IGNORES.iter().map(|s| s.to_string()).collect();
    ignores.extend(ignore_patterns);

    let mut files = Vec::new();
    let walker = WalkDir::new(root_path)
        .follow_links(false) // evita loop infiniti su symlink/junction
        .into_iter()
        .filter_entry(|e| !is_ignored(e, &ignores, root_path));

    for entry in walker {
        // Una cartella illeggibile (permessi) non deve far fallire tutta la scansione.
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let size = metadata.len();
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let modified = metadata
            .modified()
            .map(|t| DateTime::<Utc>::from(t).to_rfc3339())
            .unwrap_or_else(|_| Utc::now().to_rfc3339());

        let relative_path = path
            .strip_prefix(root_path)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        let content_preview = if size > PREVIEW_MIN_SIZE && PREVIEWABLE.contains(&extension.as_str()) {
            read_preview(path)
        } else {
            None
        };

        files.push(FileInfo {
            name: entry.file_name().to_string_lossy().to_string(),
            path: path.to_string_lossy().to_string(),
            extension,
            size,
            modified,
            relative_path,
            depth: entry.depth(),
            content_preview,
        });

        // Un evento ogni 250 file: aggiorna la UI senza saturare il canale IPC.
        if files.len() % 250 == 0 {
            on_progress(files.len(), &path.to_string_lossy());
        }
    }

    on_progress(files.len(), "");
    Ok(files)
}
