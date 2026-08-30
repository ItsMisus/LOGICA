//! Il riassunto della parte vecchia di una conversazione lunga.
//!
//! Al modello vanno gli ultimi turni e basta: oltre quella soglia i messaggi
//! sparivano muti, e in una chat lunga l'assistente si dimenticava di colpo
//! una decisione presa cento righe prima. Qui quella parte non sparisce, viene
//! condensata in poche righe che restano in testa al prompt.
//!
//! Il riassunto sta fuori dal file della chat apposta: quel file lo scrive il
//! frontend a ogni salvataggio, e due scrittori sullo stesso JSON si
//! sovrascriverebbero a vicenda.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Quanti turni si lasciano crescere oltre la finestra prima di rifare il
/// riassunto. Senza questo margine servirebbe una chiamata al modello a ogni
/// messaggio, per una conversazione lunga: i turni in eccesso vanno comunque
/// al modello per intero, quindi non si perde niente ad aspettare.
pub const REFRESH_EVERY: usize = 8;

/// Caratteri tenuti del riassunto. Piu' lungo di cosi' non condensa piu'
/// niente: tanto varrebbe mandare i messaggi veri.
pub const MAX_SUMMARY_CHARS: usize = 2_000;

/// Il riassunto di una chat e fin dove arriva.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Digest {
    /// Quanti turni, dall'inizio della conversazione, questo riassunto copre.
    /// I turni successivi vanno al modello per intero.
    pub covered: usize,
    pub text: String,
}

type Store = HashMap<String, Digest>;

fn path() -> Result<PathBuf, String> {
    Ok(crate::storage::logica_dir()?.join("summaries.json"))
}

fn load_all() -> Store {
    let Ok(file) = path() else { return Store::new() };
    let Ok(data) = std::fs::read_to_string(&file) else { return Store::new() };
    serde_json::from_str(&data).unwrap_or_default()
}

fn save_all(store: &Store) -> Result<(), String> {
    let file = path()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Impossibile creare {}: {e}", parent.display()))?;
    }
    let data = serde_json::to_string_pretty(store)
        .map_err(|e| format!("Serializzazione dei riassunti fallita: {e}"))?;
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("Scrittura fallita su {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &file)
        .map_err(|e| format!("Salvataggio fallito su {}: {e}", file.display()))
}

pub fn load(chat_id: &str) -> Option<Digest> {
    load_all().remove(chat_id)
}

pub fn save(chat_id: &str, digest: &Digest) -> Result<(), String> {
    let mut store = load_all();
    store.insert(chat_id.to_string(), digest.clone());
    save_all(&store)
}

/// Il riassunto di una chat cancellata non serve piu' a nessuno.
pub fn forget(chat_id: &str) {
    let mut store = load_all();
    if store.remove(chat_id).is_some() {
        // Un riassunto orfano non fa danni: se non si riesce a scrivere, la
        // cancellazione della chat non deve fallire per questo.
        let _ = save_all(&store);
    }
}

/// Serve rifare il riassunto? Solo quando i turni non ancora condensati sono
/// cresciuti oltre la finestra piu' il margine.
pub fn needs_refresh(total_turns: usize, covered: usize, window: usize) -> bool {
    total_turns.saturating_sub(covered) > window + REFRESH_EVERY
}

/// Fin dove deve arrivare il riassunto nuovo: tutto tranne l'ultima finestra,
/// che al modello va per intero.
pub fn target(total_turns: usize, window: usize) -> usize {
    total_turns.saturating_sub(window)
}

/// Il pezzo di prompt col riassunto.
pub fn prompt_block(text: &str) -> String {
    format!(
        "PRIMA PARTE DELLA CONVERSAZIONE (riassunto)\nI messaggi qui sotto non ci sono piu' per intero: questo e' il riassunto di com'e' cominciata. Trattalo come cose gia' dette fra te e l'utente, non come materiale da citare.\n{text}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_chat_is_never_condensed() {
        assert!(!needs_refresh(10, 0, 30));
        assert!(!needs_refresh(30, 0, 30));
    }

    #[test]
    fn the_margin_keeps_the_model_from_being_called_every_message() {
        // Oltre la finestra ma dentro il margine: si aspetta.
        assert!(!needs_refresh(35, 0, 30));
        assert!(!needs_refresh(38, 0, 30));
        assert!(needs_refresh(39, 0, 30));

        // Riassunto gia' fatto: il conto riparte da li'.
        assert!(!needs_refresh(60, 30, 30));
        assert!(needs_refresh(69, 30, 30));
    }

    #[test]
    fn the_last_window_always_stays_whole() {
        assert_eq!(target(45, 30), 15);
        assert_eq!(target(20, 30), 0);
    }
}
