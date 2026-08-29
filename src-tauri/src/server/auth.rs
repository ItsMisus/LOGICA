//! Password, sessioni e freno ai tentativi di accesso.
//!
//! Un solo utente: tu. La password non viene mai salvata, solo il suo hash
//! Argon2id. Le sessioni vivono in memoria e muoiono con il processo: se
//! chiudi LOGICA, ogni browser collegato deve rifare il login.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use argon2::password_hash::rand_core::OsRng as ArgonRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

/// Sotto questa lunghezza la password non viene accettata. Argon2 rende lento
/// il singolo tentativo, ma una password corta resta corta.
pub const MIN_PASSWORD_LEN: usize = 12;

/// Sessione ferma da piu' di cosi': scaduta.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Durata massima comunque, anche con la scheda aperta e attiva.
const ABSOLUTE_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);

/// Tentativi falliti tollerati prima del blocco.
const MAX_FAILURES: u32 = 5;

/// Quanto dura il blocco, e la finestra entro cui i fallimenti si sommano.
const LOCKOUT: Duration = Duration::from_secs(15 * 60);

/* ------------------------------ impostazioni ------------------------------ */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSettings {
    /// Il server parte da solo all'avvio dell'app.
    #[serde(default)]
    pub autostart: bool,
    #[serde(default = "default_port")]
    pub port: u16,
    /// `false` = raggiungibile solo da questo PC (127.0.0.1).
    /// `true` = raggiungibile dalla rete locale.
    #[serde(default)]
    pub expose_to_lan: bool,
    /// Hash Argon2id. Finche' e' `None` il server non puo' partire.
    #[serde(default)]
    pub password_hash: Option<String>,
}

fn default_port() -> u16 {
    8443
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self { autostart: false, port: default_port(), expose_to_lan: false, password_hash: None }
    }
}

fn settings_path() -> Result<std::path::PathBuf, String> {
    Ok(crate::storage::logica_dir()?.join("server.json"))
}

pub fn load_settings() -> ServerSettings {
    settings_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_settings(s: &ServerSettings) -> Result<(), String> {
    let path = settings_path()?;
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("scrittura di {}: {e}", path.display()))?;
    restrict_to_owner(&path);
    Ok(())
}

/// Toglie il file dall'ereditarieta' dei permessi e lascia solo l'utente
/// corrente. Contiene l'hash della password: non deve essere leggibile dagli
/// altri account della macchina.
#[cfg(windows)]
fn restrict_to_owner(path: &std::path::Path) {
    let target = path.display().to_string();
    let user = std::env::var("USERNAME").unwrap_or_default();
    if user.is_empty() {
        return;
    }
    let _ = std::process::Command::new("icacls")
        .args([&target, "/inheritance:r", "/grant:r", &format!("{user}:(F)")])
        .output();
}

#[cfg(not(windows))]
fn restrict_to_owner(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

/* -------------------------------- password -------------------------------- */

pub fn hash_password(plain: &str) -> Result<String, String> {
    if plain.chars().count() < MIN_PASSWORD_LEN {
        return Err(format!("La password deve avere almeno {MIN_PASSWORD_LEN} caratteri."));
    }
    let salt = SaltString::generate(&mut ArgonRng);
    Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("hashing della password: {e}"))
}

