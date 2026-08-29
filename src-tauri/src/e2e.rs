//! Test end-to-end del ciclo completo: scansione -> proposta -> piano ->
//! applicazione -> undo, su una cartella temporanea usa-e-getta.
//!
//! Usa lo schema di fallback (deterministico, nessuna rete), cosi' il test
//! resta verde anche senza Ollama. Il test con l'AI vera e' `#[ignore]`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{ai_analyzer, file_mapper, file_operations, scanner};

/// File finti che coprono tutte le categorie dello schema di fallback.
const SAMPLE_FILES: &[&str] = &[
    "fattura_2024.pdf",
    "relazione.docx",
    "appunti.txt",
    "budget.xlsx",
    "foto_vacanza.jpg",
    "screenshot.png",
    "logo.svg",
    "script.py",
    "app.js",
    "main.rs",
    "backup.zip",
    "archivio.tar.gz",
    "canzone.mp3",
    "video_demo.mp4",
    "dati.csv",
    "config.json",
    "sottocartella/report_annuale.pdf",
    "sottocartella/vecchia_foto.jpeg",
];

/// Cartella temporanea unica; rimossa dal `Drop` anche se il test fallisce.
struct TempTree(PathBuf);

impl TempTree {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("foai-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).expect("creazione cartella temporanea");
        for rel in SAMPLE_FILES {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("creazione sottocartella");
            }
            fs::write(&path, format!("contenuto finto di {rel}\n")).expect("scrittura file finto");
        }
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn as_str(&self) -> &str {
        self.0.to_str().expect("path non UTF-8")
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Percorsi relativi di tutti i file presenti sotto `root`, normalizzati con `/`.
/// Il log di undo scritto dall'app non e' un file dell'utente: resta fuori.
fn relative_files(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("lettura cartella").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root).expect("prefisso radice");
                let rel = rel.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
                if rel != file_operations::UNDO_FILE {
                    out.insert(rel);
                }
            }
        }
    }
    out
}

#[test]
fn full_cycle_scan_plan_apply_undo() {
    let tree = TempTree::new("cycle");
    let before = relative_files(tree.path());
    assert_eq!(before.len(), SAMPLE_FILES.len(), "setup: file finti non creati tutti");

    // --- scansione ---
    let mut progress_calls = 0usize;
    let files = scanner::scan_directory(tree.as_str(), Vec::new(), |_, _| progress_calls += 1)
        .expect("scansione fallita");
    assert_eq!(files.len(), SAMPLE_FILES.len(), "scansione: conteggio file sbagliato");
    assert!(progress_calls > 0, "scansione: nessun evento di progresso emesso");

    // --- proposta (fallback: deterministica, niente rete) ---
    let proposal = ai_analyzer::fallback_proposal(&files);
    assert_eq!(proposal.source, "fallback");
    assert!(!proposal.rules.is_empty(), "proposta senza regole");

    // --- piano ---
    let plan = file_mapper::create_mapping_plan(tree.as_str(), &files, &proposal);
    assert_eq!(
        plan.moves.len() + plan.statistics.unmatched,
        files.len(),
        "piano: ogni file deve essere spostato o risultare non mappato"
    );
    assert!(plan.moves.len() >= files.len() - 1, "piano: troppi file non mappati");
    for mv in &plan.moves {
        assert!(
            Path::new(&mv.destination).starts_with(tree.path()),
            "piano: destinazione fuori dalla radice: {}",
            mv.destination
        );
    }

    // --- applicazione ---
    let cancel = AtomicBool::new(false);
    let mut applied = 0usize;
    let (result, log) = file_operations::apply_mapping_plan(&plan, &cancel, |done, _, _| {
        applied = done;
    })
    .expect("applicazione fallita");
    assert_eq!(result.status, "success", "apply: errori {:?}", result.errors);
    assert_eq!(result.files_moved, plan.moves.len());
    assert!(result.errors.is_empty(), "apply: {:?}", result.errors);
    assert!(result.undo_available);
    assert!(
        tree.path().join(file_operations::UNDO_FILE).is_file(),
        "apply: log di undo non scritto"
    );
    assert_eq!(applied, plan.moves.len(), "apply: progresso finale incoerente");

    // Nessun file perso: stessi nomi, posizioni nuove.
    let after = relative_files(tree.path());
    let after_names: BTreeSet<&str> =
        after.iter().map(|p| p.rsplit('/').next().unwrap()).collect();
    let before_names: BTreeSet<&str> =
        before.iter().map(|p| p.rsplit('/').next().unwrap()).collect();
    assert_eq!(after_names, before_names, "apply: file persi o rinominati");
    assert_ne!(after, before, "apply: niente si e' mosso");
    for mv in &plan.moves {
        assert!(Path::new(&mv.destination).is_file(), "apply: manca {}", mv.destination);
        assert!(!Path::new(&mv.source).exists(), "apply: originale rimasto {}", mv.source);
    }

    // --- undo ---
    file_operations::undo(&log).expect("undo fallito");
    assert_eq!(relative_files(tree.path()), before, "undo: albero non ripristinato");
    assert!(
        !tree.path().join(file_operations::UNDO_FILE).exists(),
        "undo: log rimasto sul disco"
    );
}

#[test]
fn cancelled_apply_leaves_tree_untouched() {
    let tree = TempTree::new("cancel");
    let before = relative_files(tree.path());

    let files = scanner::scan_directory(tree.as_str(), Vec::new(), |_, _| {}).expect("scansione");
    let proposal = ai_analyzer::fallback_proposal(&files);
    let plan = file_mapper::create_mapping_plan(tree.as_str(), &files, &proposal);

    // Annullato in partenza: il rollback deve riportare tutto com'era.
    let cancel = AtomicBool::new(true);
    let outcome = file_operations::apply_mapping_plan(&plan, &cancel, |_, _, _| {});
    let status = match outcome {
        Ok((result, _)) => result.status,
        Err(_) => "error".to_string(),
    };
    assert_ne!(status, "success", "apply annullato non deve riportare success");
    assert_eq!(relative_files(tree.path()), before, "rollback: albero non ripristinato");
}

/// Ciclo completo contro Ollama vero. Richiede `ollama serve` e il modello
/// scaricato, quindi non gira di default:
/// `cargo test -- --ignored live_ollama_proposal`
#[test]
#[ignore]
fn live_ollama_proposal() {
    let tree = TempTree::new("ollama");
    let files = scanner::scan_directory(tree.as_str(), Vec::new(), |_, _| {}).expect("scansione");

    let runtime = tokio::runtime::Runtime::new().expect("runtime tokio");
    let online = runtime.block_on(ai_analyzer::check_ollama_connection());
    assert!(online, "Ollama non risponde su localhost:11434");

    let model = std::env::var("FOAI_TEST_MODEL").unwrap_or_else(|_| "mistral".to_string());
    let proposal =
        runtime.block_on(ai_analyzer::get_organization_proposal(&files, true, &model, None));
    assert_eq!(proposal.source, "ollama", "risposta non valida: si e' usato il fallback");
    assert!(!proposal.rules.is_empty(), "proposta AI senza regole");

    let plan = file_mapper::create_mapping_plan(tree.as_str(), &files, &proposal);
    assert!(!plan.moves.is_empty(), "piano vuoto dalla proposta AI");
    for mv in &plan.moves {
        assert!(
            Path::new(&mv.destination).starts_with(tree.path()),
            "destinazione AI fuori dalla radice: {}",
            mv.destination
        );
    }
}
