//! Come si giudica una risposta.
//!
//! Sono confronti fra stringhe, e la cosa importante e' che si vedano: un
//! criterio scritto qui si puo' leggere, discutere e correggere, mentre un
//! giudizio dato da un altro modello sarebbe piu' morbido e non si potrebbe
//! guardare in faccia.

use std::time::Duration;

use super::cases::{Case, Category, Tools, Web};
use super::Outcome;
use crate::assistant::Reply;

/// Il testo contiene almeno una delle alternative del gruppo.
fn any_of(text: &str, group: &[&str]) -> bool {
    let low = text.to_lowercase();
    group.iter().any(|needle| low.contains(&needle.to_lowercase()))
}

/// Tutti i gruppi sono soddisfatti. Un gruppo vuoto non chiede niente.
fn all_groups(text: &str, groups: &[&[&str]]) -> bool {
    groups.iter().all(|g| any_of(text, g))
}

fn forbidden(text: &str, forbid: &[&str]) -> bool {
    !forbid.is_empty() && any_of(text, forbid)
}

/// Le estensioni con cui un modello nomina un file inventato. Chiusa apposta,
/// come quella del vaglio: una regola generica su "punto piu' tre lettere"
/// prenderebbe anche la fine di ogni frase.
const NAMED_EXTENSIONS: [&str; 9] =
    ["txt", "md", "pdf", "doc", "docx", "xls", "xlsx", "csv", "json"];

/// La risposta cita un file che non esiste.
///
/// E' l'unica cosa inventata che si puo' contare senza averla prevista: i nomi
/// dei file del corpus si sanno tutti, e un nome che non e' fra quelli — e che
/// non era nemmeno nella domanda — se l'e' inventato il modello. Il `forbid`
/// scritto caso per caso resta, ma da solo lascia passare tutto quello a cui
/// non aveva pensato chi ha scritto il caso.
/// Nomi che un modello usa come esempio, non come citazione. Quando rifiuta di
/// creare un file spiega come si farebbe ("echo prova > nuovofile.txt"): non
/// sta dicendo che quel file esiste, quindi non e' una cosa inventata.
const NOMI_ESEMPIO: [&str; 8] =
    ["nuovofile", "nuovo_file", "nomefile", "nome_file", "miofile", "esempio", "example", "prova"];

fn invented_file_names(text: &str, prompt: &str) -> bool {
    let known = super::fixtures::known_names();
    let asked = prompt.to_lowercase();
    text.to_lowercase()
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | ')' | ',' | ';' | '`' | '*' | '\\' | '/'))
        .filter_map(|word| {
            let word = word.trim_end_matches(['.', '!', '?', ':']);
            let (stem, ext) = word.rsplit_once('.')?;
            (!stem.is_empty()
                && NAMED_EXTENSIONS.contains(&ext)
                && !NOMI_ESEMPIO.contains(&stem))
            .then(|| word.to_string())
        })
        .any(|name| !known.iter().any(|k| k == &name) && !asked.contains(&name))
}

/// Parole che in italiano compaiono di continuo e in inglese mai, e viceversa.
/// Si contano come parole intere: "the" dentro "theme" non e' inglese, e "di"
/// dentro "dire" non e' italiano.
const ITALIANE: &[&str] = &[
    "il", "lo", "la", "i", "gli", "le", "di", "del", "della", "che", "non", "per", "con", "una",
    "un", "sono", "nel", "nella", "come", "piu'", "più", "e'", "è", "questo", "questa", "ho", "hai",
    "ma", "se", "anche", "dei", "alla", "al",
];
const INGLESI: &[&str] = &[
    "the", "and", "is", "are", "of", "to", "for", "with", "this", "that", "have", "was", "you",
    "it", "on", "as", "be", "from", "your",
];

fn count_words(text: &str, words: &[&str]) -> usize {
    let low = text.to_lowercase();
    low.split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}')
        .filter(|w| words.contains(w))
        .count()
}

/// La risposta e' in italiano.
///
/// Un conteggio, non un riconoscitore di lingue: le risposte da giudicare sono
/// frasi intere in una lingua sola, e per quel caso basta. Una risposta troppo
/// corta per contare parole ("391") si considera buona: un numero non ha lingua.
pub fn is_italian(text: &str) -> bool {
    let italiane = count_words(text, ITALIANE);
    let inglesi = count_words(text, INGLESI);
    if italiane == 0 && inglesi == 0 {
        return true;
    }
    italiane >= inglesi
}

