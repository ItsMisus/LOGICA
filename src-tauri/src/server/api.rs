//! Le rotte HTTP.
//!
//! Il server gira dentro il processo di LOGICA, quindi puo' chiamare
//! direttamente le stesse funzioni che usa la finestra desktop: qui non c'e'
//! logica di organizzazione, solo traduzione da richiesta HTTP a chiamata, con
//! i controlli in mezzo.
//!
//! Ogni comando che tocca il disco passa da `guard`: la sessione valida dice
//! *chi* sta chiedendo, non *cosa* gli e' permesso toccare.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Path, Request, State};
use tauri::Manager;
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tower_http::limit::RequestBodyLimitLayer;

use crate::ai::{AiConfig, ChatTurn};
use crate::server::{auth, guard};
use crate::storage::{Config, Preferences, StoredChat};

/// Nome del cookie di sessione.
const COOKIE: &str = "logica_session";

/// Header con il token anti-CSRF, che solo il nostro frontend puo' leggere.
const CSRF_HEADER: &str = "x-logica-csrf";

/// Le richieste sono JSON di chat e proposte: qualche megabyte basta e avanza.
/// Oltre, la richiesta viene troncata prima di essere deserializzata.
const MAX_BODY: usize = 8 * 1024 * 1024;

/// Un evento diretto ai browser collegati (gli stessi che la finestra desktop
/// riceve come eventi Tauri).
#[derive(Debug, Clone, Serialize)]
pub struct Envelope {
    pub event: String,
    pub payload: Value,
}

/// Chi puo' entrare. Non conosce Tauri: decidere se una richiesta e'
/// autenticata non ha niente a che vedere con l'app che sta sotto, e tenerlo
/// separato permette di provarlo con un server nudo.
pub struct Guardian {
    pub sessions: auth::Sessions,
    pub throttle: auth::Throttle,
    /// Hash della password letto all'avvio del server.
    pub password_hash: String,
}

/// Cosa si puo' fare una volta entrati: qui serve l'app vera.
pub struct Ctx {
    pub app: tauri::AppHandle,
    pub guardian: Arc<Guardian>,
    pub events: broadcast::Sender<Envelope>,
}

pub fn router(ctx: Arc<Ctx>) -> Router {
    let guardian = ctx.guardian.clone();

    // Dietro il controllo di sessione.
    let protected = Router::new()
        .route("/api/cmd/{name}", post(command))
        .route("/api/events", get(events))
        .with_state(ctx.clone())
        .merge(Router::new().route("/api/logout", post(logout)).with_state(guardian.clone()))
        .layer(axum::middleware::from_fn_with_state(guardian.clone(), require_session));

    // Davanti: il login e la domanda "ho gia' una sessione?".
    let public = Router::new()
        .route("/api/login", post(login))
        .route("/api/session", get(session_info))
        .with_state(guardian);

    // L'interfaccia deve essere servita anche a chi non ha ancora fatto il
    // login: e' la pagina che contiene il login stesso.
    let files = Router::new().fallback(assets).with_state(ctx);

    Router::new()
        .merge(protected)
        .merge(public)
        .merge(files)
        .layer(RequestBodyLimitLayer::new(MAX_BODY))
        .layer(axum::middleware::from_fn(security_headers))
}

/* ------------------------------- middleware ------------------------------- */

/// Intestazioni che valgono per ogni risposta.
///
/// La CSP e' la rete di sicurezza se una risposta finisse per contenere testo
/// non nostro: il browser non caricherebbe comunque script da altrove, e la
/// pagina non puo' essere incorniciata da un altro sito.
async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; font-src 'self' data:; connect-src 'self'; \
         object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"
            .parse()
            .expect("CSP valida"),
    );
    h.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().expect("header valido"));
    h.insert(header::X_FRAME_OPTIONS, "DENY".parse().expect("header valido"));
    h.insert(header::REFERRER_POLICY, "no-referrer".parse().expect("header valido"));
    h.insert(
        "permissions-policy",
        "geolocation=(), microphone=(), camera=(), interest-cohort=()"
            .parse()
            .expect("header valido"),
    );
    // Niente HSTS: il certificato e' auto-firmato, e inchiodare il browser a
    // HTTPS su questo host renderebbe difficile tornare indietro.
    response
}

