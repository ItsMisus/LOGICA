//! Quello che l'assistente si ricorda da una conversazione all'altra.
//!
//! Lo storico che va al modello e' quello della chat corrente e basta: aprire
//! una chat nuova, fino a qui, voleva dire ripresentarsi da capo. Qui restano
//! poche righe — chi e' l'utente, cosa usa, a cosa sta lavorando — scritte dal
//! modello stesso alla fine di uno scambio e rimesse nel prompt di sistema di
//! tutte le chat successive.
//!
//! Volutamente *non* e' un archivio di tutto: sono fatti duraturi, corti e
//! contati. Una memoria che cresce senza limite finisce per mangiarsi la
//! finestra di contesto che dovrebbe servire alla domanda.

use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Fatti tenuti in tutto. Oltre, i piu' vecchi escono: la memoria utile e'
/// quella recente, e un prompt di sistema lungo peggiora le risposte dei
/// modelli piccoli invece di migliorarle.
const MAX_FACTS: usize = 60;

/// Fatti nuovi accettati da un solo scambio. Un modello che ne propone dieci
/// ha capito male il compito e sta riassumendo la conversazione.
const MAX_NEW_FACTS: usize = 3;

/// Un fatto piu' lungo di cosi' non e' un fatto: e' un riassunto.
const MAX_FACT_CHARS: usize = 200;

/// Una cosa che l'assistente sa dell'utente e che vale oltre la chat in cui
/// l'ha imparata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fact {
    pub id: String,
    pub text: String,
    /// Quando e' stato imparato, in RFC 3339.
    pub created: String,
    /// La conversazione da cui viene, per poterci tornare.
    #[serde(default)]
    pub chat_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Memory {
    #[serde(default)]
    pub facts: Vec<Fact>,
}

fn path() -> Result<PathBuf, String> {
    Ok(crate::storage::logica_dir()?.join("memory.json"))
}

/// La memoria salvata. Un file assente o illeggibile vale memoria vuota: non
/// ricordarsi niente e' molto meglio che rifiutare di rispondere.
pub fn load() -> Memory {
    let Ok(file) = path() else { return Memory::default() };
    let Ok(data) = std::fs::read_to_string(&file) else { return Memory::default() };
    serde_json::from_str(&data).unwrap_or_default()
}

pub fn save(memory: &Memory) -> Result<(), String> {
    let file = path()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Impossibile creare {}: {e}", parent.display()))?;
    }
    let data = serde_json::to_string_pretty(memory)
        .map_err(|e| format!("Serializzazione della memoria fallita: {e}"))?;
    // Stessa cautela delle chat: prima il temporaneo, poi il rename.
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("Scrittura fallita su {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &file)
        .map_err(|e| format!("Salvataggio fallito su {}: {e}", file.display()))
}

pub fn list() -> Vec<Fact> {
    load().facts
}

pub fn forget(id: &str) -> Result<(), String> {
    let mut memory = load();
    memory.facts.retain(|f| f.id != id);
    save(&memory)
}

pub fn clear() -> Result<(), String> {
    save(&Memory::default())
}

/// Aggiunge i fatti nuovi, saltando quelli che ci sono gia', e torna quanti
/// ne sono entrati davvero.
pub fn remember(new_facts: &[String], chat_id: &str) -> Result<usize, String> {
    let mut memory = load();
    let mut added = 0;

    for text in new_facts.iter().take(MAX_NEW_FACTS) {
        let text = text.trim();
        if text.is_empty() || text.chars().count() > MAX_FACT_CHARS {
            continue;
        }
        if memory.facts.iter().any(|f| same_fact(&f.text, text)) {
            continue;
        }
        memory.facts.push(Fact {
            id: format!("{}_{}", Utc::now().format("%Y%m%d%H%M%S"), memory.facts.len()),
            text: text.to_string(),
            created: Utc::now().to_rfc3339(),
            chat_id: chat_id.to_string(),
        });
        added += 1;
    }

    if added == 0 {
        return Ok(0);
    }
    // I piu' vecchi escono per primi.
    if memory.facts.len() > MAX_FACTS {
        let excess = memory.facts.len() - MAX_FACTS;
        memory.facts.drain(..excess);
    }
    save(&memory)?;
    Ok(added)
}

/// Due fatti sono lo stesso quando uno contiene l'altro: il modello riscrive
/// volentieri la stessa cosa con una parola in piu' ("L'utente usa Rust" /
/// "L'utente usa Rust al lavoro"), e senza questo la memoria si riempirebbe di
/// varianti della stessa riga.
fn same_fact(a: &str, b: &str) -> bool {
    let a = normalize(a);
    let b = normalize(b);
    a == b || a.contains(&b) || b.contains(&a)
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Il pezzo di prompt con quello che si ricorda. `None` quando non c'e'
/// niente: una sezione vuota e' solo rumore nel contesto.
pub fn prompt_block() -> Option<String> {
    let facts = list();
    if facts.is_empty() {
        return None;
    }
    let lines: Vec<String> = facts.iter().map(|f| format!("- {}", f.text)).collect();
    Some(format!(
        "QUELLO CHE SAI GIA' DELL'UTENTE\nRaccolto nelle conversazioni precedenti. Usalo quando serve, senza annunciarlo e senza ripeterlo all'utente. Se qualcosa qui contraddice quello che l'utente dice adesso, vale quello che dice adesso.\n{}",
        lines.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeated_fact_does_not_enter_twice() {
        assert!(same_fact("L'utente usa Rust", "l'utente usa rust"));
        assert!(same_fact("L'utente usa Rust", "L'utente usa Rust al lavoro."));
        assert!(!same_fact("L'utente usa Rust", "L'utente usa Python"));
    }

    #[test]
    fn punctuation_and_spacing_do_not_make_a_new_fact() {
        assert_eq!(normalize("  L'utente   usa Rust!! "), "lutente usa rust");
    }

    #[test]
    fn an_empty_memory_adds_nothing_to_the_prompt() {
        let memory = Memory::default();
        assert!(memory.facts.is_empty());
    }
}
