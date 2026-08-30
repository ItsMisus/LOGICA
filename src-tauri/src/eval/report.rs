//! Il rapporto: cosa e' successo, in una forma che si legge.
//!
//! Due regole, che valgono piu' della formattazione. La prima e' che le
//! percentuali non stanno mai da sole: accanto c'e' sempre su quanti casi sono
//! calcolate, perche' "80%" su cinque richieste e "80%" su cento non dicono la
//! stessa cosa. La seconda e' che i casi caduti si elencano per intero, con la
//! risposta che il modello ha dato: un riepilogo che nasconde le cadute serve a
//! sentirsi tranquilli, non a capire cosa non va.

use std::fmt::Write as _;
use std::time::Duration;

use super::cases::Category;
use super::Outcome;

/// I conti di un gruppo di richieste.
struct Tally {
    total: usize,
    passed: usize,
    correct: usize,
    tool_ok: (usize, usize),
    retrieval_ok: (usize, usize),
    web_ok: (usize, usize),
    language_ok: usize,
    hallucinated: usize,
    failures: usize,
    calls: usize,
    times: Vec<Duration>,
}

impl Tally {
    fn of<'a>(outcomes: impl Iterator<Item = &'a Outcome>) -> Self {
        let mut t = Tally {
            total: 0,
            passed: 0,
            correct: 0,
            tool_ok: (0, 0),
            retrieval_ok: (0, 0),
            web_ok: (0, 0),
            language_ok: 0,
            hallucinated: 0,
            failures: 0,
            calls: 0,
            times: Vec::new(),
        };
        for o in outcomes {
            t.total += 1;
            t.passed += o.passed() as usize;
            t.correct += o.correct as usize;
            t.language_ok += o.language_ok as usize;
            t.hallucinated += o.hallucinated as usize;
            t.failures += o.failure.is_some() as usize;
            t.calls += o.tool_calls;
            t.times.push(o.elapsed());
            // Le misure che non valgono per tutti i casi contano solo dove
            // c'erano: un denominatore gonfiato falsa la percentuale.
            if let Some(ok) = o.tool_ok {
                t.tool_ok.0 += ok as usize;
                t.tool_ok.1 += 1;
            }
            if let Some(ok) = o.retrieval_ok {
                t.retrieval_ok.0 += ok as usize;
                t.retrieval_ok.1 += 1;
            }
            if let Some(ok) = o.web_ok {
                t.web_ok.0 += ok as usize;
                t.web_ok.1 += 1;
            }
        }
        t
    }

    /// Il tempo mediano, non la media: una richiesta che va in timeout
    /// sposterebbe la media e farebbe sembrare lento tutto il resto.
    fn median(&self) -> Duration {
        if self.times.is_empty() {
            return Duration::ZERO;
        }
        let mut sorted = self.times.clone();
        sorted.sort_unstable();
        sorted[sorted.len() / 2]
    }

    fn slowest(&self) -> Duration {
        self.times.iter().copied().max().unwrap_or_default()
    }
}

fn share(part: usize, whole: usize) -> String {
    if whole == 0 {
        return "—".to_string();
    }
    format!("{part}/{whole} ({}%)", part * 100 / whole)
}

fn optional_share(pair: (usize, usize)) -> String {
    if pair.1 == 0 {
        "—".to_string()
    } else {
        share(pair.0, pair.1)
    }
}

/// Il rapporto intero, in Markdown.
pub fn write(everything: &[(String, Vec<Outcome>)]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Misura dell'assistente\n");
    let _ = writeln!(
        out,
        "Cento richieste divise per quello che mettono alla prova. Il giudizio e' fatto di \
         confronti fra stringhe: una risposta giusta scritta in un modo non previsto viene contata \
         sbagliata, quindi questi numeri sono una stima per difetto. I casi caduti sono elencati in \
         fondo con la risposta intera, cosi' si vede se e' caduto il modello o il criterio.\n"
    );

    for (model, outcomes) in everything {
        let all = Tally::of(outcomes.iter());
        let _ = writeln!(out, "\n## {model}\n");
        let _ = writeln!(
            out,
            "**{}** richieste superate in pieno su {}. Superata vuol dire: risposta giusta, \
             strumenti giusti, lingua giusta, niente di inventato.\n",
            all.passed, all.total
        );

        let _ = writeln!(
            out,
            "| categoria | n | superate | risposta | strumenti | recupero | web | lingua | inventate | falliti | tempo mediano |"
        );
        let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|---|---|");
        for category in Category::ALL {
            let group: Vec<&Outcome> =
                outcomes.iter().filter(|o| o.category == *category).collect();
            if group.is_empty() {
                continue;
            }
            let t = Tally::of(group.into_iter());
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {:.1}s |",
                category.label(),
                t.total,
                share(t.passed, t.total),
                share(t.correct, t.total),
                optional_share(t.tool_ok),
                optional_share(t.retrieval_ok),
                optional_share(t.web_ok),
                share(t.language_ok, t.total),
                t.hallucinated,
                t.failures,
                t.median().as_secs_f32()
            );
        }
        let _ = writeln!(
            out,
            "| **tutte** | {} | {} | {} | {} | {} | {} | {} | {} | {} | {:.1}s |",
            all.total,
            share(all.passed, all.total),
            share(all.correct, all.total),
            optional_share(all.tool_ok),
            optional_share(all.retrieval_ok),
            optional_share(all.web_ok),
            share(all.language_ok, all.total),
            all.hallucinated,
            all.failures,
            all.median().as_secs_f32()
        );

        let _ = writeln!(
            out,
            "\nChiamate agli strumenti in tutto: {}. Richiesta piu' lenta: {:.1}s.",
            all.calls,
            all.slowest().as_secs_f32()
        );
    }

    if everything.len() > 1 {
        out.push_str(&comparison(everything));
    }

    out.push_str(&fallen(everything));
    out
}