/// Cookie valido, indirizzo coerente, e per le scritture anche origine e token
/// anti-CSRF. Chi non passa di qui non arriva ai comandi.
pub(crate) async fn require_session(
    State(guardian): State<Arc<Guardian>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let headers = request.headers().clone();
    let token = match cookie_value(&headers, COOKIE) {
        Some(t) => t,
        None => return fail(StatusCode::UNAUTHORIZED, "Sessione assente."),
    };
    let session = match guardian.sessions.touch(&token, peer.ip()) {
        Some(s) => s,
        None => return fail(StatusCode::UNAUTHORIZED, "Sessione scaduta o non valida."),
    };

    if request.method() != axum::http::Method::GET {
        if let Err(e) = check_origin(&headers) {
            return fail(StatusCode::FORBIDDEN, e);
        }
        let given = headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok()).unwrap_or_default();
        if !auth::csrf_matches(&session.csrf, given) {
            return fail(StatusCode::FORBIDDEN, "Token anti-CSRF mancante o errato.");
        }
    }

    next.run(request).await
}

/// L'origine dichiarata dal browser deve coincidere con l'host richiesto.
///
/// Senza questo controllo, una pagina ostile aperta in un'altra scheda potrebbe
/// far partire richieste verso il server usando il tuo cookie. Il token
/// anti-CSRF gia' lo impedisce; questo e' il secondo strato, per quando il
/// primo avesse un buco.
fn check_origin(headers: &HeaderMap) -> Result<(), String> {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| "Origine assente.".to_string())?;
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| "Host assente.".to_string())?;

    let origin_host = origin.split("://").nth(1).unwrap_or_default();
    if origin_host.eq_ignore_ascii_case(host) {
        Ok(())
    } else {
        Err(format!("Origine {origin} non ammessa."))
    }
}

/* --------------------------------- accesso -------------------------------- */

#[derive(Deserialize)]
pub(crate) struct LoginBody {
    password: String,
}

pub(crate) async fn login(
    State(guardian): State<Arc<Guardian>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Response {
    if let Err(e) = check_origin(&headers) {
        return fail(StatusCode::FORBIDDEN, e);
    }
    let ip: IpAddr = peer.ip();

    if let Some(seconds) = guardian.throttle.locked_for(ip) {
        let minutes = seconds.div_ceil(60);
        return fail(
            StatusCode::TOO_MANY_REQUESTS,
            format!("Troppi tentativi falliti. Riprova fra {minutes} minuti."),
        );
    }

    if !auth::verify_password(&body.password, &guardian.password_hash) {
        guardian.throttle.record_failure(ip);
        return fail(StatusCode::UNAUTHORIZED, "Password errata.");
    }

    guardian.throttle.record_success(ip);
    let (token, csrf) = guardian.sessions.open(ip);

    let cookie = format!(
        "{COOKIE}={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={}",
        12 * 60 * 60
    );
    let mut response = Json(json!({ "csrf": csrf })).into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, cookie.parse().expect("cookie valido"));
    response
}

async fn logout(State(guardian): State<Arc<Guardian>>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie_value(&headers, COOKIE) {
        guardian.sessions.close(&token);
    }
    let mut response = Json(json!({ "ok": true })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        format!("{COOKIE}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0")
            .parse()
            .expect("cookie valido"),
    );
    response
}

/// Dice al frontend se ha gia' una sessione buona, senza rivelare altro.
async fn session_info(
    State(guardian): State<Arc<Guardian>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let session =
        cookie_value(&headers, COOKIE).and_then(|t| guardian.sessions.touch(&t, peer.ip()));
    match session {
        Some(s) => Json(json!({ "authenticated": true, "csrf": s.csrf })).into_response(),
        None => Json(json!({ "authenticated": false })).into_response(),
    }
}

/* --------------------------------- eventi --------------------------------- */

