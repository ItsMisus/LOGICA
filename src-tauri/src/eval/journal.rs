//! Il giornale: ogni esito su disco appena arriva.
//!
//! La prima misura intera e' stata fermata a trentasette richieste su cento e
//! non ha lasciato niente, perche' il rapporto si scriveva solo alla fine. Un
//! lavoro che dura un'ora e mezza va considerato come una cosa che verra'
//! interrotta, non come una cosa che finira': il computer si riavvia, qualcuno
//! preme Ctrl-C, la rete cade a meta' delle richieste web.
//!
//! Una riga per esito, in JSON. Il formato a righe non e' pigrizia: un file
//! JSON unico andrebbe riscritto intero a ogni aggiunta, e un'interruzione a
//! meta' della riscrittura lascerebbe un file rotto — cioe' perderebbe tutto
//! proprio nel momento per cui il giornale esiste. Una riga in fondo o c'e' o
//! non c'e', e una riga scritta a meta' si butta senza toccare le altre.

use std::io::Write;
use std::path::PathBuf;

use super::Outcome;

/// Una riga del giornale: l'esito, piu' con quale modello e' stato ottenuto.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub model: String,
    #[serde(flatten)]
    pub outcome: Outcome,
}

pub fn path() -> PathBuf {
    std::env::temp_dir().join("logica_eval_giornale.jsonl")
}

/// Butta via il giornale e ricomincia da capo.
pub fn reset() {
    let _ = std::fs::remove_file(path());
}

/// Aggiunge un esito in fondo.
///
/// Apre, scrive e chiude a ogni riga invece di tenere il file aperto: cosi' il
/// contenuto e' sul disco quando la riga e' finita, e non nella memoria di un
/// processo che sta per essere ucciso.
pub fn append(model: &str, outcome: &Outcome) -> Result<(), String> {
    let entry = Entry { model: model.to_string(), outcome: outcome.clone() };
    let line = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path())
        .map_err(|e| format!("giornale non apribile: {e}"))?;
    writeln!(file, "{line}").map_err(|e| format!("giornale non scrivibile: {e}"))
}

/// Quello che c'e' gia'.
///
/// Le righe illeggibili si saltano invece di far fallire tutto: l'unica riga
/// che puo' essere rotta e' l'ultima, quella che stava uscendo quando il
/// processo e' morto, e perdere quella non e' un buon motivo per perdere le
/// novantanove prima.
pub fn load() -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(path()) else { return Vec::new() };
    text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect()
}

/// Quali richieste sono gia' state fatte, per non rifarle.
pub fn already_done(entries: &[Entry]) -> Vec<(String, String)> {
    entries.iter().map(|e| (e.model.clone(), e.outcome.id.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::cases::Category;

    fn outcome(id: &str) -> Outcome {
        Outcome {
            id: id.to_string(),
            category: Category::Chat,
            prompt: "domanda".to_string(),
            answer: "risposta".to_string(),
            correct: true,
            tool_ok: None,
            tool_calls: 0,
            retrieval_ok: None,
            web_ok: None,
            language_ok: true,
            hallucinated: false,
            seconds: 1.5,
            failure: None,
        }
    }

    #[test]
    fn what_was_written_can_be_read_back() {
        let _home = crate::storage::tests::TempHome::new("giornale");
        reset();
        append("modello", &outcome("chat-01")).expect("scrittura");
        append("modello", &outcome("chat-02")).expect("scrittura");

        let entries = load();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].outcome.id, "chat-01");
        assert_eq!(entries[1].model, "modello");
        assert!((entries[0].outcome.seconds - 1.5).abs() < 1e-6);
        reset();
    }

    /// Il caso per cui il giornale esiste: il processo muore mentre scrive.
    /// L'ultima riga resta a meta', e le altre devono sopravvivere.
    #[test]
    fn a_half_written_line_does_not_take_the_others_down() {
        let _home = crate::storage::tests::TempHome::new("giornale_rotto");
        reset();
        append("modello", &outcome("chat-01")).expect("scrittura");
        let mut file = std::fs::OpenOptions::new().append(true).open(path()).expect("apertura");
        write!(file, "{{\"model\":\"modello\",\"id\":\"chat-0").expect("riga a meta'");
        drop(file);

        let entries = load();
        assert_eq!(entries.len(), 1, "la riga rotta si e' portata via anche quella buona");
        assert_eq!(entries[0].outcome.id, "chat-01");
        reset();
    }

    #[test]
    fn a_journal_that_is_not_there_is_empty_not_an_error() {
        let _home = crate::storage::tests::TempHome::new("giornale_assente");
        reset();
        assert!(load().is_empty());
    }

    #[test]
    fn what_is_done_is_known_by_model_and_case() {
        let done = already_done(&[
            Entry { model: "a".to_string(), outcome: outcome("chat-01") },
            Entry { model: "b".to_string(), outcome: outcome("chat-01") },
        ]);
        // Lo stesso caso su due modelli sono due richieste, non una.
        assert_eq!(done.len(), 2);
        assert!(done.contains(&("a".to_string(), "chat-01".to_string())));
        assert!(done.contains(&("b".to_string(), "chat-01".to_string())));
    }
}