/// Il confronto fra modelli, categoria per categoria. La domanda a cui deve
/// rispondere e' una sola: dove il modello piccolo basta, e dove no.
fn comparison(everything: &[(String, Vec<Outcome>)]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "\n\n## Confronto fra modelli\n");
    let _ = write!(out, "| categoria |");
    for (model, _) in everything {
        let _ = write!(out, " {model} |");
    }
    let _ = writeln!(out);
    let _ = write!(out, "|---|");
    for _ in everything {
        let _ = write!(out, "---|");
    }
    let _ = writeln!(out);

    for category in Category::ALL {
        let mut row = format!("| {} |", category.label());
        let mut any = false;
        for (_, outcomes) in everything {
            let group: Vec<&Outcome> =
                outcomes.iter().filter(|o| o.category == *category).collect();
            if group.is_empty() {
                row.push_str(" — |");
                continue;
            }
            any = true;
            let t = Tally::of(group.into_iter());
            let _ = write!(row, " {} |", share(t.passed, t.total));
        }
        if any {
            let _ = writeln!(out, "{row}");
        }
    }

    let mut row = String::from("| **tempo mediano** |");
    for (_, outcomes) in everything {
        let t = Tally::of(outcomes.iter());
        let _ = write!(row, " {:.1}s |", t.median().as_secs_f32());
    }
    let _ = writeln!(out, "{row}");
    out
}

/// I casi caduti, per esteso. Sono la parte utile del rapporto: le percentuali
/// dicono quanto, questi dicono cosa.
fn fallen(everything: &[(String, Vec<Outcome>)]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "\n\n## Richieste cadute\n");
    for (model, outcomes) in everything {
        let caduti: Vec<&Outcome> = outcomes.iter().filter(|o| !o.passed()).collect();
        let _ = writeln!(out, "\n### {model} — {} cadute\n", caduti.len());
        if caduti.is_empty() {
            let _ = writeln!(out, "Nessuna.");
            continue;
        }
        for o in caduti {
            let mut why: Vec<String> = Vec::new();
            if let Some(e) = &o.failure {
                why.push(format!("non arrivata ({e})"));
            }
            if !o.correct {
                why.push("risposta non corretta".to_string());
            }
            if o.tool_ok == Some(false) {
                why.push(format!("strumenti sbagliati ({} chiamate)", o.tool_calls));
            }
            if o.retrieval_ok == Some(false) {
                why.push("file sbagliato".to_string());
            }
            if o.web_ok == Some(false) {
                why.push("ha cercato quando non doveva, o il contrario".to_string());
            }
            if !o.language_ok {
                why.push("non in italiano".to_string());
            }
            if o.hallucinated {
                why.push("**inventata**".to_string());
            }

            let _ = writeln!(out, "**{}** ({}) — {}", o.id, o.category.label(), why.join(", "));
            let _ = writeln!(out, "> {}\n", o.prompt);
            let answer = o.answer.replace('\n', " ");
            let short: String = answer.chars().take(400).collect();
            let tail = if answer.chars().count() > 400 { "..." } else { "" };
            let _ = writeln!(out, "```\n{short}{tail}\n```\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(category: Category, passed: bool) -> Outcome {
        Outcome {
            id: "prova".to_string(),
            category,
            prompt: "domanda".to_string(),
            answer: "risposta".to_string(),
            correct: passed,
            tool_ok: None,
            tool_calls: 0,
            retrieval_ok: None,
            web_ok: None,
            language_ok: true,
            hallucinated: false,
            seconds: 1.0,
            failure: None,
        }
    }

    #[test]
    fn a_percentage_never_travels_without_its_denominator() {
        assert_eq!(share(4, 5), "4/5 (80%)");
        // Quattro su cinque e ottanta su cento si scrivono diversi apposta.
        assert_eq!(share(80, 100), "80/100 (80%)");
    }

    #[test]
    fn a_measure_that_did_not_apply_is_a_dash_not_a_zero() {
        // Contare "non misurato" come "fallito" farebbe sembrare rotto quello
        // che non e' stato nemmeno provato.
        assert_eq!(optional_share((0, 0)), "—");
        assert_eq!(optional_share((0, 3)), "0/3 (0%)");
    }

    #[test]
    fn the_median_ignores_the_one_that_hung() {
        let t = Tally {
            times: vec![
                Duration::from_secs(2),
                Duration::from_secs(3),
                Duration::from_secs(300),
            ],
            ..Tally::of(std::iter::empty())
        };
        assert_eq!(t.median(), Duration::from_secs(3));
        assert_eq!(t.slowest(), Duration::from_secs(300));
    }

    #[test]
    fn the_report_names_every_fallen_case() {
        let outcomes = vec![outcome(Category::Chat, true), outcome(Category::Rag, false)];
        let text = write(&[("modello".to_string(), outcomes)]);
        assert!(text.contains("Richieste cadute"));
        assert!(text.contains("risposta non corretta"));
        assert!(text.contains("domanda"), "la richiesta caduta non e' nel rapporto");
    }

    #[test]
    fn with_one_model_there_is_nothing_to_compare() {
        let text = write(&[("solo".to_string(), vec![outcome(Category::Chat, true)])]);
        assert!(!text.contains("Confronto fra modelli"));
    }
}
