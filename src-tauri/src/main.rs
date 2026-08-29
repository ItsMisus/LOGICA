// Niente console nera dietro la finestra nelle build di release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod assistant;
mod attachments;
mod server;
mod storage;
mod web;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::broadcast;

use ai::{AiConfig, AiStatus, ChatTurn};
use storage::{ChatMetadata, Config, Preferences, StoredChat};

struct AppState {
    /// Alzato da `stop_talk`, letto dallo stream della risposta AI.
    stop: Arc<AtomicBool>,
    /// Gli stessi eventi di avanzamento che riceve la finestra, in partenza
    /// verso i browser collegati al server web.
    events: broadcast::Sender<server::api::Envelope>,
    /// Il server web: spento finche' non lo si accende dalle impostazioni.
    server: server::ServerControl,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            stop: Arc::default(),
            events: broadcast::channel(server::EVENT_CAPACITY).0,
            server: server::ServerControl::default(),
        }
    }
}

/* --------------------------------- helper --------------------------------- */

/// Manda un evento di avanzamento a chiunque stia guardando: la finestra
/// desktop tramite Tauri, i browser collegati tramite il canale del server.
/// Un errore su uno dei due non deve fermare l'operazione in corso.
fn emit_all<T: Serialize + Clone>(app: &AppHandle, event: &str, payload: T) {
    let _ = app.emit(event, payload.clone());

    let state: State<AppState> = app.state();
    if let Ok(value) = serde_json::to_value(payload) {
        // Fallisce solo se nessun browser e' in ascolto: e' il caso normale.
        let _ = state
            .events
            .send(server::api::Envelope { event: event.to_string(), payload: value });
    }
}



/// Pezzo di risposta in arrivo: il frontend lo aggiunge al messaggio in corso.
#[derive(Clone, Serialize)]
struct AssistantDelta {
    chat_id: String,
    text: String,
}

/// Esito del turno, quando lo streaming e' finito.
#[derive(Debug, Clone, Serialize)]
struct TalkReply {
    text: String,
    /// L'utente ha premuto "Interrompi": il testo e' parziale ma si tiene.
    stopped: bool,
    /// Problema non bloccante a meta' risposta.
    error: Option<String>,
    /// Pagine consultate su internet per questa risposta.
    sources: Vec<web::SearchHit>,
}

/// Cosa sta facendo l'app prima di cominciare a scrivere: una ricerca, una
/// pagina aperta. Senza, quei secondi sembrano l'app bloccata.
#[derive(Clone, Serialize)]
struct AssistantStatus {
    chat_id: String,
    text: String,
}

/// Un turno di conversazione. Il testo arriva al frontend evento per evento
/// (`assistant-delta`); qui torna solo la versione completa, per il salvataggio.
#[tauri::command]
async fn talk(
    app: AppHandle,
    chat_id: String,
    message: String,
    history: Vec<ChatTurn>,
    // Immagini allegate a questo messaggio, per i modelli che vedono. Manca
    // nelle chiamate scritte prima che gli allegati diventassero visivi.
    images: Option<Vec<attachments::Image>>,
    config: AiConfig,
) -> Result<TalkReply, String> {
    let cancel = {
        let state: State<AppState> = app.state();
        state.stop.store(false, Ordering::Relaxed);
        state.stop.clone()
    };

    let images = images.unwrap_or_default();

    let emitter = app.clone();
    let stream_id = chat_id.clone();
    let status_emitter = app.clone();
    let status_id = chat_id.clone();
    let reply = assistant::stream_reply(
        &message,
        &history,
        &images,
        &config,
        &cancel,
        move |delta| {
            emit_all(
                &emitter,
                "assistant-delta",
                AssistantDelta { chat_id: stream_id.clone(), text: delta.to_string() },
            );
        },
        move |status| {
            emit_all(
                &status_emitter,
                "assistant-status",
                AssistantStatus { chat_id: status_id.clone(), text: status.to_string() },
            );
        },
    )
    .await?;

    Ok(TalkReply {
        text: reply.text,
        stopped: reply.stopped,
        error: reply.error,
        sources: reply.sources,
    })
}

/// "Interrompi": il testo gia' scritto resta nella chat.
#[tauri::command]
fn stop_talk(state: State<AppState>) {
    state.stop.store(true, Ordering::Relaxed);
}

