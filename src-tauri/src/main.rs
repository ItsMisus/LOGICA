// Niente console nera dietro la finestra nelle build di release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod assistant;
mod attachments;
/// La misura su cento richieste. Esiste solo nelle build di test: non e' un
/// pezzo del programma, e nell'eseguibile che si installa non ci deve stare.
#[cfg(test)]
mod eval;
mod memory;
mod rag;
mod server;
mod storage;
mod summary;
mod tools;
mod updater;
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
    /// Cartelle e file guardati sul disco per questa risposta.
    tools: Vec<tools::ToolUse>,
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
        &chat_id,
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

    // Cosa c'era da ricordare si decide dopo, in un compito a parte: la
    // risposta e' gia' a schermo e l'utente non deve aspettare una seconda
    // chiamata al modello. Se fallisce, si e' persa una nota, non un turno.
    if config.memory && !reply.stopped && !reply.text.trim().is_empty() {
        let question = message.clone();
        let answer = reply.text.clone();
        let cfg = config.clone();
        let chat = chat_id.clone();
        tauri::async_runtime::spawn(async move {
            let facts = assistant::learn(&question, &answer, &cfg).await;
            if !facts.is_empty() {
                if let Err(e) = memory::remember(&facts, &chat) {
                    eprintln!("[logica] memoria non salvata: {e}");
                }
            }
        });
    }

    Ok(TalkReply {
        text: reply.text,
        stopped: reply.stopped,
        error: reply.error,
        sources: reply.sources,
        tools: reply.tools_used,
    })
}

/* --------------------------------- memoria -------------------------------- */

/// Quello che l'assistente si ricorda dell'utente fra una chat e l'altra.
#[tauri::command]
fn get_memory() -> Vec<memory::Fact> {
    memory::list()
}

#[tauri::command]
fn forget_fact(fact_id: String) -> Result<(), String> {
    memory::forget(&fact_id)
}

#[tauri::command]
fn clear_memory() -> Result<(), String> {
    memory::clear()
}

/* ------------------------------ aggiornamenti ----------------------------- */

/// Quanto e' stato scaricato dell'aggiornamento.
#[derive(Clone, Serialize)]
struct UpdateProgress {
    downloaded: u64,
    /// Quanto pesa in tutto, quando il server lo dichiara.
    total: Option<u64>,
}

#[tauri::command]
async fn check_update(app: AppHandle) -> updater::UpdateInfo {
    updater::check(&app).await
}

/// Scarica, installa e riavvia. Non torna: se va bene, il processo e' gia'
/// finito quando il frontend leggerebbe la risposta.
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    let emitter = app.clone();
    updater::install(&app, move |downloaded, total| {
        emit_all(&emitter, "update-progress", UpdateProgress { downloaded, total });
    })
    .await
}

/* -------------------------------- documenti ------------------------------- */

/// A che punto e' l'indicizzazione, file per file.
#[derive(Clone, Serialize)]
struct IndexProgress {
    done: usize,
    total: usize,
    /// Il file in corso, vuoto quando ha finito.
    file: String,
}

#[tauri::command]
fn index_status() -> rag::IndexStatus {
    rag::status()
}

/// Costruisce (o aggiorna) l'indice dei documenti nelle cartelle autorizzate.
///
/// Dura minuti la prima volta: l'avanzamento esce come evento, cosi' la
/// finestra puo' mostrarlo invece di sembrare bloccata.
#[tauri::command]
async fn build_index(app: AppHandle, ollama_url: Option<String>) -> Result<rag::IndexStatus, String> {
    let url = ollama_url.unwrap_or_else(|| storage::load_preferences().ollama_url);
    let emitter = app.clone();
    rag::build(&url, &mut move |done, total, file| {
        emit_all(
            &emitter,
            "index-progress",
            IndexProgress { done, total, file: file.to_string() },
        );
    })
    .await
}

#[tauri::command]
fn clear_index() -> Result<(), String> {
    rag::clear()
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

/// Un'immagine incollata negli appunti: non ha un percorso, quindi arriva
/// gia' in base64 invece che per nome di file.
#[tauri::command]
fn attach_image(
    name: String,
    media_type: String,
    data: String,
) -> Result<attachments::Attachment, String> {
    attachments::from_image_data(&name, &media_type, &data)
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
    storage::delete_chat(&chat_id)?;
    // Il riassunto della parte vecchia non ha piu' una conversazione a cui
    // appartenere.
    summary::forget(&chat_id);
    Ok(())
}

/// Cerca dentro le conversazioni salvate, contenuto compreso.
#[tauri::command]
fn search_chats(query: String) -> Result<Vec<storage::ChatMatch>, String> {
    storage::search_chats(&query)
}

#[tauri::command]
fn rename_chat(chat_id: String, title: String) -> Result<ChatMetadata, String> {
    storage::rename_chat(&chat_id, &title)
}

#[tauri::command]
fn duplicate_chat(chat_id: String) -> Result<ChatMetadata, String> {
    let copy = storage::duplicate_chat(&chat_id)?;
    // La copia ha gli stessi messaggi: le tocca anche lo stesso riassunto,
    // altrimenti il primo turno nella copia lo rifarebbe da zero.
    if let Some(digest) = summary::load(&chat_id) {
        if let Err(e) = summary::save(&copy.id, &digest) {
            eprintln!("[logica] riassunto non copiato: {e}");
        }
    }
    Ok(copy)
}

/// Il testo della conversazione da salvare fuori dall'app: `markdown` per
/// leggerla, `json` per riaverla intera.
#[tauri::command]
fn export_chat(chat_id: String, format: String) -> Result<String, String> {
    storage::export_chat(&chat_id, &format)
}

/// Scrive l'esportazione dove l'utente ha detto nel dialogo di sistema.
///
/// Non ha una strada dal server web: dal browser il file si scarica, e un
/// comando che scrive un percorso qualsiasi sul PC non deve esistere per chi
/// arriva dalla rete, nemmeno con la sessione valida.
#[tauri::command]
fn save_export(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| format!("Salvataggio fallito su {path}: {e}"))
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

    let mut builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());

    // L'aggiornatore si registra solo se questa build sa dove guardare e con
    // quale chiave verificare la firma. Senza, sarebbe un pezzo acceso che non
    // puo' fare niente. Indirizzo e chiave non stanno in tauri.conf.json ma
    // vengono passati a ogni controllo (vedi `updater`), cosi' il repository
    // non contiene ne' un indirizzo finto ne' una chiave finta.
    if updater::configured() {
        builder = builder
            .plugin(tauri_plugin_process::init())
            .plugin(tauri_plugin_updater::Builder::new().build());
    }

    builder
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
            attach_image,
            suggest_title,
            save_chat,
            load_chat,
            list_chats,
            delete_chat,
            search_chats,
            rename_chat,
            duplicate_chat,
            export_chat,
            save_export,
            get_preferences,
            set_preferences,
            get_config,
            set_config,
            check_ai_status,
            check_update,
            install_update,
            get_memory,
            forget_fact,
            clear_memory,
            index_status,
            build_index,
            clear_index,
            server_status,
            server_start,
            server_stop,
            server_set_password,
            server_configure,
            server_regenerate_certificate,
        ])
        .build(tauri::generate_context!())
        .expect("errore durante l'avvio di LOGICA")
        // Alla chiusura si restituisce la memoria video: il modello resta
        // caricato mentre LOGICA e' aperta (vedi `OLLAMA_KEEP_ALIVE`), non
        // mezz'ora dopo che l'utente ha chiuso la finestra.
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                assistant::release_ollama();
            }
        });
}
