// Niente console nera dietro la finestra nelle build di release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai_analyzer;
mod ai_conversationalist;
mod file_mapper;
mod file_operations;
mod scanner;
mod storage;

#[cfg(test)]
mod e2e;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use ai_analyzer::OrganizationProposal;
use ai_conversationalist::{AiConfig, AiStatus, ChatTurn, Intent};
use file_mapper::{MappingPlan, PlanPreview};
use file_operations::ApplyResult;
use scanner::FileInfo;
use storage::{ChatMetadata, Config, Preferences, StoredChat};

/// Una cartella scansionata con i suoi file. Le scansioni restano nel processo
/// Rust: passarle avanti e indietro sulla IPC a ogni messaggio, con 100k file,
/// bloccherebbe la UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderScan {
    pub root: String,
    pub files: Vec<FileInfo>,
}

#[derive(Default)]
struct AppState {
    /// Alzato da `cancel_apply`, letto dal ciclo di spostamento.
    cancel: Arc<AtomicBool>,
    /// Scansioni per chat: chat_id -> cartelle scansionate.
    scans: Mutex<HashMap<String, Vec<FolderScan>>>,
}

#[derive(Clone, Serialize)]
struct ScanProgress {
    scanned: usize,
    current: String,
}

#[derive(Clone, Serialize)]
struct ApplyProgress {
    done: usize,
    total: usize,
    current: String,
}

/// Risposta di LOGICA a un messaggio: testo + eventuale azione gia' eseguita.
#[derive(Debug, Clone, Serialize)]
struct ChatReply {
    response: String,
    /// "chat" | "scanned" | "proposal" | "applied" | "undone" | "cancelled" | "error"
    intent: String,
    proposal: Option<OrganizationProposal>,
    preview: Option<PlanPreview>,
    apply_result: Option<ApplyResult>,
    folders: Vec<String>,
    file_count: usize,
    /// Errore non bloccante (es. AI irraggiungibile): la chat continua.
    error: Option<String>,
}

impl ChatReply {
    fn new(intent: &str, response: String) -> Self {
        Self {
            response,
            intent: intent.to_string(),
            proposal: None,
            preview: None,
            apply_result: None,
            folders: Vec::new(),
            file_count: 0,
            error: None,
        }
    }
}

/* --------------------------------- helper --------------------------------- */

// unwrap sui mutex: si avvelenano solo se un altro thread e' andato in panic
// mentre li teneva, e in quel caso vogliamo fallire in modo rumoroso.
fn scans_of(app: &AppHandle, chat_id: &str) -> Vec<FolderScan> {
    let state: State<AppState> = app.state();
    let scans = state.scans.lock().unwrap();
    scans.get(chat_id).cloned().unwrap_or_default()
}

fn set_scans(app: &AppHandle, chat_id: &str, value: Vec<FolderScan>) {
    let state: State<AppState> = app.state();
    state.scans.lock().unwrap().insert(chat_id.to_string(), value);
}

fn file_count(scans: &[FolderScan]) -> usize {
    scans.iter().map(|s| s.files.len()).sum()
}

fn all_files(scans: &[FolderScan]) -> Vec<FileInfo> {
    scans.iter().flat_map(|s| s.files.iter().cloned()).collect()
}

/// Aggiorna i percorsi dei file in memoria dopo che sono stati spostati.
/// Senza questo, un secondo giro di organizzazione nella stessa chat
/// pianificherebbe partendo da percorsi che non esistono piu'.
fn remap_scans(app: &AppHandle, chat_id: &str, moved: &[(String, String)]) {
    if moved.is_empty() {
        return;
    }
    let map: HashMap<String, String> = moved
        .iter()
        .map(|(from, to)| (from.to_lowercase(), to.clone()))
        .collect();

    let state: State<AppState> = app.state();
    let mut scans = state.scans.lock().unwrap();
    let Some(folders) = scans.get_mut(chat_id) else { return };
    for folder in folders.iter_mut() {
        remap_folder(folder, &map);
    }
}

/// Applica la mappa "vecchio percorso (lowercase) -> nuovo percorso" a una cartella.
fn remap_folder(folder: &mut FolderScan, map: &HashMap<String, String>) {
    let root = folder.root.clone();
    for file in folder.files.iter_mut() {
        let Some(new_path) = map.get(&file.path.to_lowercase()) else { continue };
        file.path = new_path.clone();
        file.relative_path = new_path
            .strip_prefix(&root)
            .unwrap_or(new_path)
            .trim_start_matches(['/', '\\'])
            .replace('\\', "/");
    }
}

fn plans_for(scans: &[FolderScan], proposal: &OrganizationProposal) -> Vec<MappingPlan> {
    scans
        .iter()
        .map(|s| file_mapper::create_mapping_plan(&s.root, &s.files, proposal))
        .collect()
}

