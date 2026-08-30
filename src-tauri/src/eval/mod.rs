//! Misura l'assistente su cento richieste, invece che su un caso alla volta.
//!
//! I test dal vivo che ci sono altrove provano una funzione con un esempio
//! scelto da chi ha scritto la funzione, ed e' il modo piu' comodo per farsi
//! ingannare: l'esempio che viene in mente e' quello che gia' funziona. Qui le
//! richieste sono cento, divise per quello che mettono alla prova, e diverse
//! sono scritte apposta perche' una risposta non ci sia.
//!
//! ## Cosa si misura, e come
//!
//! - **risposta corretta** — nel testo compare quello che deve comparire. I
//!   dati da trovare sono codici e importi che non si possono indovinare, cosi'
//!   "corretta" vuol dire "ha guardato", non "ha detto qualcosa di sensato".
//! - **strumento corretto** — ha aperto i file quando serviva, e non li ha
//!   aperti quando non serviva. Il secondo caso conta quanto il primo.
//! - **numero di chiamate** — quante volte ha usato uno strumento. Serve per i
//!   casi in catena, dove fermarsi alla prima chiamata vuol dire completare a
//!   memoria il resto.
//! - **recupero corretto** — la ricerca nei documenti ha pescato il file
//!   giusto. Si misura da sola, chiamando la ricerca direttamente: cosi' un
//!   recupero buono con una risposta sbagliata resta distinguibile dal
//!   contrario.
//! - **lingua** — la risposta e' in italiano. Ha gia' fallito una volta, nel
//!   riassunto, e non era visibile da nessuna altra parte.
//! - **risposte inventate** — nel testo compare qualcosa che nei documenti non
//!   c'e'. Ogni caso porta la sua lista.
//! - **tempo** e **fallimenti** — quanto ci mette, e quante volte non arriva
//!   niente.
//!
//! ## Cosa questa misura non e'
//!
//! Il giudizio e' fatto di confronti fra stringhe, non di comprensione. Una
//! risposta giusta scritta in un modo che non avevo previsto viene contata
//! sbagliata, e i numeri che seguono sono quindi una stima **per difetto**
//! della bravura del modello. E' un compromesso voluto: un giudice automatico
//! costruito con un altro modello sarebbe piu' indulgente ma non si potrebbe
//! controllare, e sbaglierebbe in modo invisibile invece che in modo elencato.
//! Ogni caso caduto finisce nel rapporto con la risposta intera, cosi' si vede
//! se e' caduto il modello o il criterio.
//!
//! Il contrario vale per le risposte inventate: si scoprono solo quelle
//! previste da `forbid`. Un modello che si inventa qualcosa a cui non avevo
//! pensato passa senza essere visto.
//!
//! ## Come si esegue
//!
//! ```text
//! cargo test -- --ignored eval_run --nocapture
//! FOAI_EVAL_MODELS=qwen3:30b-a3b,llama3.1:8b cargo test -- --ignored eval_run --nocapture
//! FOAI_EVAL_CATEGORIES=documenti,file cargo test -- --ignored eval_run --nocapture
//! ```
//!
//! Cento richieste contro un modello da 30 miliardi di parametri durano piu' di
//! un'ora. Il filtro per categoria serve a rifare una parte sola dopo una
//! modifica, senza ripagare tutto il conto.

pub mod cases;
pub mod fixtures;
mod report;
mod score;

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use crate::ai::AiConfig;
use cases::{Case, Category, CASES};

/// Com'e' andata una richiesta.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub id: &'static str,
    pub category: Category,
    /// Cosa e' stato chiesto, per rileggere il rapporto senza il dataset a
    /// fianco.
    pub prompt: &'static str,
    /// La risposta, intera. Nel rapporto compare solo per i casi caduti, ma
    /// serve tenerla per poterli guardare.
    pub answer: String,
    pub correct: bool,
    /// `None` quando il caso non parla di strumenti.
    pub tool_ok: Option<bool>,
    pub tool_calls: usize,
    /// `None` fuori dai casi sui documenti.
    pub retrieval_ok: Option<bool>,
    /// `None` dove la ricerca web e' spenta e non c'e' niente da misurare.
    pub web_ok: Option<bool>,
    pub language_ok: bool,
    pub hallucinated: bool,
    pub elapsed: Duration,
    /// Il motivo, quando non e' arrivato niente.
    pub failure: Option<String>,
}

impl Outcome {
    /// Un caso e' passato quando ha risposto giusto, con gli strumenti giusti,
    /// nella lingua giusta, senza inventarsi niente. Tenere le condizioni
    /// insieme evita il conforto di guardare solo la percentuale piu' bella.
    pub fn passed(&self) -> bool {
        self.failure.is_none()
            && self.correct
            && self.tool_ok.unwrap_or(true)
            && self.retrieval_ok.unwrap_or(true)
            && self.web_ok.unwrap_or(true)
            && self.language_ok
            && !self.hallucinated
    }
}

/// Quali interruttori accendere per un caso.
///
/// Non si accende tutto per tutti. Accendere la ricerca web anche dove non si
/// misura costerebbe una chiamata al classificatore per ogni richiesta senza
/// dire niente di nuovo, e accendere i documenti nei casi sugli strumenti
/// renderebbe impossibile capire da dove arriva una risposta giusta: dal file
/// aperto o dall'estratto gia' pronto nel prompt.
fn switches(case: &Case) -> (bool, bool, bool) {
    let file_tools = !matches!(case.category, Category::Memoria | Category::Rag);
    let documents = case.category == Category::Rag;
    let web = matches!(case.category, Category::Chat | Category::Web);
    (file_tools, documents, web)
}