/// Gli strumenti usati corrispondono a quello che il caso si aspettava.
fn tools_ok(expected: Tools, used: &[String]) -> Option<bool> {
    match expected {
        Tools::Ignora => None,
        Tools::Nessuno => Some(used.is_empty()),
        Tools::Qualcuno => Some(!used.is_empty()),
        Tools::Almeno(n) => Some(used.len() >= n),
        Tools::Questi(names) => Some(names.iter().all(|n| used.iter().any(|u| u == n))),
    }
}

/// Ha cercato su internet quando doveva, e non quando non doveva.
fn web_ok(expected: Web, searched: bool) -> Option<bool> {
    match expected {
        Web::Ignora => None,
        Web::Cerca => Some(searched),
        Web::No => Some(!searched),
    }
}

/// Dove il corpus e' la verita', e quindi un nome di file inventato si puo'
/// riconoscere. Nella chat e nella ricerca web i file non c'entrano, e una
/// risposta che ne nomina uno per esempio non sta mentendo su niente.
fn cita_file_inventati(case: &Case, text: &str) -> bool {
    matches!(
        case.category,
        Category::File | Category::Strumenti | Category::Rag | Category::Allegati
    ) && invented_file_names(text, case.prompt)
}

/// Il giudizio su un turno vero.
pub fn turn(
    case: &Case,
    result: Result<Reply, String>,
    retrieval_ok: Option<bool>,
    elapsed: Duration,
) -> Outcome {
    let base = |answer: String, failure: Option<String>| Outcome {
        id: case.id.to_string(),
        category: case.category,
        prompt: case.prompt.to_string(),
        answer,
        correct: false,
        tool_ok: None,
        tool_calls: 0,
        retrieval_ok,
        web_ok: None,
        language_ok: true,
        hallucinated: false,
        seconds: elapsed.as_secs_f32(),
        failure,
    };

    let reply = match result {
        Ok(reply) => reply,
        Err(e) => return base(String::new(), Some(e)),
    };
    if let Some(e) = reply.error.clone() {
        return base(reply.text, Some(e));
    }
    if reply.text.trim().is_empty() {
        return base(reply.text, Some("risposta vuota".to_string()));
    }

    let used: Vec<String> = reply.tools_used.iter().map(|t| t.name.clone()).collect();
    let searched = !reply.sources.is_empty();

    // Le richieste ambigue si giudicano al contrario delle altre: la risposta
    // giusta e' non rispondere. Chiedere cosa si intende basta, e usare uno
    // strumento vuol dire aver tirato a indovinare su file veri.
    let correct = if case.category == Category::Ambigue {
        all_groups(&reply.text, case.expect) && used.is_empty()
    } else {
        all_groups(&reply.text, case.expect)
    };

    Outcome {
        correct,
        tool_ok: tools_ok(case.tools, &used),
        tool_calls: used.len(),
        web_ok: web_ok(case.web, searched),
        language_ok: is_italian(&reply.text),
        hallucinated: forbidden(&reply.text, case.forbid) || cita_file_inventati(case, &reply.text),
        ..base(reply.text, None)
    }
}