/// Scansione bloccante fuori dal thread della UI, con eventi di progresso.
async fn scan_one(app: AppHandle, path: String) -> Result<FolderScan, String> {
    let root = path.clone();
    let files = tauri::async_runtime::spawn_blocking(move || {
        scanner::scan_directory(&path, Vec::new(), |scanned, current| {
            let _ = app.emit("scan-progress", ScanProgress { scanned, current: current.to_string() });
        })
    })
    .await
    .map_err(|e| format!("Scansione interrotta: {e}"))??;
    Ok(FolderScan { root, files })
}

async fn apply_plans(app: AppHandle, chat_id: &str, plans: Vec<MappingPlan>) -> Result<ApplyResult, String> {
    let cancel = {
        let state: State<AppState> = app.state();
        state.cancel.store(false, Ordering::Relaxed);
        state.cancel.clone()
    };

    // Copia delle mosse: serve dopo, per aggiornare i percorsi in memoria.
    let moved: Vec<(String, String)> = plans
        .iter()
        .flat_map(|p| p.moves.iter().map(|m| (m.source.clone(), m.destination.clone())))
        .collect();

    let emitter = app.clone();
    let (result, logs) = tauri::async_runtime::spawn_blocking(move || {
        file_operations::apply_plans(&plans, &cancel, |done, total, current| {
            let _ = emitter.emit(
                "apply-progress",
                ApplyProgress { done, total, current: current.to_string() },
            );
        })
    })
    .await
    .map_err(|e| format!("Applicazione interrotta: {e}"))??;

    remap_scans(&app, chat_id, &moved);
    // Il log su disco rende l'undo disponibile anche dopo un riavvio dell'app.
    storage::push_operation(chat_id, result.files_moved, logs)?;
    Ok(result)
}

/* -------------------------------- comandi -------------------------------- */

/// Cuore dell'app: un messaggio dell'utente, una risposta di LOGICA. Le azioni
/// riconosciute (scansione, proposta, applicazione, undo) vengono eseguite qui,
/// cosi' il frontend deve solo mostrare quello che torna.
#[tauri::command]
async fn chat_message(
    app: AppHandle,
    message: String,
    chat_id: String,
    history: Vec<ChatTurn>,
    config: AiConfig,
    proposal: Option<OrganizationProposal>,
) -> Result<ChatReply, String> {
    let scans = scans_of(&app, &chat_id);
    let has_files = !scans.is_empty();
    let has_proposal = proposal.is_some();

    let mut reply = match ai_conversationalist::classify(&message, has_files, has_proposal) {
        /* ------------------------------- undo ------------------------------ */
        Intent::Undo => match storage::take_last_operation(&chat_id)? {
            Some(op) => {
                let errors = file_operations::undo_logs(&op.logs);
                remap_scans(&app, &chat_id, &undo_pairs(&op));
                if errors.is_empty() {
                    ChatReply::new(
                        "undone",
                        format!("Fatto: ho rimesso {} file dove stavano prima.", op.files_moved),
                    )
                } else {
                    let mut r = ChatReply::new(
                        "error",
                        "Ho provato ad annullare ma qualche file non e' tornato al suo posto.".to_string(),
                    );
                    r.error = Some(errors.join("; "));
                    r
                }
            }
            // Niente da annullare sul disco: se c'e' una proposta in sospeso,
            // "annulla" vuol dire lasciar perdere quella.
            None if has_proposal => ChatReply::new(
                "cancelled",
                "Ok, lascio perdere questo schema. Dimmi come preferisci procedere.".to_string(),
            ),
            None => ChatReply::new(
                "chat",
                "In questa chat non ho ancora spostato niente, quindi non c'e' nulla da annullare.".to_string(),
            ),
        },

        /* ------------------------------ applica ---------------------------- */
        Intent::Apply => {
            // has_proposal e' true, quindi l'unwrap qui non puo' fallire.
            let proposal = proposal.clone().expect("classify garantisce la proposta");
            let plans = plans_for(&scans, &proposal);
            let moves: usize = plans.iter().map(|p| p.moves.len()).sum();
            if moves == 0 {
                ChatReply::new(
                    "chat",
                    "Non c'e' niente da spostare: i file sono gia' dove li metterebbe questo schema.".to_string(),
                )
            } else {
                match apply_plans(app.clone(), &chat_id, plans).await {
                    Ok(result) => {
                        let mut r = ChatReply::new(
                            "applied",
                            format!(
                                "Fatto! Ho spostato {} file in {} cartelle.\nSe non ti convince, scrivi \"annulla\" e rimetto tutto com'era.",
                                result.files_moved, result.folders_created
                            ),
                        );
                        r.apply_result = Some(result);
                        r
                    }
                    Err(e) => {
                        let mut r = ChatReply::new(
                            "error",
                            "Non sono riuscita a completare lo spostamento: ho rimesso tutto com'era.".to_string(),
                        );
                        r.error = Some(e);
                        r
                    }
                }
            }
        }

        /* ------------------------------ scansione --------------------------- */
        Intent::Scan(folders) => {
            let mut scanned: Vec<FolderScan> = Vec::new();
            let mut errors: Vec<String> = Vec::new();
            for folder in &folders {
                match scan_one(app.clone(), folder.clone()).await {
                    Ok(s) => scanned.push(s),
                    Err(e) => errors.push(e),
                }
            }

            let files = all_files(&scanned);
            let roots: Vec<String> = scanned.iter().map(|s| s.root.clone()).collect();
            set_scans(&app, &chat_id, scanned);

            let mut r = ChatReply::new("scanned", ai_conversationalist::scan_summary_reply(&roots, &files));
            if !errors.is_empty() {
                r.error = Some(errors.join("; "));
            }
            r
        }

        /* ------------------------------- proposta -------------------------- */
        Intent::Propose(grouping) => {
            let files = all_files(&scans);
            let proposal = ai_conversationalist::build_proposal(&files, grouping, &config).await;
            let plans = plans_for(&scans, &proposal);
            let mut r = ChatReply::new(
                "proposal",
                ai_conversationalist::proposal_reply(&proposal, files.len()),
            );
            r.preview = Some(file_mapper::summarize(&plans));
            r.proposal = Some(proposal);
            r
        }

        /* -------------------------------- chat ----------------------------- */
        Intent::Chat => {
            let folders: Vec<String> = scans.iter().map(|s| s.root.clone()).collect();
            let (text, ai_proposal, error) = ai_conversationalist::chat_reply(
                &message,
                &history,
                &folders,
                file_count(&scans),
                has_proposal,
                &config,
            )
            .await;

            let mut r = ChatReply::new("chat", text);
            r.error = error;
            // Una proposta arrivata dal modello vale solo se abbiamo file su cui applicarla.
            if let Some(p) = ai_proposal {
                if has_files {
                    r.intent = "proposal".to_string();
                    r.preview = Some(file_mapper::summarize(&plans_for(&scans, &p)));
                    r.proposal = Some(p);
                }
            }
            r
        }
    };

    let scans = scans_of(&app, &chat_id);
    reply.folders = scans.iter().map(|s| s.root.clone()).collect();
    reply.file_count = file_count(&scans);
    Ok(reply)
}

