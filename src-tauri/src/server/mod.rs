//! Accesso da browser, da un altro PC della rete di casa.
//!
//! Il server vive dentro il processo di LOGICA: un solo proprietario di
//! `~/.logica`, nessuna scrittura concorrente sulle chat. Si accende e si
//! spegne dalle impostazioni, e da spento non apre nessuna porta.
//!
//! Cosa protegge cosa:
//!
//! - `tls`   cifra il traffico, cosi' password e cookie non passano in chiaro
//! - `auth`  stabilisce *chi* sta chiedendo, e rallenta chi tenta a caso
//! - `guard` stabilisce *cosa* puo' toccare, e vale anche a sessione valida
//! - `api`   traduce le richieste in chiamate, applicando i due strati sopra

pub mod api;
pub mod auth;
pub mod guard;
pub mod tls;

#[cfg(test)]
mod e2e;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Mutex, Once};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::broadcast;

use api::Envelope;

/// Quanti eventi di avanzamento restano in coda per un browser lento prima che
/// i piu' vecchi vengano scartati.
pub const EVENT_CAPACITY: usize = 256;

/// Il provider crittografico va installato una volta sola per processo.
static CRYPTO: Once = Once::new();

/// Lo stato del server, come lo vede la finestra delle impostazioni.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub running: bool,
    /// Indirizzo da digitare sull'altro PC, quando il server e' acceso.
    pub url: Option<String>,
    /// Impronta SHA-256 del certificato, da confrontare con quella che mostra
    /// il browser la prima volta che accetta l'avviso.
    pub fingerprint: Option<String>,
    pub port: u16,
    pub expose_to_lan: bool,
    pub autostart: bool,
    pub has_password: bool,
    /// Le cartelle entro cui il browser puo' lavorare. Vuota = da remoto non
    /// si puo' toccare niente.
    pub allowed_folders: Vec<String>,
}

struct Running {
    handle: axum_server::Handle,
    url: String,
    fingerprint: String,
}

/// Il server acceso, se c'e'. Vive dentro lo stato dell'app Tauri.
#[derive(Default)]
pub struct ServerControl {
    running: Mutex<Option<Running>>,
}

impl ServerControl {
    pub fn status(&self) -> ServerStatus {
        let settings = auth::load_settings();
        let running = self.running.lock().unwrap();
        ServerStatus {
            running: running.is_some(),
            url: running.as_ref().map(|r| r.url.clone()),
            fingerprint: running.as_ref().map(|r| r.fingerprint.clone()),
            port: settings.port,
            expose_to_lan: settings.expose_to_lan,
            autostart: settings.autostart,
            has_password: settings.password_hash.is_some(),
            allowed_folders: crate::storage::load_config().monitored_folders,
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.lock().unwrap().is_some()
    }

    /// Spegne il server. Le sessioni aperte muoiono con lui: chi era collegato
    /// deve rifare il login alla riaccensione.
    pub fn stop(&self) {
        if let Some(r) = self.running.lock().unwrap().take() {
            r.handle.graceful_shutdown(Some(Duration::from_secs(2)));
        }
    }
}

/// Accende il server e ritorna lo stato aggiornato.
pub async fn start(
    app: tauri::AppHandle,
    events: broadcast::Sender<Envelope>,
) -> Result<ServerStatus, String> {
    use tauri::Manager;

    let control_running = {
        let state: tauri::State<crate::AppState> = app.state();
        state.server.is_running()
    };
    if control_running {
        let state: tauri::State<crate::AppState> = app.state();
        return Ok(state.server.status());
    }

    let settings = auth::load_settings();
    let password_hash = settings.password_hash.clone().ok_or_else(|| {
        "Imposta prima una password per l'accesso da browser: senza, il server non parte."
            .to_string()
    })?;

    CRYPTO.call_once(|| {
        // Backend `ring`: Rust puro, nessun compilatore C richiesto.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });

    let material = tls::load_or_create()?;
    let tls_config =
        axum_server::tls_rustls::RustlsConfig::from_pem(material.cert_pem, material.key_pem)
            .await
            .map_err(|e| format!("certificato non caricabile: {e}"))?;

    // Su 127.0.0.1 la porta esiste solo per questo PC. Su 0.0.0.0 la vede
    // tutta la rete locale: e' la scelta che l'utente fa consapevolmente
    // dalle impostazioni.
    let bind_ip = if settings.expose_to_lan {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    };
    let addr = SocketAddr::new(bind_ip, settings.port);

    let ctx = std::sync::Arc::new(api::Ctx {
        app: app.clone(),
        guardian: std::sync::Arc::new(api::Guardian {
            sessions: auth::Sessions::default(),
            throttle: auth::Throttle::default(),
            password_hash,
        }),
        events,
    });
    let router = api::router(ctx);

    let handle = axum_server::Handle::new();
    let server_handle = handle.clone();
    tokio::spawn(async move {
        let service = router.into_make_service_with_connect_info::<SocketAddr>();
        if let Err(e) = axum_server::bind_rustls(addr, tls_config)
            .handle(server_handle)
            .serve(service)
            .await
        {
            eprintln!("[logica] server web fermato: {e}");
        }
    });

    // Se la porta e' occupata l'errore arriva qui, non a `tokio::spawn`.
    handle
        .listening()
        .await
        .ok_or_else(|| format!("Porta {} non disponibile: scegline un'altra.", settings.port))?;

    let url = reachable_url(&settings);
    {
        let state: tauri::State<crate::AppState> = app.state();
        *state.server.running.lock().unwrap() =
            Some(Running { handle, url, fingerprint: material.fingerprint });
    }

    let state: tauri::State<crate::AppState> = app.state();
    Ok(state.server.status())
}

/// L'indirizzo da digitare sull'altro PC.
fn reachable_url(settings: &auth::ServerSettings) -> String {
    let port = settings.port;
    if settings.expose_to_lan {
        match tls::lan_address() {
            Some(ip) => format!("https://{ip}:{port}"),
            None => format!("https://<indirizzo-di-questo-pc>:{port}"),
        }
    } else {
        format!("https://127.0.0.1:{port}")
    }
}
