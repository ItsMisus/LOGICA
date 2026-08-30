//! Aggiornamento dell'app.
//!
//! L'aggiornatore di Tauri ha bisogno di due cose che non possono stare in un
//! repository: l'indirizzo da cui scaricare e la **chiave pubblica** con cui si
//! verifica la firma del pacchetto. La chiave privata corrispondente firma le
//! release e non deve esistere qui dentro in nessuna forma.
//!
//! Quindi non ci sono valori finti nel codice: i due dati arrivano
//! dall'ambiente al momento della compilazione (`option_env!`). Se mancano — e
//! mancano in ogni build fatta da chi clona il repository — il plugin non
//! viene nemmeno registrato e il comando lo dice, invece di provare a
//! scaricare da un indirizzo che non esiste o di verificare una firma con una
//! chiave inventata.
//!
//! ```powershell
//! $env:LOGICA_UPDATE_ENDPOINT = "https://esempio.it/logica/latest.json"
//! $env:LOGICA_UPDATE_PUBKEY   = "dW50cnVzdGVkIGNvbW1lbnQ6..."
//! npm run tauri build
//! ```

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

/// Dove sta il file che descrive l'ultima versione.
pub const ENDPOINT: Option<&str> = option_env!("LOGICA_UPDATE_ENDPOINT");

/// La chiave pubblica con cui si verifica la firma del pacchetto scaricato.
pub const PUBKEY: Option<&str> = option_env!("LOGICA_UPDATE_PUBKEY");

/// Cosa dire quando la build non e' stata preparata per aggiornarsi. Non e' un
/// errore: e' il caso normale di chi compila da se'.
const NOT_CONFIGURED: &str =
    "Questa copia di LOGICA non e' configurata per aggiornarsi da sola: aggiornala ricompilando, \
     oppure scarica la versione nuova dal posto da cui hai preso questa.";

/// L'aggiornatore serve solo se e' stato configurato: registrare il plugin
/// senza chiave vorrebbe dire tenere in piedi un pezzo che non puo' funzionare.
pub fn configured() -> bool {
    matches!((ENDPOINT, PUBKEY), (Some(e), Some(k)) if !e.trim().is_empty() && !k.trim().is_empty())
}

/// Cosa c'e' di nuovo, se c'e' qualcosa.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    /// `false` anche quando la build non e' configurata: chi guarda vuole
    /// sapere se c'e' un aggiornamento, e la ragione sta in `reason`.
    pub available: bool,
    pub current: String,
    /// La versione disponibile, quando ce n'e' una.
    pub version: Option<String>,
    /// Le note di rilascio, quando ci sono.
    pub notes: Option<String>,
    /// Perche' non si e' potuto controllare. `None` se il controllo e' andato.
    pub reason: Option<String>,
}

impl UpdateInfo {
    fn none(current: String, reason: Option<String>) -> Self {
        Self { available: false, current, version: None, notes: None, reason }
    }
}

/// L'aggiornatore con indirizzo e chiave di *questa* build. Non vengono da
/// `tauri.conf.json` apposta: quel file sta nel repository, e ci finirebbero
/// dentro un indirizzo e una chiave che valgono solo per chi pubblica.
fn build_updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    let endpoint = ENDPOINT.unwrap_or_default().trim();
    let url = endpoint
        .parse()
        .map_err(|e| format!("Indirizzo degli aggiornamenti non valido ({endpoint}): {e}"))?;
    app.updater_builder()
        .endpoints(vec![url])
        .map_err(|e| format!("Indirizzo degli aggiornamenti rifiutato: {e}"))?
        .pubkey(PUBKEY.unwrap_or_default().trim())
        .build()
        .map_err(|e| format!("Aggiornatore non pronto: {e}"))
}

fn current_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

/// C'e' una versione nuova?
pub async fn check(app: &AppHandle) -> UpdateInfo {
    let current = current_version(app);
    if !configured() {
        return UpdateInfo::none(current, Some(NOT_CONFIGURED.to_string()));
    }
    let updater = match build_updater(app) {
        Ok(u) => u,
        Err(e) => return UpdateInfo::none(current, Some(e)),
    };
    match updater.check().await {
        Ok(Some(update)) => UpdateInfo {
            available: true,
            current,
            version: Some(update.version.clone()),
            notes: update.body.clone(),
            reason: None,
        },
        Ok(None) => UpdateInfo::none(current, None),
        Err(e) => UpdateInfo::none(current, Some(format!("Controllo fallito: {e}"))),
    }
}

/// Scarica e installa, poi riavvia.
///
/// Il riavvio e' parte dell'operazione: su Windows l'installer sostituisce
/// l'eseguibile in uso, e lasciare aperta la finestra vecchia vorrebbe dire
/// tenere in memoria una versione che sul disco non c'e' piu'.
pub async fn install(app: &AppHandle, on_progress: impl Fn(u64, Option<u64>)) -> Result<(), String> {
    if !configured() {
        return Err(NOT_CONFIGURED.to_string());
    }
    let updater = build_updater(app)?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("Controllo fallito: {e}"))?
        .ok_or_else(|| "Nessun aggiornamento da installare.".to_string())?;

    let mut downloaded: u64 = 0;
    update
        .download_and_install(
            |chunk, total| {
                downloaded += chunk as u64;
                on_progress(downloaded, total);
            },
            || {},
        )
        .await
        .map_err(|e| format!("Aggiornamento non installato: {e}"))?;

    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una build fatta da chi clona il repository non ha ne' indirizzo ne'
    /// chiave: deve dirlo, non provarci.
    #[test]
    fn without_a_key_the_updater_stays_off() {
        if ENDPOINT.is_none() || PUBKEY.is_none() {
            assert!(!configured());
        }
    }

    #[test]
    fn the_message_says_what_to_do_instead() {
        assert!(NOT_CONFIGURED.contains("ricompilando"));
    }
}
