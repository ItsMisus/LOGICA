//! La traccia di una richiesta: cosa e' stato deciso, e in quanto tempo.
//!
//! Non serve a chi usa il programma, serve a chi lo scrive. Quando la misura
//! su cento richieste dice 8 su 15, la domanda successiva e' sempre la stessa
//! — il modello ha risposto male, o gli e' stato dato in mano lo strumento
//! sbagliato? — e finora la risposta si cercava a mano nei log, rileggendo
//! quali chiamate erano passate a schermo. Qui invece ogni richiesta lascia
//! scritto: che modello, quali interruttori, cosa ha deciso il vaglio, quali
//! strumenti ha chiesto, con che esito, e quanto ci ha messo in tutto.
//!
//! Spenta, non costa niente: nessuna riga viene formattata. Si accende con
//! `LOGICA_TRACE`:
//!
//! ```text
//! LOGICA_TRACE=1                       la traccia va su stderr
//! LOGICA_TRACE=C:\misure\traccia.log   la traccia va in fondo a quel file
//! ```
//!
//! Le righe si accumulano e si scrivono tutte insieme alla fine della
//! richiesta: due risposte in parallelo (la finestra e il browser) scrivono
//! ognuna il suo blocco invece di intrecciare le righe.

use std::io::Write;
use std::time::Instant;

/// Dove va la traccia, letto una volta sola.
enum Dove {
    Spenta,
    Stderr,
    File(std::path::PathBuf),
}

fn dove() -> Dove {
    match std::env::var("LOGICA_TRACE") {
        Err(_) => Dove::Spenta,
        Ok(v) => match v.trim() {
            "" | "0" | "no" => Dove::Spenta,
            "1" | "si" | "stderr" => Dove::Stderr,
            path => Dove::File(std::path::PathBuf::from(path)),
        },
    }
}

/// Il taccuino di una richiesta. Quando la traccia e' spenta, `righe` resta
/// vuoto e ogni metodo esce subito.
pub struct Trace {
    acceso: bool,
    inizio: Instant,
    righe: Vec<String>,
}

impl Trace {
    pub fn start(titolo: &str) -> Self {
        let acceso = !matches!(dove(), Dove::Spenta);
        let mut trace = Trace { acceso, inizio: Instant::now(), righe: Vec::new() };
        if acceso {
            trace.righe.push(format!(
                "\n=== {} — {titolo}",
                chrono::Local::now().format("%H:%M:%S")
            ));
        }
        trace
    }

    /// Una riga, con quanti secondi erano passati quando e' successa.
    pub fn line(&mut self, testo: impl AsRef<str>) {
        if !self.acceso {
            return;
        }
        self.righe.push(format!(
            "  {:>6.1}s  {}",
            self.inizio.elapsed().as_secs_f32(),
            testo.as_ref()
        ));
    }

    /// Come `line`, ma per i blocchi lunghi (l'esito di uno strumento, la
    /// risposta finale): si taglia, perche' una traccia che contiene mezzo
    /// file non si legge.
    pub fn block(&mut self, etichetta: &str, testo: &str) {
        if !self.acceso {
            return;
        }
        let pulito = testo.trim().replace('\n', " ");
        let tagliato: String = pulito.chars().take(300).collect();
        let coda = if pulito.chars().count() > 300 { "..." } else { "" };
        self.line(format!("{etichetta}: {tagliato}{coda}"));
    }

    /// Chiude la traccia e la scrive tutta insieme.
    pub fn finish(mut self) {
        if !self.acceso {
            return;
        }
        self.righe.push(format!("  {:>6.1}s  fine", self.inizio.elapsed().as_secs_f32()));
        let testo = format!("{}\n", self.righe.join("\n"));
        match dove() {
            Dove::Spenta => {}
            Dove::Stderr => eprint!("{testo}"),
            Dove::File(path) => {
                let scritto = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .and_then(|mut f| f.write_all(testo.as_bytes()));
                // Una traccia che non si riesce a scrivere non deve fermare
                // la risposta: si dice dove e si va avanti.
                if let Err(e) = scritto {
                    eprintln!("[logica] traccia non scritta in {}: {e}", path.display());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spenta_non_raccoglie_niente() {
        // Il test non tocca la variabile d'ambiente: la lettura vale per tutto
        // il processo, e `cargo test` fa girare le prove insieme.
        let mut trace = Trace { acceso: false, inizio: Instant::now(), righe: Vec::new() };
        trace.line("qualcosa");
        trace.block("risposta", "un testo lungo");
        assert!(trace.righe.is_empty());
    }

    #[test]
    fn i_blocchi_lunghi_si_tagliano_e_stanno_su_una_riga() {
        let mut trace = Trace { acceso: true, inizio: Instant::now(), righe: Vec::new() };
        trace.block("esito", &format!("prima riga\nseconda riga{}", "x".repeat(400)));
        let riga = &trace.righe[0];
        assert!(!riga[2..].contains('\n'), "una riga sola");
        assert!(riga.ends_with("..."), "tagliata");
        assert!(riga.contains("prima riga seconda riga"));
    }
}