/// Scansione lanciata dal pulsante "aggiungi cartella" invece che dalla chat.
#[tauri::command]
async fn scan_folders(app: AppHandle, chat_id: String, folders: Vec<String>) -> Result<ChatReply, String> {
    let mut scanned = scans_of(&app, &chat_id);
    let mut errors = Vec::new();

    for folder in folders {
        if scanned.iter().any(|s| s.root.eq_ignore_ascii_case(&folder)) {
            continue;
        }
        match scan_one(app.clone(), folder).await {
            Ok(s) => scanned.push(s),
            Err(e) => errors.push(e),
        }
    }

    let files = all_files(&scanned);
    let roots: Vec<String> = scanned.iter().map(|s| s.root.clone()).collect();
    set_scans(&app, &chat_id, scanned);

    let mut reply = ChatReply::new("scanned", ai_conversationalist::scan_summary_reply(&roots, &files));
    reply.file_count = files.len();
    reply.folders = roots;
    if !errors.is_empty() {
        reply.error = Some(errors.join("; "));
    }
    Ok(reply)
}

/// Ricalcola l'anteprima dopo che l'utente ha modificato lo schema a mano.
#[tauri::command]
fn preview_proposal(
    app: AppHandle,
    chat_id: String,
    proposal: OrganizationProposal,
) -> Result<PlanPreview, String> {
    let scans = scans_of(&app, &chat_id);
    if scans.is_empty() {
        return Err("Nessuna cartella scansionata in questa chat.".to_string());
    }
    Ok(file_mapper::summarize(&plans_for(&scans, &proposal)))
}

/// Applica la proposta (pulsante Approva).
#[tauri::command]
async fn apply_proposal(
    app: AppHandle,
    chat_id: String,
    proposal: OrganizationProposal,
) -> Result<ApplyResult, String> {
    let scans = scans_of(&app, &chat_id);
    if scans.is_empty() {
        return Err("Nessuna cartella scansionata in questa chat.".to_string());
    }
    let plans = plans_for(&scans, &proposal);
    if plans.iter().all(|p| p.moves.is_empty()) {
        return Err("Non c'e' nessun file da spostare con questo schema.".to_string());
    }
    apply_plans(app, &chat_id, plans).await
}