/// Contenuto dei file allegati a un messaggio. I percorsi arrivano dal dialogo
/// di sistema: qui si legge solo, non si scrive e non si sposta niente.
#[tauri::command]
fn read_attachments(paths: Vec<String>) -> Vec<attachments::Attachment> {
    attachments::read_all(&paths)
}

/// Titolo della conversazione, chiesto al modello dopo il primo scambio.
/// Se non ne esce niente di sensato il frontend tiene quello che ha.
#[tauri::command]
async fn suggest_title(question: String, answer: String, config: AiConfig) -> Option<String> {
    assistant::suggest_title(&question, &answer, &config).await
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
fn delete_chat(chat_id: String) -> Result<(), String> {
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
async fn check_ai_status(ollama_url: Option<String>) -> AiStatus {
    let url = ollama_url.unwrap_or_else(|| ai::DEFAULT_OLLAMA_URL.to_string());
    ai::check_status(&url).await
}

/* ----------------------------- accesso remoto ----------------------------- */

#[tauri::command]
fn server_status(app: AppHandle) -> server::ServerStatus {
    let state: State<AppState> = app.state();
    state.server.status()
}

#[tauri::command]
async fn server_start(app: AppHandle) -> Result<server::ServerStatus, String> {
    let events = {
        let state: State<AppState> = app.state();
        state.events.clone()
    };
    server::start(app.clone(), events).await
}

#[tauri::command]
fn server_stop(app: AppHandle) -> server::ServerStatus {
    let state: State<AppState> = app.state();
    state.server.stop();
    state.server.status()
}

/// Imposta la password per l'accesso da browser.
///
/// Il server viene spento: una sessione aperta con la vecchia password non deve
/// sopravvivere al cambio, e le sessioni vivono dentro il server.
#[tauri::command]
fn server_set_password(app: AppHandle, password: String) -> Result<server::ServerStatus, String> {
    let hash = server::auth::hash_password(&password)?;
    let mut settings = server::auth::load_settings();
    settings.password_hash = Some(hash);
    server::auth::save_settings(&settings)?;

    let state: State<AppState> = app.state();
    state.server.stop();
    Ok(state.server.status())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerOptions {
    port: u16,
    /// `true` = raggiungibile dagli altri dispositivi della rete locale.
    expose_to_lan: bool,
    autostart: bool,
}

/// Cambia porta, esposizione e avvio automatico. Il server va riacceso perche'
/// le prime due si decidono al momento in cui si apre la porta.
#[tauri::command]
fn server_configure(app: AppHandle, options: ServerOptions) -> Result<server::ServerStatus, String> {
    if options.port < 1024 {
        return Err("Scegli una porta da 1024 in su.".to_string());
    }
    let mut settings = server::auth::load_settings();
    settings.port = options.port;
    settings.expose_to_lan = options.expose_to_lan;
    settings.autostart = options.autostart;
    server::auth::save_settings(&settings)?;

    let state: State<AppState> = app.state();
    state.server.stop();
    Ok(state.server.status())
}

/// Butta via il certificato e ne genera uno nuovo. Da usare se la chiave
/// privata potrebbe essere finita in mani altrui: i browser mostreranno di
/// nuovo l'avviso, con una impronta diversa.
#[tauri::command]
fn server_regenerate_certificate(app: AppHandle) -> Result<server::ServerStatus, String> {
    let state: State<AppState> = app.state();
    state.server.stop();
    server::tls::regenerate()?;
    Ok(state.server.status())
}

fn main() {
    // ~/.logica/ deve esistere prima del primo messaggio.
    if let Err(e) = storage::ensure_layout() {
        eprintln!("[logica] impossibile preparare ~/.logica: {e}");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            // Accensione automatica, se l'utente l'ha chiesta e una password
            // c'e'. Senza password il server non parte comunque.
            let settings = server::auth::load_settings();
            if settings.autostart && settings.password_hash.is_some() {
                let handle = app.handle().clone();
                let events = {
                    let state: State<AppState> = handle.state();
                    state.events.clone()
                };
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = server::start(handle, events).await {
                        eprintln!("[logica] avvio automatico del server web fallito: {e}");
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            talk,
            stop_talk,
            read_attachments,
            suggest_title,
            save_chat,
            load_chat,
            list_chats,
            delete_chat,
            get_preferences,
            set_preferences,
            get_config,
            set_config,
            check_ai_status,
            server_status,
            server_start,
            server_stop,
            server_set_password,
            server_configure,
            server_regenerate_certificate,
        ])
        .run(tauri::generate_context!())
        .expect("errore durante l'avvio di LOGICA");
}