/// Argon2 confronta in tempo costante e impiega comunque decine di
/// millisecondi: e' il freno principale contro chi prova password a raffica.
pub fn verify_password(plain: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default().verify_password(plain.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

/* -------------------------------- sessioni -------------------------------- */

#[derive(Debug, Clone)]
pub struct Session {
    /// Token che il frontend rimanda nell'header `X-Logica-Csrf`. Il cookie da
    /// solo non basta: un sito ostile puo' far partire una richiesta dal tuo
    /// browser, ma non puo' leggere questo valore per metterlo nell'header.
    pub csrf: String,
    created: Instant,
    last_seen: Instant,
    /// La sessione vale solo dall'indirizzo che ha fatto il login.
    ip: IpAddr,
}

#[derive(Default)]
pub struct Sessions {
    map: Mutex<HashMap<String, Session>>,
}

impl Sessions {
    /// Torna (token di sessione, token csrf).
    pub fn open(&self, ip: IpAddr) -> (String, String) {
        let token = random_token();
        let csrf = random_token();
        let now = Instant::now();
        let mut map = self.map.lock().unwrap();
        prune(&mut map);
        map.insert(token.clone(), Session { csrf: csrf.clone(), created: now, last_seen: now, ip });
        (token, csrf)
    }

    /// Valida il cookie e rinfresca l'ultimo accesso. `None` se la sessione non
    /// esiste, e' scaduta o arriva da un altro indirizzo.
    pub fn touch(&self, token: &str, ip: IpAddr) -> Option<Session> {
        let mut map = self.map.lock().unwrap();
        prune(&mut map);
        let session = map.get_mut(token)?;
        if session.ip != ip {
            return None;
        }
        session.last_seen = Instant::now();
        Some(session.clone())
    }

    pub fn close(&self, token: &str) {
        self.map.lock().unwrap().remove(token);
    }
}

fn prune(map: &mut HashMap<String, Session>) {
    let now = Instant::now();
    map.retain(|_, s| {
        now.duration_since(s.last_seen) < IDLE_TIMEOUT
            && now.duration_since(s.created) < ABSOLUTE_TIMEOUT
    });
}

/// Confronto del token CSRF in tempo costante: la differenza di tempo fra un
/// token sbagliato al primo carattere e uno sbagliato all'ultimo direbbe a chi
/// misura quanto si e' avvicinato.
pub fn csrf_matches(expected: &str, given: &str) -> bool {
    expected.as_bytes().ct_eq(given.as_bytes()).into()
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/* -------------------------------- throttle -------------------------------- */

#[derive(Debug, Clone, Copy)]
struct Attempts {
    count: u32,
    first: Instant,
}

/// Contatore dei login falliti per indirizzo.
#[derive(Default)]
pub struct Throttle {
    map: Mutex<HashMap<IpAddr, Attempts>>,
}

impl Throttle {
    /// `Some(secondi)` se l'indirizzo e' bloccato, e per quanto ancora.
    pub fn locked_for(&self, ip: IpAddr) -> Option<u64> {
        let map = self.map.lock().unwrap();
        let a = map.get(&ip)?;
        let elapsed = a.first.elapsed();
        if a.count >= MAX_FAILURES && elapsed < LOCKOUT {
            Some((LOCKOUT - elapsed).as_secs().max(1))
        } else {
            None
        }
    }

    pub fn record_failure(&self, ip: IpAddr) {
        let mut map = self.map.lock().unwrap();
        let now = Instant::now();
        map.entry(ip)
            .and_modify(|a| {
                // Finestra scaduta: si riparte da capo invece di bloccare per
                // fallimenti sparsi in giorni diversi.
                if a.first.elapsed() >= LOCKOUT {
                    *a = Attempts { count: 1, first: now };
                } else {
                    a.count += 1;
                }
            })
            .or_insert(Attempts { count: 1, first: now });
    }

    pub fn record_success(&self, ip: IpAddr) {
        self.map.lock().unwrap().remove(&ip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip() -> IpAddr {
        "192.168.1.20".parse().unwrap()
    }

    #[test]
    fn password_troppo_corta_rifiutata() {
        assert!(hash_password("corta").is_err());
    }

    #[test]
    fn hash_verifica_solo_la_password_giusta() {
        let h = hash_password("password-lunga-abbastanza").expect("hash");
        assert!(verify_password("password-lunga-abbastanza", &h));
        assert!(!verify_password("password-lunga-abbastanzb", &h));
    }

    /// Una sessione rubata e riusata da un altro indirizzo non deve valere.
    #[test]
    fn la_sessione_e_legata_all_indirizzo() {
        let s = Sessions::default();
        let (token, _) = s.open(ip());
        assert!(s.touch(&token, ip()).is_some());
        assert!(s.touch(&token, "10.0.0.9".parse().unwrap()).is_none());
    }

    #[test]
    fn il_logout_invalida_subito() {
        let s = Sessions::default();
        let (token, _) = s.open(ip());
        s.close(&token);
        assert!(s.touch(&token, ip()).is_none());
    }

    #[test]
    fn blocco_dopo_troppi_fallimenti() {
        let t = Throttle::default();
        for _ in 0..MAX_FAILURES - 1 {
            t.record_failure(ip());
        }
        assert!(t.locked_for(ip()).is_none());
        t.record_failure(ip());
        assert!(t.locked_for(ip()).is_some());
        // Un login riuscito (o l'attesa) azzera il contatore.
        t.record_success(ip());
        assert!(t.locked_for(ip()).is_none());
    }
}
