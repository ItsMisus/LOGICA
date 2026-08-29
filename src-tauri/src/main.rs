// Niente console nera dietro la finestra nelle build di release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai_analyzer;
mod file_mapper;
mod file_operations;
mod scanner;

#[cfg(test)]
mod e2e;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use ai_analyzer::OrganizationProposal;
use file_mapper::MappingPlan;
use file_operations::{ApplyResult, UndoLog};
use scanner::FileInfo;

#[derive(Default)]
struct AppState {
    /// Alzato da `cancel_apply`, letto dal ciclo di spostamento.
    cancel: Arc<AtomicBool>,
    /// Ultima operazione applicata, per l'undo.
    undo_log: Mutex<Option<UndoLog>>,
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

/* -------------------------------- comandi -------------------------------- */

#[tauri::command]
async fn scan_directory(app: AppHandle, path: String) -> Result<Vec<FileInfo>, String> {
    // Il walk e' bloccante: fuori dal thread della UI.
    tauri::async_runtime::spawn_blocking(move || {
        scanner::scan_directory(&path, Vec::new(), |scanned, current| {
            let _ = app.emit(
                "scan-progress",
                ScanProgress { scanned, current: current.to_string() },
            );
        })
    })
    .await
    .map_err(|e| format!("Scansione interrotta: {e}"))?
}

#[tauri::command]
async fn get_organization_proposal(
    files: Vec<FileInfo>,
    use_ollama: bool,
    model: String,
    user_preferences: Option<String>,
) -> Result<OrganizationProposal, String> {
    // Non ritorna mai Err: senza AI si usa lo schema per estensione.
    Ok(ai_analyzer::get_organization_proposal(
        &files,
        use_ollama,
        &model,
        user_preferences.as_deref(),
    )
    .await)
}

#[tauri::command]
fn create_mapping_plan(
    root: String,
    files: Vec<FileInfo>,
    proposal: OrganizationProposal,
) -> Result<MappingPlan, String> {
    Ok(file_mapper::create_mapping_plan(&root, &files, &proposal))
}

#[tauri::command]
async fn apply_mapping_plan(app: AppHandle, plan: MappingPlan) -> Result<ApplyResult, String> {
    let cancel = {
        let state: State<AppState> = app.state();
        state.cancel.store(false, Ordering::Relaxed);
        state.cancel.clone()
    };

    let emitter = app.clone();
    let (result, log) = tauri::async_runtime::spawn_blocking(move || {
        file_operations::apply_mapping_plan(&plan, &cancel, |done, total, current| {
            let _ = emitter.emit(
                "apply-progress",
                ApplyProgress { done, total, current: current.to_string() },
            );
        })
    })
    .await
    .map_err(|e| format!("Applicazione interrotta: {e}"))??;

    let state: State<AppState> = app.state();
    // unwrap: il mutex viene avvelenato solo se un altro thread e' andato in panic
    // mentre lo teneva; in quel caso vogliamo davvero fallire in modo rumoroso.
    *state.undo_log.lock().unwrap() = Some(log);
    Ok(result)
}

#[tauri::command]
fn cancel_apply(state: State<AppState>) {
    state.cancel.store(true, Ordering::Relaxed);
}

#[tauri::command]
fn undo_last_operation(state: State<AppState>) -> Result<String, String> {
    let log = state
        .undo_log
        .lock()
        .unwrap() // vedi nota sopra sul mutex avvelenato
        .take()
        .ok_or_else(|| "Nessuna operazione da annullare.".to_string())?;
    file_operations::undo(&log)
}

#[tauri::command]
fn open_folder_in_explorer(path: String) -> Result<(), String> {
    file_operations::open_in_explorer(&path)
}

#[tauri::command]
async fn check_ollama_connection() -> Result<bool, String> {
    Ok(ai_analyzer::check_ollama_connection().await)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            scan_directory,
            get_organization_proposal,
            create_mapping_plan,
            apply_mapping_plan,
            cancel_apply,
            undo_last_operation,
            open_folder_in_explorer,
            check_ollama_connection,
        ])
        .run(tauri::generate_context!())
        .expect("errore durante l'avvio di File Organizer AI");
}