/// Gli eventi di avanzamento (token della risposta, scansione, spostamenti)
/// che la finestra desktop riceve come eventi Tauri.
async fn events(
    State(ctx): State<Arc<Ctx>>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let mut rx = ctx.events.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    let event = Event::default()
                        .event(envelope.event.clone())
                        .data(envelope.payload.to_string());
                    yield Ok(event);
                }
                // Client lento: si e' persa qualche notifica di avanzamento, ma
                // la connessione resta buona e il prossimo evento arriva.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/* --------------------------------- comandi -------------------------------- */

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TalkArgs {
    chat_id: String,
    message: String,
    #[serde(default)]
    history: Vec<ChatTurn>,
    /// Immagini allegate al messaggio, gia' in base64: da browser arrivano
    /// dentro la richiesta, non da un percorso sul disco.
    #[serde(default)]
    images: Vec<crate::attachments::Image>,
    config: AiConfig,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TitleArgs {
    question: String,
    answer: String,
    config: AiConfig,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathsArgs {
    paths: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatIdArgs {
    chat_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueryArgs {
    query: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenameArgs {
    chat_id: String,
    title: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportArgs {
    chat_id: String,
    format: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveChatArgs {
    chat: StoredChat,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreferencesArgs {
    preferences: Preferences,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PastedImageArgs {
    name: String,
    media_type: String,
    data: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactArgs {
    fact_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigArgs {
    config: Config,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusArgs {
    /// Il frontend lo manda per uniformita' con la modalita' desktop, ma da
    /// remoto non viene letto: lo stato si chiede sempre al motore configurato
    /// sul PC. Vedi `local_ai`.
    #[serde(default)]
    #[allow(dead_code)]
    ollama_url: Option<String>,
}

/// Traduce una richiesta in una chiamata al comando corrispondente.
///
/// I nomi sono gli stessi che usa la finestra desktop, cosi' il frontend puo'
/// avere un solo strato di chiamata per entrambe le modalita'.
async fn command(
    State(ctx): State<Arc<Ctx>>,
    Path(name): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let app = ctx.app.clone();

    match name.as_str() {
        /* ------------------------------ chat ------------------------------ */
        "talk" => match parse::<TalkArgs>(body) {
            Ok(a) => result(
                crate::talk(app, a.chat_id, a.message, a.history, Some(a.images), local_ai(a.config))
                    .await,
            ),
            Err(e) => e,
        },

        "stop_talk" => {
            crate::stop_talk(app.state());
            ok(json!(null))
        }

        "suggest_title" => match parse::<TitleArgs>(body) {
            Ok(a) => ok(json!(
                crate::suggest_title(a.question, a.answer, local_ai(a.config)).await
            )),
            Err(e) => e,
        },

        /* ----------------------------- allegati --------------------------- */
        "read_attachments" => match parse::<PathsArgs>(body) {
            Ok(a) => match guard::ensure_all_within(&a.paths) {
                Ok(()) => ok(json!(crate::read_attachments(a.paths))),
                Err(e) => fail(StatusCode::FORBIDDEN, e),
            },
            Err(e) => e,
        },

        /* --------------------------- persistenza -------------------------- */
        "save_chat" => match parse::<SaveChatArgs>(body) {
            Ok(a) => result(crate::save_chat(a.chat)),
            Err(e) => e,
        },
        "load_chat" => match parse::<ChatIdArgs>(body) {
            Ok(a) => result(crate::load_chat(a.chat_id)),
            Err(e) => e,
        },
        "list_chats" => result(crate::list_chats()),
        "delete_chat" => match parse::<ChatIdArgs>(body) {
            Ok(a) => result(crate::delete_chat(a.chat_id)),
            Err(e) => e,
        },
        "search_chats" => match parse::<QueryArgs>(body) {
            Ok(a) => result(crate::search_chats(a.query)),
            Err(e) => e,
        },
        "rename_chat" => match parse::<RenameArgs>(body) {
            Ok(a) => result(crate::rename_chat(a.chat_id, a.title)),
            Err(e) => e,
        },
        "duplicate_chat" => match parse::<ChatIdArgs>(body) {
            Ok(a) => result(crate::duplicate_chat(a.chat_id)),
            Err(e) => e,
        },
        "export_chat" => match parse::<ExportArgs>(body) {
            Ok(a) => result(crate::export_chat(a.chat_id, a.format)),
            Err(e) => e,
        },

        // Un'immagine incollata arriva col suo contenuto dentro la richiesta:
        // e' l'unico allegato che funziona anche da browser, dove il dialogo
        // di sistema non esiste e i percorsi del PC non si possono nominare.
        "attach_image" => match parse::<PastedImageArgs>(body) {
            Ok(a) => result(crate::attach_image(a.name, a.media_type, a.data)),
            Err(e) => e,
        },

        "get_memory" => ok(json!(crate::get_memory())),
        "forget_fact" => match parse::<FactArgs>(body) {
            Ok(a) => result(crate::forget_fact(a.fact_id)),
            Err(e) => e,
        },
        "clear_memory" => result(crate::clear_memory()),

        "index_status" => ok(json!(crate::index_status())),
        // Indicizzare e' un lavoro lungo che legge i file dell'utente: si
        // avvia dal PC, come le altre cose che toccano il disco per intero.
        "build_index" | "clear_index" => fail(
            StatusCode::FORBIDDEN,
            "L'indice dei documenti si costruisce e si cancella dalla finestra di LOGICA sul PC.",
        ),

        "get_preferences" => ok(json!(crate::get_preferences())),
        "set_preferences" => match parse::<PreferencesArgs>(body) {
            Ok(a) => match keep_engine_settings(a.preferences) {
                Ok(p) => result(crate::set_preferences(p)),
                Err(e) => fail(StatusCode::FORBIDDEN, e),
            },
            Err(e) => e,
        },

        "get_config" => ok(json!(crate::get_config())),
        "set_config" => match parse::<ConfigArgs>(body) {
            Ok(a) => match keep_allowlist(a.config) {
                Ok(c) => result(crate::set_config(c)),
                Err(e) => fail(StatusCode::FORBIDDEN, e),
            },
            Err(e) => e,
        },

        /* ------------------------------ sistema --------------------------- */
        "check_ai_status" => match parse::<StatusArgs>(body) {
            // L'URL arriva dal client ma non viene usato: lo stato si chiede
            // sempre al motore configurato sul PC.
            Ok(_) => ok(json!(
                crate::check_ai_status(Some(crate::storage::load_preferences().ollama_url)).await
            )),
            Err(e) => e,
        },

        _ => fail(StatusCode::NOT_FOUND, format!("Comando sconosciuto: {name}")),
    }
}

/* -------------------------------- controlli ------------------------------- */

/// Il motore AI da usare e' sempre quello configurato sul PC.
///
/// Senza questo, il corpo della richiesta potrebbe indicare un `ollama_url`
/// qualsiasi: LOGICA spedirebbe li' la conversazione e i nomi dei file,
/// trasformando una sessione valida in una fuga di dati.
fn local_ai(config: AiConfig) -> AiConfig {
    within_preferences(config, &crate::storage::load_preferences())
}

/// Tutto cio' che apre una porta sui dati o verso l'esterno si decide dal PC.
///
/// `keep_engine_settings` impedisce di accendere questi permessi cambiando le
/// preferenze; qui si chiude l'altra strada, quella della singola richiesta:
/// senza, bastava mandare `file_tools: true` dentro `talk` per far leggere i
/// file al modello, o `use_premium_ai: true` per spedire la conversazione
/// all'API di Anthropic da un PC che usa solo Ollama.
///
/// Da browser un permesso si puo' spegnere, mai accendere: la scelta per
/// singola chat fra Claude e Ollama resta, ma solo se Claude e' gia' acceso
/// sul PC.
fn within_preferences(mut config: AiConfig, prefs: &Preferences) -> AiConfig {
    config.ollama_url = prefs.ollama_url.clone();
    // Anche l'accesso a internet si decide dal PC: da browser non lo si accende.
    config.web_search = prefs.web_search;
    config.file_tools &= prefs.file_tools;
    config.documents &= prefs.documents;
    config.use_premium_ai &= prefs.use_premium_ai;
    config.memory &= prefs.memory;
    config
}

/// Le cartelle autorizzate si cambiano solo dal PC: sono il recinto, e un
/// recinto che il recintato puo' spostare non serve a niente.
fn keep_allowlist(incoming: Config) -> Result<Config, String> {
    let current = crate::storage::load_config();
    if incoming.monitored_folders != current.monitored_folders {
        return Err(
            "Le cartelle autorizzate si modificano solo dalla finestra di LOGICA sul PC."
                .to_string(),
        );
    }
    Ok(incoming)
}

/// Stessa ragione per l'indirizzo del motore AI: cambiarlo da remoto vorrebbe
/// dire scegliere dove finiscono i contenuti delle conversazioni.
///
/// E stessa ragione per gli strumenti sui file: il recinto non si sposta da
/// remoto, ma accendere da soli la lettura dentro il recinto sarebbe il modo
/// di aggirarlo lo stesso, una risposta alla volta.
fn keep_engine_settings(incoming: Preferences) -> Result<Preferences, String> {
    let current = crate::storage::load_preferences();
    if incoming.ollama_url != current.ollama_url {
        return Err(
            "L'indirizzo del motore AI si modifica solo dalla finestra di LOGICA sul PC."
                .to_string(),
        );
    }
    if incoming.file_tools != current.file_tools {
        return Err(
            "L'accesso ai file si accende e si spegne solo dalla finestra di LOGICA sul PC."
                .to_string(),
        );
    }
    if incoming.documents != current.documents {
        return Err(
            "La ricerca nei documenti si accende e si spegne solo dalla finestra di LOGICA sul PC."
                .to_string(),
        );
    }
    Ok(incoming)
}

/* --------------------------------- statici -------------------------------- */

/// L'interfaccia, presa dagli stessi file incorporati nell'eseguibile che usa
/// la finestra desktop.
async fn assets(State(ctx): State<Arc<Ctx>>, uri: Uri) -> Response {

    let path = uri.path();
    let wanted = if path == "/" { "/index.html" } else { path };
    let resolver = ctx.app.asset_resolver();

    // Una rotta del frontend (`/chat/123`) non e' un file: si serve l'index e
    // ci pensa il router lato browser.
    let asset = resolver.get(wanted.to_string()).or_else(|| resolver.get("/index.html".to_string()));

    match asset {
        Some(a) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, a.mime_type)],
            a.bytes,
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "Interfaccia non disponibile. In sviluppo il frontend gira su Vite: l'accesso da \
             browser funziona nella versione compilata dell'app.",
        )
            .into_response(),
    }
}

/* --------------------------------- aiutini -------------------------------- */

// L'errore e' una `Response` gia' pronta, che e' grande. Boxarla vorrebbe dire
// scartarla subito dopo in ogni punto di chiamata: qui l'errore risale di un
// livello solo, e finisce nel corpo della risposta HTTP.
#[allow(clippy::result_large_err)]
fn parse<T: for<'de> Deserialize<'de>>(body: Value) -> Result<T, Response> {
    serde_json::from_value(body)
        .map_err(|e| fail(StatusCode::BAD_REQUEST, format!("Parametri non validi: {e}")))
}

fn ok<T: Serialize>(value: T) -> Response {
    Json(value).into_response()
}

/// I comandi tornano `Result<T, String>` e il frontend si aspetta lo stesso
/// errore che riceverebbe da Tauri.
fn result<T: Serialize>(r: Result<T, String>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => fail(StatusCode::BAD_REQUEST, e),
    }
}

fn fail(code: StatusCode, message: impl Into<String>) -> Response {
    (code, Json(json!({ "error": message.into() }))).into_response()
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).expect("nome header"),
                v.parse().expect("valore header"),
            );
        }
        h
    }

    #[test]
    fn legge_il_cookie_fra_gli_altri() {
        let h = headers(&[("cookie", "tema=scuro; logica_session=abc123; altro=1")]);
        assert_eq!(cookie_value(&h, COOKIE).as_deref(), Some("abc123"));
        assert_eq!(cookie_value(&h, "assente"), None);
    }

    /// La richiesta chiede tutto acceso, il PC ha tutto spento: vince il PC.
    #[test]
    fn da_remoto_i_permessi_non_si_accendono() {
        let prefs = Preferences {
            use_premium_ai: false,
            web_search: false,
            file_tools: false,
            memory: false,
            documents: false,
            ..Preferences::default()
        };
        let chiesta = AiConfig {
            use_premium_ai: true,
            ollama_url: "http://altrove.example:11434".to_string(),
            web_search: true,
            file_tools: true,
            memory: true,
            documents: true,
            ..AiConfig::default()
        };

        let usata = within_preferences(chiesta, &prefs);
        assert!(!usata.use_premium_ai);
        assert!(!usata.web_search);
        assert!(!usata.file_tools);
        assert!(!usata.memory);
        assert!(!usata.documents);
        assert_eq!(usata.ollama_url, prefs.ollama_url);
    }

    /// Spegnere da browser resta possibile: e' la scelta di Ollama per una
    /// singola chat quando sul PC e' acceso Claude.
    #[test]
    fn da_remoto_i_permessi_si_possono_spegnere() {
        let prefs = Preferences {
            use_premium_ai: true,
            file_tools: true,
            documents: true,
            ..Preferences::default()
        };
        let chiesta = AiConfig {
            use_premium_ai: false,
            file_tools: false,
            documents: false,
            ..AiConfig::default()
        };

        let usata = within_preferences(chiesta, &prefs);
        assert!(!usata.use_premium_ai);
        assert!(!usata.file_tools);
        assert!(!usata.documents);
    }

    #[test]
    fn origine_diversa_dall_host_rifiutata() {
        let buona = headers(&[("origin", "https://pc-casa:8443"), ("host", "pc-casa:8443")]);
        assert!(check_origin(&buona).is_ok());

        let cattiva = headers(&[("origin", "https://sito-ostile.example"), ("host", "pc-casa:8443")]);
        assert!(check_origin(&cattiva).is_err());

        // Senza origine non si scrive: e' il caso delle richieste costruite a
        // mano da un altro contesto.
        let assente = headers(&[("host", "pc-casa:8443")]);
        assert!(check_origin(&assente).is_err());
    }
}
