//! Il controllo degli accessi visto da fuori, come lo vedrebbe un browser.
//!
//! Non si verifica che il codice faccia quello che dice, ma che una richiesta
//! costruita a mano venga fermata dove deve: senza sessione, con la sessione ma
//! senza token anti-CSRF, con tutto in regola ma da un'origine estranea.
//!
//! Il server di prova monta lo stesso middleware di quello vero su una rotta
//! finta. Sotto non c'e' Tauri: chi puo' entrare non dipende dall'app, ed e'
//! proprio per questo che si puo' provare cosi'.
//!
//! Il TLS non c'e': in prova si parla HTTP su una porta effimera. Cifratura e
//! controllo degli accessi sono strati indipendenti; qui si guarda il secondo.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::routing::post;
use axum::Router;

use crate::server::api::{login, require_session, Guardian};
use crate::server::auth;

const PASSWORD: &str = "password-di-prova-lunga";

struct Harness {
    base: String,
    client: reqwest::Client,
}

impl Harness {
    async fn start() -> Self {
        let guardian = Arc::new(Guardian {
            sessions: auth::Sessions::default(),
            throttle: auth::Throttle::default(),
            password_hash: auth::hash_password(PASSWORD).expect("hash"),
        });

        let protected = Router::new()
            .route("/prova", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(guardian.clone(), require_session));
        let public = Router::new().route("/api/login", post(login)).with_state(guardian);
        let router = protected.merge(public);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("indirizzo");
        tokio::spawn(async move {
            let service = router.into_make_service_with_connect_info::<SocketAddr>();
            let _ = axum::serve(listener, service).await;
        });

        Self {
            base: format!("http://{addr}"),
            client: reqwest::Client::builder().build().expect("client"),
        }
    }

    fn host(&self) -> String {
        self.base.trim_start_matches("http://").to_string()
    }

    async fn login(&self, password: &str) -> reqwest::Response {
        self.client
            .post(format!("{}/api/login", self.base))
            .header(reqwest::header::ORIGIN, self.base.clone())
            .header(reqwest::header::HOST, self.host())
            .json(&serde_json::json!({ "password": password }))
            .send()
            .await
            .expect("richiesta di login")
    }

    /// Credenziali complete: cookie, origine e token anti-CSRF.
    async fn entra(&self) -> (String, String) {
        let response = self.login(PASSWORD).await;
        assert_eq!(response.status(), 200);
        let cookie = session_cookie(&response);
        let csrf = response.json::<serde_json::Value>().await.expect("json")["csrf"]
            .as_str()
            .expect("token csrf")
            .to_string();
        (cookie, csrf)
    }
}

fn session_cookie(response: &reqwest::Response) -> String {
    let raw = response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("cookie di sessione")
        .to_str()
        .expect("cookie leggibile");
    raw.split(';').next().expect("valore del cookie").to_string()
}

#[tokio::test]
async fn senza_sessione_non_si_passa() {
    let h = Harness::start().await;

    let res = h
        .client
        .post(format!("{}/prova", h.base))
        .header(reqwest::header::ORIGIN, h.base.clone())
        .send()
        .await
        .expect("richiesta");

    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn password_sbagliata_respinta() {
    let h = Harness::start().await;

    assert_eq!(h.login("questa-non-e-la-password").await.status(), 401);
    assert_eq!(h.login(PASSWORD).await.status(), 200);
}

/// Il cookie arriva da solo in una richiesta partita da un altro sito: il
/// browser lo allega, ma il token anti-CSRF non puo' leggerlo nessuno.
#[tokio::test]
async fn il_cookie_senza_token_anti_csrf_non_basta() {
    let h = Harness::start().await;
    let (cookie, csrf) = h.entra().await;

    let senza_token = h
        .client
        .post(format!("{}/prova", h.base))
        .header(reqwest::header::COOKIE, cookie.clone())
        .header(reqwest::header::ORIGIN, h.base.clone())
        .header(reqwest::header::HOST, h.host())
        .send()
        .await
        .expect("richiesta");
    assert_eq!(senza_token.status(), 403);

    let con_token = h
        .client
        .post(format!("{}/prova", h.base))
        .header(reqwest::header::COOKIE, cookie)
        .header(reqwest::header::ORIGIN, h.base.clone())
        .header(reqwest::header::HOST, h.host())
        .header("x-logica-csrf", csrf)
        .send()
        .await
        .expect("richiesta");
    assert_eq!(con_token.status(), 200);
}

/// Origine di un altro sito: rifiutata anche con cookie e token validi.
#[tokio::test]
async fn origine_estranea_respinta() {
    let h = Harness::start().await;
    let (cookie, csrf) = h.entra().await;

    let res = h
        .client
        .post(format!("{}/prova", h.base))
        .header(reqwest::header::COOKIE, cookie)
        .header(reqwest::header::ORIGIN, "https://sito-ostile.example")
        .header(reqwest::header::HOST, h.host())
        .header("x-logica-csrf", csrf)
        .send()
        .await
        .expect("richiesta");

    assert_eq!(res.status(), 403);
}

/// Un token anti-CSRF valido ma di un'altra sessione non vale.
#[tokio::test]
async fn il_token_di_un_altra_sessione_non_vale() {
    let h = Harness::start().await;
    let (cookie, _) = h.entra().await;
    let (_, csrf_altrui) = h.entra().await;

    let res = h
        .client
        .post(format!("{}/prova", h.base))
        .header(reqwest::header::COOKIE, cookie)
        .header(reqwest::header::ORIGIN, h.base.clone())
        .header(reqwest::header::HOST, h.host())
        .header("x-logica-csrf", csrf_altrui)
        .send()
        .await
        .expect("richiesta");

    assert_eq!(res.status(), 403);
}

/// Dopo abbastanza tentativi falliti l'indirizzo viene bloccato, e da quel
/// momento nemmeno la password giusta passa.
#[tokio::test]
async fn troppi_tentativi_bloccano_l_indirizzo() {
    let h = Harness::start().await;

    for _ in 0..5 {
        assert_eq!(h.login("sbagliata").await.status(), 401);
    }

    let bloccato = h.login(PASSWORD).await;
    assert_eq!(bloccato.status(), 429);
}