/// Il giudizio su uno scambio da cui si doveva imparare qualcosa, o niente.
pub fn memory(case: &Case, facts: Vec<String>, elapsed: Duration) -> Outcome {
    let joined = facts.join(" | ");
    // Nessuna attesa vuol dire che non si doveva imparare niente, e allora la
    // risposta giusta e' la lista vuota.
    let correct =
        if case.expect.is_empty() { facts.is_empty() } else { all_groups(&joined, case.expect) };

    Outcome {
        id: case.id.to_string(),
        category: case.category,
        prompt: case.prompt.to_string(),
        answer: if facts.is_empty() { "(niente)".to_string() } else { joined.clone() },
        correct,
        tool_ok: None,
        tool_calls: 0,
        retrieval_ok: None,
        web_ok: None,
        language_ok: is_italian(&joined),
        hallucinated: forbidden(&joined, case.forbid),
        seconds: elapsed.as_secs_f32(),
        failure: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_file_che_non_esiste_e_una_cosa_inventata() {
        // Il nome che llama3.1:8b si e' inventato nella misura, e che il
        // `forbid` scritto a mano non prevedeva.
        assert!(invented_file_names("Ho letto bolletta_luce.txt", "Quanto ho speso di luce?"));
        // Un file del corpus non e' inventato.
        assert!(!invented_file_names(
            "Il totale sta in fattura_2024_017.txt",
            "Dimmi il totale della fattura."
        ));
        // E nemmeno un file che l'utente stesso ha nominato: li' il modello
        // ripete, non inventa.
        assert!(!invented_file_names(
            "Non trovo bilancio_2023.xlsx",
            "C'e' un file che si chiama bilancio_2023.xlsx nelle mie cartelle?"
        ));
        // Una risposta che non nomina nessun file non inventa niente.
        assert!(!invented_file_names("Il totale e' 1.240,00 euro.", "Dimmi il totale."));
        // Un nome d'esempio dentro un rifiuto a creare un file non e' una
        // citazione: il modello non dice che quel file esiste.
        assert!(!invented_file_names(
            "Non posso creare file. Useresti 'echo prova > nuovofile.txt'.",
            "Crea un file nuovo con dentro la parola prova."
        ));
    }

    #[test]
    fn a_group_is_satisfied_by_any_of_its_alternatives() {
        assert!(any_of("ne servono quattro", &["4", "quattro"]));
        assert!(any_of("ne servono 4", &["4", "quattro"]));
        assert!(!any_of("ne servono tre", &["4", "quattro"]));
    }

    #[test]
    fn every_group_has_to_be_satisfied() {
        let groups: &[&[&str]] = &[&["potenza"], &["energia", "consumo"]];
        assert!(all_groups("il kW e' potenza, il kWh energia", groups));
        // Meta' risposta non e' una risposta.
        assert!(!all_groups("il kW e' una potenza", groups));
    }

    #[test]
    fn upper_and_lower_case_do_not_change_the_verdict() {
        assert!(any_of("La Capitale e' CANBERRA", &["canberra"]));
    }

    #[test]
    fn italian_and_english_are_told_apart() {
        assert!(is_italian("Il totale della fattura e' di 1.240 euro, con IVA compresa."));
        assert!(!is_italian("The total of the invoice is 1240 euro, and this includes the VAT."));
        // Una risposta senza parole non ha lingua: non e' un errore.
        assert!(is_italian("391"));
    }

    #[test]
    fn a_word_inside_another_word_is_not_that_word() {
        // "the" dentro "theme", "di" dentro "dire": contarli renderebbe il
        // riconoscimento della lingua un caso.
        assert!(is_italian("Dire una cosa diversa non cambia il tema del discorso."));
    }

    #[test]
    fn no_tools_means_no_tools() {
        assert_eq!(tools_ok(Tools::Nessuno, &[]), Some(true));
        assert_eq!(tools_ok(Tools::Nessuno, &["read_file".to_string()]), Some(false));
    }

    #[test]
    fn a_chain_that_stops_halfway_is_not_a_chain() {
        assert_eq!(tools_ok(Tools::Almeno(2), &["read_file".to_string()]), Some(false));
        assert_eq!(
            tools_ok(Tools::Almeno(2), &["find_files".to_string(), "read_file".to_string()]),
            Some(true)
        );
    }

    #[test]
    fn the_named_tools_have_to_be_there() {
        let used = vec!["list_dir".to_string()];
        assert_eq!(tools_ok(Tools::Questi(&["list_dir"]), &used), Some(true));
        assert_eq!(tools_ok(Tools::Questi(&["read_file"]), &used), Some(false));
    }

    #[test]
    fn searching_when_it_should_not_is_a_failure_like_the_opposite() {
        assert_eq!(web_ok(Web::No, true), Some(false));
        assert_eq!(web_ok(Web::No, false), Some(true));
        assert_eq!(web_ok(Web::Cerca, false), Some(false));
    }

    #[test]
    fn nothing_learned_is_the_right_answer_when_nothing_was_there() {
        let case = Case {
            id: "prova",
            category: Category::Memoria,
            prompt: "Quanto fa 8 per 9?",
            answer: "72.",
            expect: &[],
            forbid: &["72"],
            tools: Tools::Ignora,
            retrieval: None,
            attachment: false,
            web: Web::Ignora,
        };
        assert!(memory(&case, Vec::new(), Duration::ZERO).correct);

        let imparato = memory(&case, vec!["L'utente ha chiesto quanto fa 72".to_string()], Duration::ZERO);
        assert!(!imparato.correct, "ha imparato un conto e viene contato giusto");
        assert!(imparato.hallucinated);
    }

    #[test]
    fn a_case_that_falls_on_any_count_does_not_pass() {
        let mut outcome = Outcome {
            id: "prova".to_string(),
            category: Category::Chat,
            prompt: String::new(),
            answer: String::new(),
            correct: true,
            tool_ok: Some(true),
            tool_calls: 0,
            retrieval_ok: Some(true),
            web_ok: Some(true),
            language_ok: true,
            hallucinated: false,
            seconds: 0.0,
            failure: None,
        };
        assert!(outcome.passed());

        // Rispondere giusto in inglese non e' rispondere giusto.
        outcome.language_ok = false;
        assert!(!outcome.passed());
        outcome.language_ok = true;

        // Nemmeno rispondere giusto inventandosi meta' della risposta.
        outcome.hallucinated = true;
        assert!(!outcome.passed());
    }
}