/// Mosse inverse di un'operazione: (dove sta ora, dove torna).
fn undo_pairs(op: &storage::Operation) -> Vec<(String, String)> {
    op.logs
        .iter()
        .flat_map(|log| log.moved.iter().map(|(dest, source)| (dest.clone(), source.clone())))
        .collect()
}

#[tauri::command]
fn undo_last_action(app: AppHandle, chat_id: String) -> Result<String, String> {
    let op = storage::take_last_operation(&chat_id)?
        .ok_or_else(|| "Nessuna operazione da annullare in questa chat.".to_string())?;
    let errors = file_operations::undo_logs(&op.logs);
    remap_scans(&app, &chat_id, &undo_pairs(&op));
    if errors.is_empty() {
        Ok(format!("{} file riportati nella posizione originale.", op.files_moved))
    } else {
        Err(format!("Undo completato con problemi: {}", errors.join("; ")))
    }
}

#[tauri::command]
fn has_undo(chat_id: String) -> bool {
    storage::has_undoable_operation(&chat_id)
}

#[tauri::command]
fn cancel_apply(state: State<AppState>) {
    state.cancel.store(true, Ordering::Relaxed);
}

/* ------------------------------- persistenza ------------------------------ */

#[tauri::command]
fn save_chat(chat: StoredChat) -> Result<(), String> {
    storage::save_chat(&chat)
}

#[tauri::command]
fn load_chat(chat_id: String) -> Result<StoredChat, String> {
    storage::load_chat(&chat_id)
}

#[tauri::command]
fn list_chats() -> Result<Vec<ChatMetadata>, String> {
    storage::list_chats()
}

#[tauri::command]
fn delete_chat(app: AppHandle, chat_id: String) -> Result<(), String> {
    let state: State<AppState> = app.state();
    state.scans.lock().unwrap().remove(&chat_id);
    storage::delete_chat(&chat_id)
}

#[tauri::command]
fn get_preferences() -> Preferences {
    storage::load_preferences()
}

#[tauri::command]
fn set_preferences(preferences: Preferences) -> Result<(), String> {
    storage::save_preferences(&preferences)
}

#[tauri::command]
fn get_config() -> Config {
    storage::load_config()
}

#[tauri::command]
fn set_config(config: Config) -> Result<(), String> {
    storage::save_config(&config)
}

/* --------------------------------- sistema -------------------------------- */

#[tauri::command]
fn open_folder_in_explorer(path: String) -> Result<(), String> {
    file_operations::open_in_explorer(&path)
}

#[tauri::command]
async fn check_ai_status(ollama_url: Option<String>) -> AiStatus {
    let url = ollama_url.unwrap_or_else(|| ai_analyzer::DEFAULT_OLLAMA_URL.to_string());
    ai_conversationalist::check_status(&url).await
}

fn main() {
    // ~/.logica/ deve esistere prima del primo messaggio.
    if let Err(e) = storage::ensure_layout() {
        eprintln!("[logica] impossibile preparare ~/.logica: {e}");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            chat_message,
            scan_folders,
            preview_proposal,
            apply_proposal,
            undo_last_action,
            has_undo,
            cancel_apply,
            save_chat,
            load_chat,
            list_chats,
            delete_chat,
            get_preferences,
            set_preferences,
            get_config,
            set_config,
            open_folder_in_explorer,
            check_ai_status,
        ])
        .run(tauri::generate_context!())
        .expect("errore durante l'avvio di LOGICA");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, relative: &str) -> FileInfo {
        FileInfo {
            name: path.rsplit(['/', '\\']).next().unwrap().to_string(),
            path: path.to_string(),
            extension: "pdf".to_string(),
            size: 1,
            modified: "2026-08-29T10:00:00Z".to_string(),
            relative_path: relative.to_string(),
            depth: 1,
            content_preview: None,
        }
    }

    /// Dopo l'applicazione i file in memoria devono puntare alla nuova posizione:
    /// altrimenti un secondo giro nella stessa chat pianifica su percorsi morti.
    #[test]
    fn remap_follows_the_files_that_moved() {
        let mut folder = FolderScan {
            root: "C:/tmp".to_string(),
            files: vec![file("C:/tmp/a.pdf", "a.pdf"), file("C:/tmp/b.pdf", "b.pdf")],
        };
        let map: HashMap<String, String> =
            [("c:/tmp/a.pdf".to_string(), "C:/tmp/Documenti/PDF/a.pdf".to_string())].into();

        remap_folder(&mut folder, &map);

        assert_eq!(folder.files[0].path, "C:/tmp/Documenti/PDF/a.pdf");
        assert_eq!(folder.files[0].relative_path, "Documenti/PDF/a.pdf");
        // Chi non si e' mosso resta dov'era.
        assert_eq!(folder.files[1].path, "C:/tmp/b.pdf");
        assert_eq!(folder.files[1].relative_path, "b.pdf");
    }
}