fn config_for(case: &Case, model: &str) -> AiConfig {
    let (file_tools, documents, web_search) = switches(case);
    AiConfig {
        ollama_model: model.to_string(),
        file_tools,
        documents,
        web_search,
        // La memoria si misura chiamando l'estrattore da fuori: lasciarla
        // accesa qui vorrebbe dire che i fatti di una richiesta cambiano il
        // prompt di quella dopo, e i cento casi non sarebbero piu' indipendenti.
        memory: false,
        ..AiConfig::default()
    }
}

/// Esegue una richiesta e la giudica.
fn run_case(runtime: &tokio::runtime::Runtime, case: &Case, model: &str) -> Outcome {
    let cfg = config_for(case, model);
    let started = Instant::now();

    if case.category == Category::Memoria {
        return score::memory(case, runtime.block_on(crate::assistant::learn(
            case.prompt,
            case.answer,
            &cfg,
        )), started.elapsed());
    }

    // Il testo dell'allegato va sotto al messaggio, come fa la finestra della
    // chat: il modello vede una cosa sola.
    let message = if case.attachment {
        format!("{}\n\n{}", case.prompt, fixtures::attachment_block())
    } else {
        case.prompt.to_string()
    };

    let cancel = AtomicBool::new(false);
    let result = runtime.block_on(crate::assistant::stream_reply(
        case.id,
        &message,
        &[],
        &[],
        &cfg,
        &cancel,
        |_| {},
        |_| {},
    ));
    let elapsed = started.elapsed();

    // Il recupero si misura a parte, chiamando la ricerca direttamente: cosi'
    // un file pescato bene con una risposta scritta male non si confonde con
    // il caso opposto.
    let retrieval_ok = case.retrieval.map(|atteso| {
        let hits = runtime.block_on(crate::rag::search(case.prompt, &cfg.ollama_url));
        hits.iter().any(|h| h.path.contains(atteso))
    });

    score::turn(case, result, retrieval_ok, elapsed)
}

/// Prepara la casa finta, i documenti e l'indice. Torna l'indirizzo di Ollama.
fn prepare(models: &[String]) -> Result<(), String> {
    crate::storage::ensure_layout()?;
    let corpus = fixtures::write_all().map_err(|e| format!("corpus non scritto: {e}"))?;

    let mut config = crate::storage::load_config();
    config.monitored_folders = vec![corpus.to_string_lossy().to_string()];
    crate::storage::save_config(&config)?;

    // L'indice si costruisce una volta sola e vale per tutti i modelli: i
    // vettori li calcola l'embedder, non il modello che risponde.
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    let url = AiConfig::default().ollama_url;
    let status = runtime.block_on(crate::rag::build(&url, &mut |_, _, _| {}))?;
    eprintln!(
        "indice pronto: {} file, {} pezzi, modello {}",
        status.files, status.pieces, status.model
    );
    eprintln!("modelli da provare: {}\n", models.join(", "));
    Ok(())
}

fn wanted_models() -> Vec<String> {
    std::env::var("FOAI_EVAL_MODELS")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_else(|_| vec!["qwen3:30b-a3b".to_string(), "llama3.1:8b".to_string()])
}

fn wanted_cases() -> Vec<&'static Case> {
    let filter = std::env::var("FOAI_EVAL_CATEGORIES").unwrap_or_default();
    let keys: Vec<String> =
        filter.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
    CASES
        .iter()
        .filter(|c| keys.is_empty() || keys.iter().any(|k| k == c.category.key()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La misura intera. Richiede `ollama serve`, i modelli scaricati e piu'
    /// di un'ora di tempo:
    /// `cargo test -- --ignored eval_run --nocapture`
    #[test]
    #[ignore]
    fn eval_run() {
        let _home = crate::storage::tests::TempHome::new("eval");
        let models = wanted_models();
        let cases = wanted_cases();
        prepare(&models).expect("preparazione");

        let runtime = tokio::runtime::Runtime::new().expect("runtime tokio");
        let mut everything: Vec<(String, Vec<Outcome>)> = Vec::new();

        for model in &models {
            eprintln!("\n=== {model} — {} richieste ===", cases.len());
            let mut outcomes = Vec::with_capacity(cases.len());
            for (i, case) in cases.iter().enumerate() {
                let outcome = run_case(&runtime, case, model);
                eprintln!(
                    "  [{:>3}/{}] {:<9} {:<22} {}  {:.1}s",
                    i + 1,
                    cases.len(),
                    case.id,
                    case.category.label(),
                    if outcome.passed() { "ok    " } else { "CADUTO" },
                    outcome.elapsed.as_secs_f32()
                );
                outcomes.push(outcome);
            }
            everything.push((model.clone(), outcomes));
        }

        let text = report::write(&everything);
        println!("\n{text}");

        let file = std::env::temp_dir().join("logica_eval_rapporto.md");
        if let Err(e) = std::fs::write(&file, &text) {
            eprintln!("rapporto non salvato: {e}");
        } else {
            eprintln!("\nrapporto in {}", file.display());
        }

        fixtures::clean();

        // Il test non impone una soglia: serve a produrre la misura, non a
        // promuovere o bocciare. L'unica cosa che lo fa fallire e' non essere
        // riuscito a misurare, cioe' se e' caduto tutto.
        let (_, first) = &everything[0];
        let arrivate = first.iter().filter(|o| o.failure.is_none()).count();
        assert!(arrivate > 0, "nessuna richiesta e' arrivata a destinazione: Ollama e' acceso?");
    }
}
