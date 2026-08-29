//! Operazioni reali sul disco: creazione cartelle, spostamento file, rollback
//! e undo. Nessuna funzione qui cancella file: si sposta soltanto.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::file_mapper::MappingPlan;

/// Nome del log di undo scritto nella cartella organizzata.
pub const UNDO_FILE: &str = ".file-organizer-undo.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResult {
    pub status: String,
    pub files_moved: usize,
    pub folders_created: usize,
    pub errors: Vec<String>,
    pub undo_available: bool,
}

/// Mosse effettivamente eseguite, in ordine: serve per rollback e undo.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UndoLog {
    pub root: String,
    /// (destinazione attuale, posizione originale)
    pub moved: Vec<(String, String)>,
    pub created_dirs: Vec<String>,
}

/// Sposta un file gestendo il caso "volumi diversi", dove `rename` fallisce.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Copia + rimozione dell'originale: unica strada tra dischi diversi.
            fs::copy(from, to)?;
            fs::remove_file(from)
        }
    }
}

/// Rimette i file al loro posto e cancella le cartelle create (solo se vuote).
fn rollback(log: &UndoLog) -> Vec<String> {
    let mut errors = Vec::new();
    for (dest, source) in log.moved.iter().rev() {
        if let Err(e) = move_file(Path::new(dest), Path::new(source)) {
            errors.push(format!("Rollback fallito per {dest}: {e}"));
        }
    }
    // Le cartelle piu' profonde prima: remove_dir tocca solo directory vuote.
    let mut dirs = log.created_dirs.clone();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.matches(std::path::MAIN_SEPARATOR).count()));
    for dir in dirs {
        let _ = fs::remove_dir(&dir); // se non e' vuota la lasciamo stare
    }
    errors
}

/// Applica il piano. In caso di errore o annullamento fa rollback completo:
/// o si sposta tutto, o non si sposta niente.
pub fn apply_mapping_plan<F>(
    plan: &MappingPlan,
    cancel: &AtomicBool,
    mut on_progress: F,
) -> Result<(ApplyResult, UndoLog), String>
where
    F: FnMut(usize, usize, &str),
{
    let root = Path::new(&plan.root);
    if !root.is_dir() {
        return Err(format!("La cartella '{}' non esiste piu'.", plan.root));
    }

    let mut log = UndoLog { root: plan.root.clone(), ..Default::default() };
    let total = plan.moves.len();

    // --- pre-flight: le destinazioni devono restare dentro la radice ---
    for m in &plan.moves {
        if !Path::new(&m.destination).starts_with(root) {
            return Err(format!("Destinazione fuori dalla cartella scelta: {}", m.relative_destination));
        }
        if !Path::new(&m.source).is_file() {
            return Err(format!("File sparito durante l'analisi: {}", m.source));
        }
    }

    // --- creazione cartelle ---
    for m in &plan.moves {
        let parent = match Path::new(&m.destination).parent() {
            Some(p) => p,
            None => continue,
        };
        if parent.exists() {
            continue;
        }
        // Registra ogni livello creato, cosi' il rollback puo' ripulirli tutti.
        let mut acc = PathBuf::from(root);
        for part in parent.strip_prefix(root).unwrap_or(parent).components() {
            acc.push(part);
            if !acc.exists() {
                log.created_dirs.push(acc.to_string_lossy().to_string());
            }
        }
        if let Err(e) = fs::create_dir_all(parent) {
            let errors = rollback(&log);
            return Err(format!("Impossibile creare {}: {e}. {}", parent.display(), errors.join("; ")));
        }
    }

    // --- spostamento file ---
    for (i, m) in plan.moves.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            rollback(&log);
            return Err("Operazione annullata: i file sono stati riportati al loro posto.".to_string());
        }

        if let Err(e) = move_file(Path::new(&m.source), Path::new(&m.destination)) {
            let errors = rollback(&log);
            let mut msg = format!("Errore su {}: {e}. Ripristinato lo stato iniziale.", m.source);
            if !errors.is_empty() {
                msg.push_str(&format!(" Attenzione: {}", errors.join("; ")));
            }
            return Err(msg);
        }
        log.moved.push((m.destination.clone(), m.source.clone()));

        if i % 20 == 0 || i + 1 == total {
            on_progress(i + 1, total, &m.relative_destination);
        }
    }

    // --- verifica finale ---
    let mut errors = Vec::new();
    for (dest, _) in &log.moved {
        if !Path::new(dest).exists() {
            errors.push(format!("File non trovato dopo lo spostamento: {dest}"));
        }
    }

    // Log di undo su disco: utile anche dopo un riavvio dell'app.
    let _ = fs::write(
        root.join(UNDO_FILE),
        serde_json::to_string_pretty(&log).unwrap_or_default(),
    );

    let result = ApplyResult {
        status: if errors.is_empty() { "success".into() } else { "partial".into() },
        files_moved: log.moved.len(),
        folders_created: log.created_dirs.len(),
        errors,
        undo_available: !log.moved.is_empty(),
    };
    Ok((result, log))
}

/// Ripete le mosse al contrario. Ritorna un messaggio per l'utente.
pub fn undo(log: &UndoLog) -> Result<String, String> {
    let errors = rollback(log);
    let _ = fs::remove_file(Path::new(&log.root).join(UNDO_FILE));
    if errors.is_empty() {
        Ok(format!("{} file riportati nella posizione originale.", log.moved.len()))
    } else {
        Err(format!("Undo completato con problemi: {}", errors.join("; ")))
    }
}

/// Apre la cartella nel file manager di sistema.
pub fn open_in_explorer(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("Percorso inesistente: {path}"));
    }

    #[cfg(target_os = "windows")]
    let cmd = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(p).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = std::process::Command::new("xdg-open").arg(p).spawn();

    // explorer.exe ritorna un exit code != 0 anche quando apre la finestra:
    // ci basta sapere che il processo e' partito.
    cmd.map(|_| ()).map_err(|e| format!("Impossibile aprire la cartella: {e}"))
}
