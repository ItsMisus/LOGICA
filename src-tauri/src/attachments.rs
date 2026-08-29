//! File allegati a un messaggio. Il frontend passa i percorsi scelti nel
//! dialogo di sistema; qui si legge il contenuto testuale da dare al modello.
//!
//! Regole: niente file oltre `MAX_BYTES`, niente binari (il modello non ci fa
//! nulla e riempirebbe il contesto), testo tagliato a `MAX_CHARS`.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Nessun file piu' grande di cosi' viene letto.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Caratteri passati al modello per ogni allegato.
const MAX_CHARS: usize = 12_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub path: String,
    pub size: u64,
    /// Contenuto testuale, gia' tagliato. `None` se il file non e' leggibile.
    pub text: Option<String>,
    /// Il contenuto e' stato tagliato a `MAX_CHARS`.
    pub truncated: bool,
    /// Perche' il file non e' stato letto, in italiano, pronto da mostrare.
    pub error: Option<String>,
}

impl Attachment {
    fn failed(path: &str, name: String, size: u64, reason: String) -> Self {
        Self { name, path: path.to_string(), size, text: None, truncated: false, error: Some(reason) }
    }
}

/// Legge gli allegati uno per uno: un file illeggibile non ferma gli altri,
/// torna con il suo errore accanto.
pub fn read_all(paths: &[String]) -> Vec<Attachment> {
    paths.iter().map(|p| read_one(p)).collect()
}

fn read_one(path: &str) -> Attachment {
    let file = Path::new(path);
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());

    let size = match fs::metadata(file) {
        Ok(meta) if meta.is_dir() => {
            return Attachment::failed(path, name, 0, "e' una cartella, non un file".to_string())
        }
        Ok(meta) => meta.len(),
        Err(e) => return Attachment::failed(path, name, 0, format!("non accessibile: {e}")),
    };

    if size > MAX_BYTES {
        let mb = MAX_BYTES / (1024 * 1024);
        return Attachment::failed(path, name, size, format!("troppo grande (oltre {mb} MB)"));
    }

    let bytes = match fs::read(file) {
        Ok(b) => b,
        Err(e) => return Attachment::failed(path, name, size, format!("non leggibile: {e}")),
    };

    // Un byte zero e' il segnale piu' affidabile che il file non e' testo.
    if bytes.contains(&0) {
        return Attachment::failed(path, name, size, "file binario: niente testo da leggere".to_string());
    }

    let content = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            return Attachment::failed(path, name, size, "codifica non riconosciuta (non e' UTF-8)".to_string())
        }
    };

    let truncated = content.chars().count() > MAX_CHARS;
    let text = if truncated { content.chars().take(MAX_CHARS).collect() } else { content };

    Attachment { name, path: path.to_string(), size, text: Some(text), truncated, error: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("logica_allegati_{tag}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("cartella temporanea");
        dir
    }

    #[test]
    fn reads_text_and_reports_binary() {
        let dir = temp_dir("misti");

        let testo = dir.join("note.txt");
        fs::write(&testo, "ciao mondo").expect("scrittura");

        let binario = dir.join("immagine.bin");
        let mut f = fs::File::create(&binario).expect("creazione");
        f.write_all(&[0x89, 0x50, 0x00, 0x01]).expect("scrittura");

        let letti = read_all(&[
            testo.to_string_lossy().to_string(),
            binario.to_string_lossy().to_string(),
            dir.join("assente.txt").to_string_lossy().to_string(),
        ]);

        assert_eq!(letti[0].name, "note.txt");
        assert_eq!(letti[0].text.as_deref(), Some("ciao mondo"));
        assert!(!letti[0].truncated);
        assert!(letti[0].error.is_none());

        assert!(letti[1].text.is_none());
        assert!(letti[1].error.as_deref().unwrap().contains("binario"));

        assert!(letti[2].error.is_some());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncates_long_files() {
        let dir = temp_dir("lungo");
        let lungo = dir.join("lungo.md");
        fs::write(&lungo, "a".repeat(MAX_CHARS + 500)).expect("scrittura");

        let letti = read_all(&[lungo.to_string_lossy().to_string()]);
        assert!(letti[0].truncated);
        assert_eq!(letti[0].text.as_ref().unwrap().chars().count(), MAX_CHARS);

        let _ = fs::remove_dir_all(&dir);
    }
}
