//! Il lato assistente di LOGICA: conversazione pura, in streaming.
//!
//! Qui non si tocca il disco: c'e' solo un modello che risponde, e che quando
//! serve va prima a leggere su internet. La risposta arriva un pezzo alla
//! volta perche' con un 7B locale puo' richiedere mezzo minuto, e mezzo minuto
//! di schermo fermo fa sembrare l'app bloccata.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::StreamExt;

use crate::ai::{self, AiConfig, ChatTurn};
use crate::attachments::Image;
use crate::memory;
use crate::rag;
use crate::summary::{self, Digest};
use crate::tools::{self, ToolCall, ToolUse};
use crate::web;

/// Come deve pensare e scrivere l'assistente. E' la parte che decide se le
/// risposte sono utili o generiche, quindi va trattata come codice: si cambia
/// con un motivo, non a sensazione.
const SYSTEM_PROMPT: &str = "Sei Logica, l'assistente personale di questa app desktop. Parli con una persona sola, in una finestra di chat.

COME RAGIONI
- Rispondi alla domanda vera, non a quella piu' comoda. Se la richiesta e' ambigua e le due letture portano a risposte diverse, fai una domanda precisa (una sola, non un questionario); se invece una lettura e' chiaramente la piu' probabile, vai avanti con quella dicendo cosa hai assunto.
- Prima la risposta, poi il perche'. Niente preamboli, niente riassunti di cio' che stai per dire, niente chiusure di cortesia.
- Se non sai una cosa, dillo. Se sei incerto, di' quanto e cosa servirebbe per esserne sicuro. Non inventare mai fatti, numeri, citazioni, funzioni di libreria o percorsi di file.
- Se l'utente parte da un presupposto sbagliato, dillo apertamente e spiega perche'. Non assecondarlo per educazione.
- Niente adulazione: non aprire con 'ottima domanda' o simili, non complimentarti per l'idea.
- Se serve un ragionamento in piu' passaggi, falli davvero prima di concludere, e mostra solo i passaggi che servono a chi legge.

COME SCRIVI
- Nella lingua dell'utente: se scrive in italiano rispondi in italiano.
- Lunghezza proporzionata alla domanda: una riga per una cosa semplice, piu' spazio per un ragionamento o del codice. Denso, non lungo.
- Markdown solo dove aiuta davvero: elenchi per cose davvero elencabili, **grassetto** per i termini chiave, `codice` inline per nomi di file, comandi e simboli, blocchi di codice con il linguaggio indicato. Per due frasi usa la prosa, non i punti elenco.
- Il codice deve essere completo e funzionante, con i nomi veri di librerie e funzioni; commenta solo i passaggi non ovvi.
- Non ripetere la domanda dell'utente prima di rispondere.

COSA SEI E COSA NON PUOI FARE
- Sei un assistente generale: qualsiasi argomento, dal codice alla vita di tutti i giorni.
- Quando serve, l'app cerca su internet per te e ti mette i risultati sotto gli occhi in un blocco RISULTATI DAL WEB. Se quel blocco non c'e', non hai cercato niente: rispondi con quello che sai e dillo.
- Non hai accesso alla posta o ad altri programmi.";

/// Cosa dire quando la memoria e' spenta o non c'e' ancora niente dentro. O
/// si dice che si ricorda solo questa chat, o si elencano le cose che si
/// ricordano: mai le due insieme.
const NO_MEMORY: &str = "Ricordi solo la chat corrente: le altre conversazioni non le vedi.";

/// Cosa dire quando gli strumenti sui file sono spenti o non c'e' nessuna
/// cartella autorizzata. Sta fuori dal prompt fisso per la stessa ragione:
/// l'alternativa (l'elenco delle cartelle leggibili) lo contraddirebbe riga
/// per riga, e un modello piccolo davanti a due istruzioni opposte ne sceglie
/// una a caso.
const NO_FILE_ACCESS: &str = "Non hai accesso ai file del computer: vedi solo questa conversazione e gli allegati. Se ti chiedono di aprire, cercare o spostare qualcosa sul disco, dillo invece di fingere di averlo fatto; l'utente puo' autorizzare una cartella dalle impostazioni dell'app.";

/// Timeout sull'intera risposta in streaming. Un 7B lento sta ben sotto,
/// una connessione morta non tiene la UI in attesa per sempre.
const STREAM_TIMEOUT: Duration = Duration::from_secs(300);

/// Turni di storico inviati al modello. Piu' in la' i modelli piccoli
/// perdono comunque il filo e la latenza cresce.
const MAX_HISTORY_TURNS: usize = 30;

/// Caratteri di un esito di strumento rimandati al modello. Un `list_dir` su
/// una cartella con dentro mezzo disco riempirebbe da solo tutto il contesto.
const MAX_TOOL_OUTPUT: usize = 8_000;

/// Finestra di contesto richiesta a Ollama: il default (2048) taglia le
/// conversazioni lunghe a meta'.
const OLLAMA_CONTEXT: u32 = 8192;

/// Temperatura della risposta vera: un po' di varieta' fa bene alla prosa.
const CHAT_TEMPERATURE: f32 = 0.7;

/// Temperatura dei passi di servizio (decidere se cercare, dare un titolo).
/// Qui la varieta' e' solo un danno: la stessa domanda deve dare sempre la
/// stessa decisione, altrimenti l'app cerca a volte si' e a volte no.
const SERVICE_TEMPERATURE: f32 = 0.0;

/// Pagine aperte per ogni ricerca. Due bastano quasi sempre e tengono
/// l'attesa prima della risposta a pochi secondi.
const PAGES_PER_SEARCH: usize = 2;

/// Link scritti dall'utente che l'app apre in un solo messaggio.
const MAX_LINKS_PER_MESSAGE: usize = 2;

/// Prompt del passo che decide se cercare. E' separato dalla risposta vera
/// perche' un 7B che deve insieme decidere e rispondere fa male entrambe.
const SEARCH_PLANNER: &str = "Sei un classificatore. Non rispondi mai alla domanda dell'utente: decidi solo se per rispondere servono informazioni prese da internet, e con quali parole cercarle.

Rispondi sempre in due righe, esattamente in questo formato:
SERVE: si oppure no
QUERY: le parole chiave da cercare, oppure - se hai risposto no

SERVE: si per notizie, eventi, prezzi, meteo, orari, risultati sportivi e vincitori di campionati o premi, classifiche, dati che cambiano nel tempo, versioni di software, informazioni su persone, aziende, prodotti o luoghi specifici, e qualsiasi fatto legato a una data. Nel dubbio su un fatto del mondo, SERVE: si: la tua memoria e' vecchia e puo' essere sbagliata.
SERVE: no per saluti e ringraziamenti, matematica e calcoli, codice da scrivere, traduzioni, riscritture e riassunti, opinioni, e domande sui messaggi gia' scritti in questa conversazione.
Se l'utente chiede espressamente di cercare, di guardare online o di dare le fonti, allora SERVE: si.

Esempi.
Utente: che tempo fa a Torino domani
SERVE: si
QUERY: meteo Torino domani

Utente: quanto fa 12 per 12
SERVE: no
QUERY: -

Utente: scrivi una funzione python che ordina una lista
SERVE: no
QUERY: -

Utente: cosa e' successo oggi? dammi le ultime fonti
SERVE: si
QUERY: notizie principali oggi

Utente: chi ha vinto la Champions League 2025
SERVE: si
QUERY: vincitore Champions League 2025

Utente: riassumi il messaggio di prima
SERVE: no
QUERY: -

Utente: cerca online il prezzo della RTX 5090
SERVE: si
QUERY: prezzo RTX 5090";

/// Prompt del passo che decide cosa vale la pena ricordare oltre la chat.
///
/// E' separato dalla risposta, come il pianificatore della ricerca: chiedere
/// al modello di rispondere *e* di annotare cosa ha imparato lo porta a fare
/// male le due cose. Il rischio qui non e' dimenticare, e' ricordare troppo:
/// una memoria piena di dettagli di ieri e' peggio di una memoria vuota.
const MEMORY_EXTRACTOR: &str = "Sei un estrattore di memoria. Leggi lo scambio e scrivi solo i fatti che varra' la pena sapere anche fra un mese, in una conversazione diversa: come si chiama l'utente, che lavoro fa, che strumenti e linguaggi usa, preferenze stabili su come vuole le risposte, progetti in corso, vincoli suoi.

Regole:
- Un fatto per riga, al massimo 3 righe, niente elenchi puntati e niente numerazione.
- Ogni riga e' una frase breve in terza persona che comincia con 'L'utente'.
- Non scrivere quello che vale solo adesso: la domanda di oggi, il contenuto di un file, un calcolo, una notizia, cosa hai risposto.
- Non scrivere quello che riguarda te invece dell'utente.
- Se non c'e' niente da ricordare, rispondi esattamente NIENTE.

Esempi.
Scambio: 'come si scrive una regex in Rust?' / '...'
NIENTE

Scambio: 'mi chiamo Franco e lavoro su un gestionale in Rust, rispondimi sempre in italiano' / '...'
L'utente si chiama Franco
L'utente lavora a un gestionale scritto in Rust
L'utente vuole le risposte in italiano";

/// Prompt del passo che condensa la parte vecchia di una conversazione lunga.
///
/// L'istruzione che conta e' l'ultima: un riassunto che perde i nomi, i numeri
/// e i percorsi e' peggio del taglio secco che sostituisce, perche' sembra
/// che l'assistente ricordi mentre ha dimenticato proprio le cose precise.
const SUMMARIZER: &str = "Riassumi questa parte di conversazione perche' l'assistente possa continuarla senza aver piu' sotto gli occhi i messaggi originali.

Regole:
- Al massimo 10 righe, ognuna una frase secca. Niente titoli, niente preamboli, niente commenti sul riassunto stesso.
- Tieni: cosa ha chiesto l'utente, cosa avete deciso, cosa e' rimasto in sospeso, e le sue preferenze emerse qui.
- Copia esatti i dati concreti: nomi, numeri, percorsi di file, versioni, comandi, termini tecnici. Se un dato non lo puoi riportare preciso, non riportarlo affatto.
- Non aggiungere niente che non sia scritto nella conversazione.";

/// Caratteri tenuti di ogni turno quando si prepara il testo da riassumere.
/// Il riassuntore deve vedere di cosa si parlava, non rileggere tutto.
const MAX_TURN_IN_SUMMARY: usize = 400;

/// Esito di un turno: il testo completo, piu' come e' finito.
pub struct Reply {
    pub text: String,
    /// L'utente ha premuto "Interrompi": il testo e' parziale ma valido.
    pub stopped: bool,
    /// Problema non bloccante (rete caduta a meta' risposta, ad esempio).
    pub error: Option<String>,
    /// Pagine consultate su internet per questa risposta. Vuoto quando non
    /// si e' cercato niente.
    pub sources: Vec<web::SearchHit>,
    /// Strumenti sui file che il modello ha chiesto di usare in questo passo.
    /// Vuoto in una risposta finita.
    pub tool_calls: Vec<ToolCall>,
    /// Cosa ha guardato sul disco per arrivare a questa risposta.
    pub tools_used: Vec<ToolUse>,
}

impl Reply {
    fn new(text: String, stopped: bool, error: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self { text, stopped, error, sources: Vec::new(), tool_calls, tools_used: Vec::new() }
    }
}

/// Un pezzo di risposta letto dallo stream.
#[derive(Debug, PartialEq)]
enum Chunk {
    Text(String),
    /// Chiamate complete, come le manda Ollama: nome e argomenti in un colpo
    /// solo, a volte insieme a un pezzo di testo.
    Tools { text: String, calls: Vec<ToolCall> },
    /// Claude apre il blocco con nome e id, poi manda gli argomenti a pezzi di
    /// JSON e infine lo chiude.
    ToolStart { id: String, name: String },
    ToolArgs(String),
    ToolEnd,
    Done,
    Error(String),
    /// Riga di servizio (keep-alive SSE, evento senza testo): si ignora.
    Skip,
}

/* -------------------------------- prompt -------------------------------- */

/// `web_block` e' il materiale appena scaricato da internet, quando c'e'.
/// Sta nel prompt di sistema e non in un messaggio dell'utente: cosi' non
/// finisce nello storico della chat e non si porta dietro il turno dopo.
fn system_prompt(cfg: &AiConfig, web_block: Option<&str>, file_tools: bool) -> String {
    let mut prompt = SYSTEM_PROMPT.to_string();
    if file_tools {
        prompt.push_str(&format!("\n\n{}", tools::roots_block()));
    } else {
        prompt.push_str(&format!("\n- {NO_FILE_ACCESS}"));
    }
    match cfg.memory.then(memory::prompt_block).flatten() {
        Some(block) => prompt.push_str(&format!("\n\n{block}")),
        None => prompt.push_str(&format!("\n- {NO_MEMORY}")),
    }
    prompt.push_str(&format!(
        "\n\nData di oggi: {}. Usala per capire riferimenti come 'oggi' o 'l'anno scorso'.",
        chrono::Local::now().format("%d/%m/%Y")
    ));
    let extra = cfg.user_preferences.trim();
    if !extra.is_empty() {
        prompt.push_str(&format!(
            "\n\nIstruzioni scritte dall'utente per questa app, da seguire quando non contraddicono quanto sopra: {extra}"
        ));
    }
    if let Some(block) = web_block {
        prompt.push_str(&format!(
            "\n\n{block}\nSu quello che riguarda la domanda fidati di questi dati, non della tua memoria che e' piu' vecchia. Cita le fonti come link Markdown nel punto in cui usi l'informazione. Se qui dentro la risposta non c'e', dillo: non inventarla e non riempirla con quello che ricordi."
        ));
    }
    prompt
}

/// I messaggi da mandare al modello: i turni da `start` in poi, piu' quello
/// appena scritto. Quello che viene prima di `start` non e' perduto, sta nel
/// riassunto dentro il prompt di sistema.
fn history_messages(
    history: &[ChatTurn],
    start: usize,
    message: &str,
    images: &[Image],
    premium: bool,
) -> Vec<serde_json::Value> {
    let start = start.min(history.len());
    let mut out: Vec<serde_json::Value> = history[start..]
        .iter()
        // I messaggi vuoti (una risposta interrotta subito) fanno rifiutare la
        // richiesta dall'API di Claude.
        .filter(|t| !t.content.trim().is_empty())
        .map(|t| {
            let role = if t.role == "user" { "user" } else { "assistant" };
            serde_json::json!({ "role": role, "content": t.content })
        })
        .collect();
    out.push(user_content(message, images, premium));
    out
}

/// Testo scritto dall'utente senza niente da dire: succede quando si allega
/// una immagine e si preme invio.
const LOOK_AT_IMAGES: &str = "Guarda le immagini allegate.";

/// Il messaggio di questo turno, con le immagini allegate se ce ne sono.
///
/// I due motori le vogliono in due modi diversi: Claude in blocchi tipizzati
/// dentro `content`, Ollama in un campo `images` accanto al testo. In
/// entrambi i casi le immagini valgono solo per il turno in corso: nello
/// storico restano le etichette scritte dal frontend, non i byte, o ogni
/// messaggio successivo se le porterebbe dietro.
fn user_content(message: &str, images: &[Image], premium: bool) -> serde_json::Value {
    let text = if message.trim().is_empty() { LOOK_AT_IMAGES } else { message };

    if images.is_empty() {
        return serde_json::json!({ "role": "user", "content": text });
    }

    if premium {
        // Immagini prima del testo: e' l'ordine in cui i modelli di Anthropic
        // rispondono meglio quando la domanda riguarda quello che si vede.
        let mut parts: Vec<serde_json::Value> = images
            .iter()
            .map(|img| {
                serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": img.media_type,
                        "data": img.data,
                    }
                })
            })
            .collect();
        parts.push(serde_json::json!({ "type": "text", "text": text }));
        serde_json::json!({ "role": "user", "content": parts })
    } else {
        let data: Vec<&str> = images.iter().map(|img| img.data.as_str()).collect();
        serde_json::json!({ "role": "user", "content": text, "images": data })
    }
}

/* ----------------------------- lettura stream ---------------------------- */

/// Una riga NDJSON di `/api/chat`: `{"message":{"content":"..."},"done":false}`.
fn ollama_chunk(line: &str) -> Chunk {
    let line = line.trim();
    if line.is_empty() {
        return Chunk::Skip;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Chunk::Skip;
    };
    if let Some(err) = value["error"].as_str() {
        return Chunk::Error(format!("Ollama: {err}"));
    }
    let text = value["message"]["content"].as_str().unwrap_or("");

    // Gli strumenti arrivano completi, spesso nello stesso pezzo che chiude la
    // risposta: vanno letti prima del testo o si perdono.
    let calls = ollama_tool_calls(&value["message"]["tool_calls"]);
    if !calls.is_empty() {
        return Chunk::Tools { text: text.to_string(), calls };
    }

    if !text.is_empty() {
        return Chunk::Text(text.to_string());
    }
    if value["done"].as_bool().unwrap_or(false) {
        return Chunk::Done;
    }
    Chunk::Skip
}

/// `tool_calls` di Ollama. Gli argomenti sono un oggetto JSON, ma qualche
/// modello (e qualche versione) li manda come stringa da riaprire.
fn ollama_tool_calls(value: &serde_json::Value) -> Vec<ToolCall> {
    let Some(list) = value.as_array() else { return Vec::new() };
    list.iter()
        .filter_map(|call| {
            let function = &call["function"];
            let name = function["name"].as_str()?.to_string();
            let raw = &function["arguments"];
            let input = match raw {
                serde_json::Value::String(text) => {
                    serde_json::from_str(text).unwrap_or(serde_json::json!({}))
                }
                serde_json::Value::Null => serde_json::json!({}),
                other => other.clone(),
            };
            Some(ToolCall { id: String::new(), name, input })
        })
        .collect()
}

/// Una riga SSE dell'API Messages: solo le `data:` portano contenuto.
fn claude_chunk(line: &str) -> Chunk {
    let line = line.trim();
    let Some(payload) = line.strip_prefix("data:") else {
        return Chunk::Skip;
    };
    let payload = payload.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return Chunk::Skip;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
        return Chunk::Skip;
    };
    match value["type"].as_str().unwrap_or("") {
        // Un blocco `tool_use` si apre con nome e id; gli argomenti arrivano
        // dopo, a pezzi di JSON che da soli non sono riaprribili.
        "content_block_start" => {
            let block = &value["content_block"];
            match (block["type"].as_str(), block["name"].as_str()) {
                (Some("tool_use"), Some(name)) => Chunk::ToolStart {
                    id: block["id"].as_str().unwrap_or_default().to_string(),
                    name: name.to_string(),
                },
                _ => Chunk::Skip,
            }
        }
        "content_block_delta" => {
            if let Some(partial) = value["delta"]["partial_json"].as_str() {
                return Chunk::ToolArgs(partial.to_string());
            }
            match value["delta"]["text"].as_str() {
                Some(text) if !text.is_empty() => Chunk::Text(text.to_string()),
                _ => Chunk::Skip,
            }
        }
        "content_block_stop" => Chunk::ToolEnd,
        "message_stop" => Chunk::Done,
        "error" => Chunk::Error(format!(
            "Claude: {}",
            value["error"]["message"].as_str().unwrap_or("errore sconosciuto")
        )),
        _ => Chunk::Skip,
    }
}

/// Legge la risposta riga per riga e passa il testo a `on_delta` mano a mano.
/// Si ferma da sola quando lo stream finisce, quando il modello dice `done`
/// o quando `cancel` viene alzato da "Interrompi".
async fn consume(
    res: reqwest::Response,
    parse: fn(&str) -> Chunk,
    cancel: &AtomicBool,
    on_delta: &mut impl FnMut(&str),
) -> Reply {
    let mut text = String::new();
    let mut error = None;
    let mut buffer: Vec<u8> = Vec::new();
    let mut stream = res.bytes_stream();
    let mut calls: Vec<ToolCall> = Vec::new();
    // Blocco `tool_use` di Claude aperto e non ancora chiuso: nome, id e il
    // JSON degli argomenti che si sta ancora accumulando.
    let mut pending: Option<(String, String, String)> = None;

    while let Some(next) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            return Reply::new(text, true, error, Vec::new());
        }
        let bytes = match next {
            Ok(b) => b,
            Err(e) => {
                error = Some(format!("Connessione interrotta: {e}"));
                break;
            }
        };
        buffer.extend_from_slice(&bytes);

        // Si convertono solo le righe complete: cosi' un carattere multibyte
        // spezzato tra due chunk non diventa mai un '?'.
        while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            match parse(&line) {
                Chunk::Text(delta) => {
                    on_delta(&delta);
                    text.push_str(&delta);
                }
                Chunk::Tools { text: delta, calls: found } => {
                    if !delta.is_empty() {
                        on_delta(&delta);
                        text.push_str(&delta);
                    }
                    calls.extend(found);
                }
                Chunk::ToolStart { id, name } => pending = Some((id, name, String::new())),
                Chunk::ToolArgs(partial) => {
                    if let Some((_, _, args)) = pending.as_mut() {
                        args.push_str(&partial);
                    }
                }
                // Chiude anche i blocchi di testo: senza niente in sospeso non
                // c'e' nulla da fare.
                Chunk::ToolEnd => {
                    if let Some((id, name, args)) = pending.take() {
                        let input = if args.trim().is_empty() {
                            serde_json::json!({})
                        } else {
                            serde_json::from_str(&args).unwrap_or(serde_json::json!({}))
                        };
                        calls.push(ToolCall { id, name, input });
                    }
                }
                Chunk::Done => return Reply::new(text, false, error, calls),
                Chunk::Error(e) => return Reply::new(text, false, Some(e), calls),
                Chunk::Skip => {}
            }
        }
    }

    // Ultima riga senza '\n' finale.
    if !buffer.is_empty() {
        match parse(&String::from_utf8_lossy(&buffer)) {
            Chunk::Text(delta) => {
                on_delta(&delta);
                text.push_str(&delta);
            }
            Chunk::Tools { text: delta, calls: found } => {
                if !delta.is_empty() {
                    on_delta(&delta);
                    text.push_str(&delta);
                }
                calls.extend(found);
            }
            _ => {}
        }
    }
    Reply::new(text, false, error, calls)
}

/* ------------------------------ chiamate HTTP ----------------------------- */

async fn stream_ollama(
    cfg: &AiConfig,
    system: &str,
    messages: &[serde_json::Value],
    temperature: f32,
    tools: Option<&[serde_json::Value]>,
    cancel: &AtomicBool,
    on_delta: &mut impl FnMut(&str),
) -> Result<Reply, String> {
    let mut all = vec![serde_json::json!({ "role": "system", "content": system })];
    all.extend_from_slice(messages);

    let mut body = serde_json::json!({
        "model": cfg.ollama_model,
        "messages": all,
        "stream": true,
        "options": { "temperature": temperature, "num_ctx": OLLAMA_CONTEXT }
    });
    if let Some(tools) = tools {
        body["tools"] = serde_json::json!(tools);
    }

    let client = reqwest::Client::builder()
        .timeout(STREAM_TIMEOUT)
        .build()
        .map_err(|e| format!("Client HTTP non inizializzabile: {e}"))?;

    let res = client
        .post(format!("{}/api/chat", cfg.ollama_url.trim_end_matches('/')))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Ollama non raggiungibile su {}: {e}", cfg.ollama_url))?;

    if !res.status().is_success() {
        let status = res.status();
        let detail = res.text().await.unwrap_or_default();
        return Err(missing_model_hint(status, &detail, &cfg.ollama_model));
    }

    Ok(consume(res, ollama_chunk, cancel, on_delta).await)
}

/// Il caso di gran lunga piu' comune: modello scritto nelle impostazioni ma
/// mai scaricato. Il messaggio generico "Ollama ha risposto 404" non aiuta.
fn missing_model_hint(status: reqwest::StatusCode, detail: &str, model: &str) -> String {
    if status == reqwest::StatusCode::NOT_FOUND {
        format!("Il modello \"{model}\" non risulta installato in Ollama. Scaricalo con `ollama pull {model}` oppure scegline un altro nelle impostazioni.")
    } else {
        format!("Ollama ha risposto {status}: {}", detail.trim())
    }
}

async fn stream_claude(
    system: &str,
    messages: &[serde_json::Value],
    temperature: f32,
    tools: Option<&[serde_json::Value]>,
    cancel: &AtomicBool,
    on_delta: &mut impl FnMut(&str),
) -> Result<Reply, String> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| "ANTHROPIC_API_KEY non impostata: imposta la chiave oppure usa Ollama.".to_string())?;

    let mut body = serde_json::json!({
        "model": ai::CLAUDE_MODEL,
        "max_tokens": 4096,
        "system": system,
        "messages": messages,
        "temperature": temperature,
        "stream": true
    });
    if let Some(tools) = tools {
        body["tools"] = serde_json::json!(tools);
    }

    let client = reqwest::Client::builder()
        .timeout(STREAM_TIMEOUT)
        .build()
        .map_err(|e| format!("Client HTTP non inizializzabile: {e}"))?;

    let res = client
        .post(ai::CLAUDE_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Claude API non raggiungibile: {e}"))?;

    if !res.status().is_success() {
        let status = res.status();
        let detail = res.text().await.unwrap_or_default();
        return Err(format!("Claude API ha risposto {status}: {}", detail.trim()));
    }

    Ok(consume(res, claude_chunk, cancel, on_delta).await)
}

/* -------------------------------- internet -------------------------------- */

/// Una domanda secca al modello, senza streaming e senza storico: serve ai
/// passi di servizio (titolo della chat, decisione di cercare).
async fn ask(system: &str, prompt: &str, cfg: &AiConfig) -> Option<String> {
    let messages = vec![serde_json::json!({ "role": "user", "content": prompt })];
    let cancel = AtomicBool::new(false);
    let mut sink = |_: &str| {};
    let reply = if cfg.use_premium_ai {
        stream_claude(system, &messages, SERVICE_TEMPERATURE, None, &cancel, &mut sink).await
    } else {
        stream_ollama(cfg, system, &messages, SERVICE_TEMPERATURE, None, &cancel, &mut sink).await
    }
    .ok()?;
    Some(reply.text)
}

/// I link http/https scritti dall'utente nel messaggio.
fn links_in(message: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = message;
    loop {
        let Some(pos) = [rest.find("http://"), rest.find("https://")].into_iter().flatten().min()
        else {
            break;
        };
        let tail = &rest[pos..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`'))
            .unwrap_or(tail.len());
        // La punteggiatura che chiude la frase non fa parte dell'indirizzo.
        let url = tail[..end]
            .trim_end_matches(|c| matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}'));
        if url.len() > 10 && !out.iter().any(|u| u == url) {
            out.push(url.to_string());
        }
        rest = &tail[end.max(1)..];
    }
    out
}

/// Legge la risposta del classificatore: due righe, `SERVE:` e `QUERY:`.
/// `None` vuol dire "rispondi con quello che sai". I modelli piccoli sbavano
/// sul formato, quindi si accetta anche la sola query su una riga.
fn clean_query(raw: &str) -> Option<String> {
    let mut needs: Option<bool> = None;
    let mut query: Option<String> = None;

    for line in raw.lines() {
        let line = line.trim().trim_matches(|c: char| c == '*' || c == '`').trim();
        let Some((label, value)) = line.split_once(':') else { continue };
        let value = value.trim().trim_matches(|c: char| c == '"' || c == '\'').trim();
        match label.trim().to_lowercase().as_str() {
            "serve" => needs = Some(value.starts_with("si") || value.starts_with("sì") || value.starts_with("yes")),
            "query" | "ricerca" => {
                if value != "-" && !value.is_empty() {
                    query = Some(value.trim_end_matches('.').trim().to_string());
                }
            }
            _ => {}
        }
    }

    // Formato rispettato: comanda SERVE, anche quando la QUERY e' rimasta
    // attaccata per inerzia.
    if let Some(needs) = needs {
        return if needs { query.filter(|q| is_query(q)) } else { None };
    }
    // Formato saltato: resta la prima riga utile, com'era prima.
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = match line.split_once(':') {
        Some((before, after)) if before.chars().count() <= 60 && !after.trim().is_empty() => after.trim(),
        _ => line,
    };
    let line = line.trim_matches(|c: char| c == '"' || c == '\'' || c == '*').trim();
    let normalized = line.trim_end_matches('.').trim();

    // "NO", ma anche "NO, non serve internet": un rifiuto motivato resta un
    // rifiuto e non va cercato su Google.
    let is_no = normalized.len() >= 2
        && normalized[..2].eq_ignore_ascii_case("no")
        && normalized[2..].chars().next().is_none_or(|c| !c.is_alphanumeric());
    if is_no {
        return None;
    }
    Some(normalized.to_string()).filter(|q| is_query(q))
}

/// Una query plausibile: non vuota e non un paragrafo. Piu' lunga di cosi'
/// e' il modello che ha risposto alla domanda invece di pianificare.
fn is_query(text: &str) -> bool {
    !text.is_empty() && text.chars().count() <= 120
}

/// Frasi con cui si chiede di cercare, non di rispondere a memoria. Quando
/// una compare nel messaggio, la ricerca si fa e basta: il modello puo'
/// sbagliare a giudicare la domanda, non a leggere un ordine esplicito.
const SEARCH_COMMANDS: [&str; 14] = [
    "cerca",
    "cercare",
    "ricerca su internet",
    "su internet",
    "su google",
    "online",
    "in rete",
    "tempo reale",
    "ultime notizie",
    "dammi le fonti",
    "dammi le ultime fonti",
    "cita le fonti",
    "search",
    "look up",
];

/// L'utente ha chiesto di andare a vedere, non di rispondere a memoria.
fn wants_search(message: &str) -> bool {
    let lower = message.to_lowercase();
    SEARCH_COMMANDS.iter().any(|c| lower.contains(c))
}

/// Radici dei verbi con cui si ordina una ricerca. Si tagliano per radice
/// perche' l'italiano coniuga: "cerca", "cerchi", "cercami", "ricercare".
const COMMAND_STEMS: [&str; 4] = ["cerc", "ricerc", "search", "googl"];

/// Parole che in una casella di ricerca sono solo rumore.
const COMMAND_NOISE: [&str; 24] = [
    "voglio", "vorrei", "puoi", "potresti", "per", "favore", "mi", "che",
    "su", "internet", "google", "online", "in", "rete", "tempo", "reale",
    "dammi", "dimmi", "le", "la", "il", "fonti", "adesso", "subito",
];

/// Ultima spiaggia quando l'utente ha chiesto di cercare e il pianificatore
/// non ha prodotto niente di usabile: si cerca il messaggio stesso, ripulito
/// dalle parole di comando che in un motore di ricerca sono solo rumore.
fn query_from_message(message: &str, history: &[ChatTurn]) -> Option<String> {
    let lower = message.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .filter(|w| !COMMAND_STEMS.iter().any(|stem| w.starts_with(stem)))
        .filter(|w| !COMMAND_NOISE.contains(w))
        .collect();

    let cleaned = words.join(" ");
    if cleaned.chars().count() >= 3 {
        return Some(truncate(&cleaned, 120));
    }
    // Il messaggio era solo un ordine ("cerca in tempo reale"): la domanda
    // vera e' quella di prima.
    history
        .iter()
        .rev()
        .find(|t| t.role == "user")
        .map(|t| truncate(&t.content, 120))
}

/// Chiede al modello se questa domanda ha bisogno di internet, e con quale
/// query. `None` significa: rispondi con quello che sai.
async fn plan_search(message: &str, history: &[ChatTurn], cfg: &AiConfig) -> Option<String> {
    let start = history.len().saturating_sub(4);
    let recent: String = history[start..]
        .iter()
        .map(|t| {
            let who = if t.role == "user" { "Utente" } else { "Assistente" };
            format!("{who}: {}
", truncate(&t.content, 300))
        })
        .collect();

    let forced = wants_search(message);
    let order = if forced {
        "

L'utente ha chiesto espressamente di cercare: SERVE deve essere si."
    } else {
        ""
    };
    let prompt = format!(
        "{recent}Ultimo messaggio dell'utente: {}

Data di oggi: {}.{order}",
        truncate(message, 1000),
        chrono::Local::now().format("%d/%m/%Y")
    );

    let planned = ask(SEARCH_PLANNER, &prompt, cfg).await.and_then(|r| clean_query(&r));
    match planned {
        Some(query) => Some(query),
        // Ordine esplicito e pianificatore inutile: si cerca lo stesso.
        None if forced => query_from_message(message, history),
        None => None,
    }
}

/// Raccoglie il materiale da internet per questo messaggio: prima i link che
/// l'utente ha scritto, altrimenti una ricerca, se il modello dice che serve.
/// Torna il blocco da mettere nel prompt e le fonti da mostrare in chat.
async fn gather_web(
    message: &str,
    history: &[ChatTurn],
    cfg: &AiConfig,
    on_status: &mut impl FnMut(&str),
) -> Option<(String, Vec<web::SearchHit>)> {
    let links = links_in(message);

    if !links.is_empty() {
        let mut research = web::Research {
            query: "link scritti dall'utente".to_string(),
            pages: Vec::new(),
            hits: Vec::new(),
        };
        let mut failures = String::new();
        for url in links.iter().take(MAX_LINKS_PER_MESSAGE) {
            on_status(&format!("Apro {url}"));
            match web::fetch_page(url).await {
                Ok(page) => {
                    research.hits.push(web::SearchHit {
                        title: page.title.clone(),
                        url: page.url.clone(),
                        snippet: String::new(),
                    });
                    research.pages.push(page);
                }
                Err(e) => failures.push_str(&format!("Pagina non aperta: {e}\n")),
            }
        }
        if research.pages.is_empty() && failures.is_empty() {
            return None;
        }
        let mut block = web::as_prompt_block(&research);
        block.push_str(&failures);
        return Some((block, research.hits));
    }

    if !cfg.web_search {
        return None;
    }

    let query = plan_search(message, history, cfg).await?;
    on_status(&format!("Cerco su internet: {query}"));

    let research = match web::research(&query, PAGES_PER_SEARCH).await {
        Ok(r) if !r.hits.is_empty() => r,
        // Ricerca a vuoto o rete assente: si risponde lo stesso, dicendo che
        // da internet non e' arrivato niente.
        Ok(_) => {
            return Some((
                format!("RICERCA WEB SENZA RISULTATI (query \"{query}\"). Di' all'utente che su internet non hai trovato niente, poi rispondi con quello che sai gia', dicendo che potrebbe non essere aggiornato."),
                Vec::new(),
            ))
        }
        Err(e) => {
            return Some((
                format!("RICERCA WEB FALLITA: {e}. Dillo all'utente in una riga, poi rispondi con quello che sai gia'."),
                Vec::new(),
            ))
        }
    };

    for page in &research.pages {
        on_status(&format!("Leggo {}", page.url));
    }
    let hits = research.hits.clone();
    Some((web::as_prompt_block(&research), hits))
}

/* ------------------------------ conversazione ----------------------------- */

/// Un turno di conversazione. `on_delta` viene chiamata a ogni pezzo di testo
/// che arriva: e' li' che il frontend riceve l'effetto macchina da scrivere.
/// `on_status` racconta cosa sta facendo l'app prima di iniziare a scrivere
/// (ricerca, pagine aperte): quei secondi sono di silenzio.
pub async fn stream_reply(
    chat_id: &str,
    message: &str,
    history: &[ChatTurn],
    images: &[Image],
    cfg: &AiConfig,
    cancel: &AtomicBool,
    mut on_delta: impl FnMut(&str),
    mut on_status: impl FnMut(&str),
) -> Result<Reply, String> {
    // I link scritti a mano si aprono anche a ricerca spenta: e' una richiesta
    // esplicita dell'utente, non un'iniziativa del modello.
    let material = if cfg.web_search || !links_in(message).is_empty() {
        gather_web(message, history, cfg, &mut on_status).await
    } else {
        None
    };
    let (block, sources) = match material {
        Some((block, sources)) => (Some(block), sources),
        None => (None, Vec::new()),
    };

    // Gli strumenti si accendono solo se l'utente li vuole e se c'e' davvero
    // una cartella autorizzata: dichiararli a vuoto fa perdere al modello sei
    // giri contro lo stesso rifiuto.
    let file_tools = cfg.file_tools && tools::available();
    let (older, start) = condense(chat_id, history, cfg, &mut on_status).await;

    // Gli estratti dai documenti costano una sola chiamata di embedding, corta
    // e locale: non serve chiedere a un modello se convenga cercare, come per
    // internet. Se niente somiglia abbastanza, non entra niente nel prompt.
    let documents = if cfg.documents {
        on_status("Cerco nei tuoi documenti");
        rag::search(message, &cfg.ollama_url).await
    } else {
        Vec::new()
    };

    let mut system = system_prompt(cfg, block.as_deref(), file_tools);
    if !documents.is_empty() {
        system.push_str(&format!("\n\n{}", rag::as_prompt_block(&documents)));
    }
    if let Some(older) = older {
        system.push_str(&format!("\n\n{older}"));
    }
    let mut messages = history_messages(history, start, message, images, cfg.use_premium_ai);

    // Testo gia' mandato al frontend nei passi precedenti: e' partito con gli
    // eventi `assistant-delta`, quindi deve ritrovarsi anche nella risposta
    // salvata, o la chat ricaricata risulterebbe monca.
    let mut carried = String::new();
    let mut tools_used: Vec<ToolUse> = Vec::new();
    let mut step = 0usize;

    let mut reply = loop {
        // All'ultimo giro gli strumenti si tolgono: il modello deve concludere
        // con quello che ha raccolto invece di chiedere ancora.
        let offer_tools = file_tools && step < tools::MAX_STEPS;
        let step_reply =
            run_turn(cfg, &system, &messages, offer_tools, cancel, &mut on_delta).await?;

        if step_reply.tool_calls.is_empty() || step_reply.stopped || !offer_tools {
            break step_reply;
        }

        carried.push_str(&step_reply.text);
        messages.push(assistant_with_tools(&step_reply, cfg.use_premium_ai));

        let mut results: Vec<serde_json::Value> = Vec::new();
        for call in &step_reply.tool_calls {
            on_status(&tools::status_line(call));
            let output = tools::run(call);
            tools_used.push(ToolUse::of(call));
            results.push(tool_result(call, &output, cfg.use_premium_ai));
        }
        // Claude vuole tutti gli esiti in un solo messaggio utente, Ollama uno
        // per chiamata con ruolo `tool`.
        if cfg.use_premium_ai {
            messages.push(serde_json::json!({ "role": "user", "content": results }));
        } else {
            messages.extend(results);
        }
        step += 1;
    };

    reply.text = format!("{carried}{}", reply.text);
    reply.sources = sources;
    reply.tools_used = tools_used;
    Ok(reply)
}

/// Un giro di modello, con o senza strumenti dichiarati.
///
/// Se la richiesta con gli strumenti viene rifiutata si riprova una volta
/// senza: le versioni vecchie di Ollama non sanno rispondere in streaming
/// quando ci sono `tools`, e in quel caso e' meglio una risposta senza accesso
/// ai file che nessuna risposta.
async fn run_turn(
    cfg: &AiConfig,
    system: &str,
    messages: &[serde_json::Value],
    offer_tools: bool,
    cancel: &AtomicBool,
    on_delta: &mut impl FnMut(&str),
) -> Result<Reply, String> {
    let declared = if !offer_tools {
        None
    } else if cfg.use_premium_ai {
        Some(tools::claude_tools())
    } else {
        Some(tools::ollama_tools())
    };

    let attempt = if cfg.use_premium_ai {
        stream_claude(system, messages, CHAT_TEMPERATURE, declared.as_deref(), cancel, on_delta).await
    } else {
        stream_ollama(cfg, system, messages, CHAT_TEMPERATURE, declared.as_deref(), cancel, on_delta)
            .await
    };

    match attempt {
        Err(e) if declared.is_some() => {
            eprintln!("[logica] strumenti sui file rifiutati dal motore, riprovo senza: {e}");
            if cfg.use_premium_ai {
                stream_claude(system, messages, CHAT_TEMPERATURE, None, cancel, on_delta).await
            } else {
                stream_ollama(cfg, system, messages, CHAT_TEMPERATURE, None, cancel, on_delta).await
            }
        }
        other => other,
    }
}

/// Il messaggio dell'assistente che ha chiesto gli strumenti, da rimettere
/// nello storico prima degli esiti.
fn assistant_with_tools(reply: &Reply, premium: bool) -> serde_json::Value {
    if premium {
        let mut parts: Vec<serde_json::Value> = Vec::new();
        // Un blocco di testo vuoto fa rifiutare la richiesta dall'API.
        if !reply.text.trim().is_empty() {
            parts.push(serde_json::json!({ "type": "text", "text": reply.text }));
        }
        for call in &reply.tool_calls {
            parts.push(serde_json::json!({
                "type": "tool_use",
                "id": call.id,
                "name": call.name,
                "input": call.input,
            }));
        }
        serde_json::json!({ "role": "assistant", "content": parts })
    } else {
        let calls: Vec<serde_json::Value> = reply
            .tool_calls
            .iter()
            .map(|c| serde_json::json!({ "function": { "name": c.name, "arguments": c.input } }))
            .collect();
        serde_json::json!({ "role": "assistant", "content": reply.text, "tool_calls": calls })
    }
}

/// L'esito di uno strumento nel formato che il motore si aspetta.
fn tool_result(call: &ToolCall, output: &str, premium: bool) -> serde_json::Value {
    let output = truncate(output, MAX_TOOL_OUTPUT);
    if premium {
        serde_json::json!({
            "type": "tool_result",
            "tool_use_id": call.id,
            "content": output,
        })
    } else {
        serde_json::json!({ "role": "tool", "tool_name": call.name, "content": output })
    }
}

/// Titolo breve per la conversazione, chiesto al modello dopo il primo
/// scambio. Se il modello non collabora si torna `None` e il frontend tiene
/// il titolo ricavato dal testo dell'utente.
pub async fn suggest_title(question: &str, answer: &str, cfg: &AiConfig) -> Option<String> {
    let system = "Scrivi un titolo per una conversazione. Da 2 a 5 parole, nella lingua della conversazione, senza virgolette, senza punto finale, senza la parola 'chat'. Rispondi soltanto col titolo.";
    let prompt = format!("Utente: {}\n\nAssistente: {}", truncate(question, 500), truncate(answer, 500));
    let text = ask(system, &prompt, cfg).await?;

    let title = text
        .lines()
        // I modelli piccoli premettono spesso "Titolo:" o ragionano prima.
        .map(|l| l.trim().trim_start_matches("Titolo:").trim_start_matches("titolo:").trim())
        .find(|l| !l.is_empty())?
        .trim_matches(|c| c == '"' || c == '\'' || c == '.' || c == '*')
        .trim()
        .to_string();

    // Un titolo lungo e' quasi sempre il modello che ha risposto alla domanda
    // invece di intitolarla: meglio lasciarlo perdere.
    if title.is_empty() || title.chars().count() > 60 {
        None
    } else {
        Some(title)
    }
}

/* -------------------------------- riassunto ------------------------------- */

/// Prepara il contesto di una conversazione lunga: il riassunto della parte
/// vecchia, e da che turno in poi i messaggi vanno al modello per intero.
///
/// Torna `(None, start)` per le chat corte e per quelle in cui il riassunto
/// non si e' potuto fare: in quel caso vale il taglio secco di prima, che e'
/// peggio ma non blocca la risposta.
async fn condense(
    chat_id: &str,
    history: &[ChatTurn],
    cfg: &AiConfig,
    on_status: &mut impl FnMut(&str),
) -> (Option<String>, usize) {
    let total = history.len();
    let fallback = total.saturating_sub(MAX_HISTORY_TURNS);
    if total <= MAX_HISTORY_TURNS {
        return (None, 0);
    }

    // Un riassunto che copre piu' turni di quanti ce ne siano parla di
    // messaggi che non esistono piu': l'utente ne ha modificato uno e la
    // conversazione e' ripartita da li'. Si ricomincia da capo.
    let existing = summary::load(chat_id).filter(|d| d.covered <= total);
    let covered = existing.as_ref().map(|d| d.covered).unwrap_or(0);

    let mut digest = existing;
    if summary::needs_refresh(total, covered, MAX_HISTORY_TURNS) {
        let target = summary::target(total, MAX_HISTORY_TURNS);
        on_status("Riassumo la parte piu' vecchia della conversazione");
        let previous = digest.as_ref().map(|d| d.text.as_str());
        if let Some(text) = summarize(previous, &history[covered..target], cfg).await {
            let fresh = Digest { covered: target, text };
            // Se il salvataggio fallisce si rifara' il riassunto al turno dopo:
            // costa una chiamata, non una risposta sbagliata.
            if let Err(e) = summary::save(chat_id, &fresh) {
                eprintln!("[logica] riassunto non salvato: {e}");
            }
            digest = Some(fresh);
        }
    }

    match digest {
        Some(d) => (Some(summary::prompt_block(&d.text)), d.covered),
        None => (None, fallback),
    }
}

/// Condensa dei turni, tenendo conto del riassunto gia' fatto delle parti
/// ancora precedenti.
async fn summarize(
    previous: Option<&str>,
    turns: &[ChatTurn],
    cfg: &AiConfig,
) -> Option<String> {
    if turns.is_empty() {
        return previous.map(str::to_string);
    }
    let conversation: String = turns
        .iter()
        .filter(|t| !t.content.trim().is_empty())
        .map(|t| {
            let who = if t.role == "user" { "Utente" } else { "Assistente" };
            format!("{who}: {}\n", truncate(&t.content, MAX_TURN_IN_SUMMARY))
        })
        .collect();

    let prompt = match previous {
        Some(before) => format!(
            "Riassunto di quello che veniva ancora prima:\n{before}\n\nSeguito della conversazione da riassumere insieme al pezzo qui sopra, in un unico riassunto:\n{conversation}"
        ),
        None => format!("Conversazione da riassumere:\n{conversation}"),
    };

    let text = ask(SUMMARIZER, &prompt, cfg).await?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(truncate(text, summary::MAX_SUMMARY_CHARS))
}

/// Cosa c'era da imparare in questo scambio, se c'era qualcosa. Gira dopo che
/// la risposta e' gia' a schermo: l'utente non aspetta questa chiamata.
pub async fn learn(question: &str, answer: &str, cfg: &AiConfig) -> Vec<String> {
    let prompt = format!(
        "Scambio:\nUtente: {}\nAssistente: {}",
        truncate(question, 1500),
        truncate(answer, 1500)
    );
    match ask(MEMORY_EXTRACTOR, &prompt, cfg).await {
        Some(text) => clean_facts(&text),
        None => Vec::new(),
    }
}

/// Legge la risposta dell'estrattore. I modelli piccoli antepongono trattini,
/// numeri e frasi di servizio: si tiene solo quello che assomiglia a un fatto.
fn clean_facts(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|line| {
            line.trim()
                .trim_start_matches(|c: char| c == '-' || c == '*' || c == '•' || c.is_numeric() || c == '.' || c == ')')
                .trim()
                .trim_matches(|c: char| c == '"' || c == '\'' || c == '*')
                .trim()
                .to_string()
        })
        .filter(|line| !line.is_empty())
        // "NIENTE", ma anche "NIENTE da ricordare": il rifiuto resta un rifiuto.
        .filter(|line| !line.to_lowercase().starts_with("niente"))
        // Una riga di preambolo ("Ecco i fatti:") non e' un fatto.
        .filter(|line| !line.ends_with(':'))
        // L'estrattore deve parlare dell'utente. Quando scivola sul contenuto
        // della risposta, la riga non comincia cosi'.
        .filter(|line| line.to_lowercase().starts_with("l'utente") || line.to_lowercase().starts_with("lutente"))
        .take(3)
        .collect()
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect::<String>() + "..."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_lines_become_text_and_end() {
        assert_eq!(
            ollama_chunk(r#"{"message":{"role":"assistant","content":"Ciao"},"done":false}"#),
            Chunk::Text("Ciao".to_string())
        );
        assert_eq!(ollama_chunk(r#"{"message":{"content":""},"done":true}"#), Chunk::Done);
        assert_eq!(
            ollama_chunk(r#"{"error":"model not found"}"#),
            Chunk::Error("Ollama: model not found".to_string())
        );
        // Riga vuota o JSON monco: non deve far cadere lo stream.
        assert_eq!(ollama_chunk(""), Chunk::Skip);
        assert_eq!(ollama_chunk("{\"message\":"), Chunk::Skip);
    }

    #[test]
    fn claude_sse_events_are_read() {
        let delta = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Ciao"}}"#;
        assert_eq!(claude_chunk(delta), Chunk::Text("Ciao".to_string()));
        assert_eq!(claude_chunk(r#"data: {"type":"message_stop"}"#), Chunk::Done);
        assert_eq!(
            claude_chunk(r#"data: {"type":"error","error":{"message":"overloaded"}}"#),
            Chunk::Error("Claude: overloaded".to_string())
        );
        // Righe di servizio SSE.
        assert_eq!(claude_chunk("event: content_block_delta"), Chunk::Skip);
        assert_eq!(claude_chunk(""), Chunk::Skip);
        assert_eq!(claude_chunk(r#"data: {"type":"ping"}"#), Chunk::Skip);
    }

    #[test]
    fn history_starts_where_it_is_told_to() {
        let history: Vec<ChatTurn> = (0..40)
            .map(|i| ChatTurn { role: if i % 2 == 0 { "user" } else { "ai" }.to_string(), content: format!("m{i}") })
            .collect();
        let messages = history_messages(&history, 10, "ultimo", &[], false);

        assert_eq!(messages.len(), 31);
        assert_eq!(messages[0]["content"], "m10");
        assert_eq!(messages[0]["role"], "user");
        // I ruoli dell'app ("ai") diventano quelli dell'API ("assistant").
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages.last().unwrap()["content"], "ultimo");
    }

    #[test]
    fn a_start_past_the_end_leaves_only_the_new_message() {
        // Succede se un riassunto vecchio dice di coprire piu' turni di quanti
        // ce ne sono: meglio una conversazione corta di un panico.
        let history =
            vec![ChatTurn { role: "user".to_string(), content: "unico".to_string() }];
        let messages = history_messages(&history, 99, "ultimo", &[], false);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"], "ultimo");
    }

    #[test]
    fn empty_turns_are_dropped() {
        let history = vec![
            ChatTurn { role: "user".to_string(), content: "ciao".to_string() },
            ChatTurn { role: "ai".to_string(), content: "   ".to_string() },
        ];
        let messages = history_messages(&history, 0, "ci sei?", &[], false);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["content"], "ciao");
        assert_eq!(messages[1]["content"], "ci sei?");
    }

    #[test]
    fn user_instructions_reach_the_prompt() {
        let cfg = AiConfig { user_preferences: "  parlami in modo tecnico  ".to_string(), ..AiConfig::default() };
        let prompt = system_prompt(&cfg, None, false);
        assert!(prompt.contains("parlami in modo tecnico"));
        assert!(prompt.contains("Data di oggi"));

        // Senza istruzioni non si aggiunge una sezione vuota.
        let plain = system_prompt(&AiConfig::default(), None, false);
        assert!(!plain.contains("Istruzioni scritte dall'utente"));
    }

    #[test]
    fn nothing_to_remember_stays_nothing() {
        assert!(clean_facts("NIENTE").is_empty());
        assert!(clean_facts("Niente da ricordare.").is_empty());
        assert!(clean_facts("   \n  \n").is_empty());
    }

    #[test]
    fn facts_survive_the_decorations_small_models_add() {
        let facts = clean_facts(
            "Ecco i fatti:\n- L'utente si chiama Franco\n2. L'utente lavora in Rust\n\"L'utente vuole risposte in italiano\"",
        );
        assert_eq!(
            facts,
            vec![
                "L'utente si chiama Franco",
                "L'utente lavora in Rust",
                "L'utente vuole risposte in italiano",
            ]
        );
    }

    #[test]
    fn a_summary_of_the_answer_is_not_a_fact() {
        // L'estrattore scivola spesso sul contenuto della risposta: quelle
        // righe non parlano dell'utente e non devono entrare in memoria.
        let facts = clean_facts("La regex in Rust si scrive con la crate regex\nL'utente usa Rust");
        assert_eq!(facts, vec!["L'utente usa Rust"]);
    }

    #[test]
    fn memory_is_described_once_and_in_one_verse_only() {
        let forgetful = system_prompt(&AiConfig { memory: false, ..AiConfig::default() }, None, false);
        assert!(forgetful.contains("Ricordi solo la chat corrente"));
        assert!(!forgetful.contains("QUELLO CHE SAI GIA'"));
    }

    #[test]
    fn file_access_is_described_once_and_in_one_verse_only() {
        let closed = system_prompt(&AiConfig::default(), None, false);
        assert!(closed.contains("Non hai accesso ai file"));
        assert!(!closed.contains("STRUMENTI SUI FILE"));

        let open = system_prompt(&AiConfig::default(), None, true);
        assert!(open.contains("STRUMENTI SUI FILE"));
        assert!(!open.contains("Non hai accesso ai file"));
    }

    #[test]
    fn ollama_tool_calls_are_read_with_the_text_that_comes_with_them() {
        let line = r#"{"message":{"role":"assistant","content":"guardo","tool_calls":[{"function":{"name":"list_dir","arguments":{"path":"C:\\Documenti"}}}]},"done":false}"#;
        match ollama_chunk(line) {
            Chunk::Tools { text, calls } => {
                assert_eq!(text, "guardo");
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "list_dir");
                assert_eq!(calls[0].input["path"], "C:\\Documenti");
            }
            other => panic!("atteso Tools, ottenuto {other:?}"),
        }
    }

    #[test]
    fn ollama_arguments_survive_arriving_as_a_string() {
        let line = r#"{"message":{"content":"","tool_calls":[{"function":{"name":"read_file","arguments":"{\"path\":\"/tmp/a.txt\"}"}}]}}"#;
        match ollama_chunk(line) {
            Chunk::Tools { calls, .. } => assert_eq!(calls[0].input["path"], "/tmp/a.txt"),
            other => panic!("atteso Tools, ottenuto {other:?}"),
        }
    }

    #[test]
    fn claude_builds_a_tool_call_out_of_three_events() {
        assert_eq!(
            claude_chunk(r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"find_files","input":{}}}"#),
            Chunk::ToolStart { id: "toolu_1".to_string(), name: "find_files".to_string() }
        );
        assert_eq!(
            claude_chunk(r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"name\":"}}"#),
            Chunk::ToolArgs("{\"name\":".to_string())
        );
        assert_eq!(claude_chunk(r#"data: {"type":"content_block_stop","index":1}"#), Chunk::ToolEnd);
        // Il testo normale continua a passare come prima.
        assert_eq!(
            claude_chunk(r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"ok"}}"#),
            Chunk::Text("ok".to_string())
        );
    }

    #[test]
    fn tool_results_take_the_shape_each_engine_wants() {
        let call = ToolCall {
            id: "toolu_1".to_string(),
            name: "list_dir".to_string(),
            input: serde_json::json!({ "path": "/dati" }),
        };
        let claude = tool_result(&call, "una riga", true);
        assert_eq!(claude["type"], "tool_result");
        assert_eq!(claude["tool_use_id"], "toolu_1");

        let ollama = tool_result(&call, "una riga", false);
        assert_eq!(ollama["role"], "tool");
        assert_eq!(ollama["tool_name"], "list_dir");
    }

    #[test]
    fn an_empty_text_block_never_reaches_claude() {
        let reply = Reply::new(
            "   ".to_string(),
            false,
            None,
            vec![ToolCall {
                id: "toolu_1".to_string(),
                name: "list_dir".to_string(),
                input: serde_json::json!({ "path": "/dati" }),
            }],
        );
        let message = assistant_with_tools(&reply, true);
        let parts = message["content"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["type"], "tool_use");
    }


    #[test]
    fn links_are_found_in_the_message() {
        let found = links_in("guarda https://esempio.it/pagina?a=1 e anche http://altro.it/x, poi dimmi");
        assert_eq!(found, vec!["https://esempio.it/pagina?a=1", "http://altro.it/x"]);

        // La punteggiatura finale non entra nell'indirizzo, i duplicati non si ripetono.
        assert_eq!(links_in("apri https://esempio.it/a. e https://esempio.it/a."), vec!["https://esempio.it/a"]);
        assert!(links_in("nessun link qui").is_empty());
    }

    #[test]
    fn the_two_line_format_is_read() {
        assert_eq!(
            clean_query("SERVE: si
QUERY: meteo Torino domani"),
            Some("meteo Torino domani".to_string())
        );
        assert_eq!(clean_query("SERVE: no
QUERY: -"), None);
        // SERVE comanda: una query rimasta attaccata per inerzia si ignora.
        assert_eq!(clean_query("SERVE: no
QUERY: meteo Torino"), None);
        // Un modello che scrive "Sì" invece di "si" va capito lo stesso.
        assert_eq!(clean_query("SERVE: sì
QUERY: prezzo bitcoin"), Some("prezzo bitcoin".to_string()));
    }

    #[test]
    fn planner_preambles_are_stripped() {
        assert_eq!(
            clean_query("le parole chiave da cercare sono: \"ultime notizie 29 agosto 2026\""),
            Some("ultime notizie 29 agosto 2026".to_string())
        );
        assert_eq!(clean_query("Query: meteo Torino"), Some("meteo Torino".to_string()));
    }

    #[test]
    fn a_motivated_no_is_still_a_no() {
        assert_eq!(clean_query("NO, non serve internet"), None);
        assert_eq!(clean_query("No."), None);
        // Ma una query che comincia per "no" si cerca eccome.
        assert_eq!(clean_query("nobel per la pace 2026"), Some("nobel per la pace 2026".to_string()));
    }

    #[test]
    fn explicit_orders_to_search_are_recognized() {
        assert!(wants_search("voglio che cerchi in tempo reale"));
        assert!(wants_search("dammi le ultime fonti"));
        assert!(wants_search("Cerca su Google il prezzo"));
        assert!(!wants_search("ciao come stai"));
        assert!(!wants_search("quanto fa 12 per 12"));
    }

    #[test]
    fn a_bare_order_falls_back_to_the_question_before_it() {
        let history = vec![
            ChatTurn { role: "user".to_string(), content: "cosa e' successo oggi in Italia?".to_string() },
            ChatTurn { role: "ai".to_string(), content: "Non ho accesso...".to_string() },
        ];
        // Il messaggio e' solo un ordine: la domanda vera sta un turno indietro.
        assert_eq!(
            query_from_message("voglio che cerchi in tempo reale", &history),
            Some("cosa e' successo oggi in Italia?".to_string())
        );
        // Se invece l'ordine contiene la domanda, si usa quella.
        let q = query_from_message("cerca su internet il prezzo della RTX 5090", &history).unwrap();
        assert!(q.contains("rtx 5090"), "query inutile: {q}");
    }

    #[test]
    fn planner_answers_become_queries() {
        assert_eq!(clean_query("meteo roma domani").as_deref(), Some("meteo roma domani"));
        assert_eq!(clean_query("Query: \"prezzo bitcoin oggi\"").as_deref(), Some("prezzo bitcoin oggi"));
        // "NO" in ogni forma vuol dire: rispondi a memoria.
        assert_eq!(clean_query("NO"), None);
        assert_eq!(clean_query("no."), None);
        assert_eq!(clean_query("   "), None);
        // Il modello che risponde invece di pianificare non fa partire una ricerca.
        assert_eq!(clean_query(&"parola ".repeat(40)), None);
    }

    #[test]
    fn web_material_reaches_the_prompt() {
        let block = "RISULTATI DAL WEB\n[1] Titolo - https://esempio.it\nriassunto";
        let prompt = system_prompt(&AiConfig::default(), Some(block), false);
        assert!(prompt.contains("https://esempio.it"));
        assert!(prompt.contains("Cita le fonti"));

        // Senza ricerca il prompt non parla di fonti da citare.
        assert!(!system_prompt(&AiConfig::default(), None, false).contains("Cita le fonti"));
    }

    /// Il giro completo con internet, contro Ollama e contro il web vero:
    /// `cargo test -- --ignored live_web_flow`
    #[test]
    #[ignore]
    fn live_web_flow_searches_only_when_needed() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime tokio");
        let cfg = AiConfig {
            ollama_model: std::env::var("FOAI_TEST_MODEL").unwrap_or_else(|_| "mistral".to_string()),
            ..AiConfig::default()
        };

        // Domanda che vive di dati aggiornati: deve nascere una query.
        let query = runtime.block_on(plan_search("chi e' il presidente della repubblica italiana oggi?", &[], &cfg));
        assert!(query.is_some(), "il pianificatore non ha cercato niente");

        // Domanda che il modello sa da solo: niente ricerca, niente attesa.
        let none = runtime.block_on(plan_search("quanto fa 12 per 12?", &[], &cfg));
        assert!(none.is_none(), "ricerca inutile: {none:?}");

        // Turno intero: il testo arriva e le fonti sono quelle lette.
        let cancel = AtomicBool::new(false);
        let mut statuses: Vec<String> = Vec::new();
        let reply = runtime
            .block_on(stream_reply(
                "test_live_web",
                "chi ha vinto il campionato di Serie A 2024/2025?",
                &[],
                &[],
                &cfg,
                &cancel,
                |_| {},
                |s| statuses.push(s.to_string()),
            ))
            .expect("turno fallito");

        assert!(!reply.text.trim().is_empty(), "risposta vuota");
        assert!(statuses.iter().any(|s| s.starts_with("Cerco su internet")), "nessuna ricerca: {statuses:?}");
        assert!(!reply.sources.is_empty(), "nessuna fonte: stati {statuses:?}, testo {}", reply.text);
        eprintln!("risposta: {}\nfonti: {:?}", reply.text, reply.sources);
    }

    /// Turno vero contro Ollama: verifica lo streaming end-to-end, che i test
    /// offline non toccano. Richiede `ollama serve` e il modello scaricato:
    /// `cargo test -- --ignored live_ollama_stream`
    #[test]
    #[ignore]
    fn live_ollama_stream_arrives_in_pieces() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime tokio");
        let cfg = AiConfig {
            ollama_model: std::env::var("FOAI_TEST_MODEL").unwrap_or_else(|_| "mistral".to_string()),
            ..AiConfig::default()
        };

        let cancel = AtomicBool::new(false);
        let mut pieces = 0usize;
        let reply = runtime
            .block_on(stream_reply(
                "test_live_ollama",
                "Rispondi solo con la parola: pronto",
                &[],
                &[],
                &cfg,
                &cancel,
                |_| pieces += 1,
                |_| {},
            ))
            .expect("streaming fallito");

        assert!(!reply.text.trim().is_empty(), "risposta vuota");
        assert!(pieces > 1, "il testo non e' arrivato in streaming ({pieces} pezzi)");
        assert!(!reply.stopped);
        assert!(reply.error.is_none(), "errore: {:?}", reply.error);
    }

    #[test]
    fn missing_model_gets_a_useful_message() {
        let hint = missing_model_hint(reqwest::StatusCode::NOT_FOUND, "not found", "llama3.2");
        assert!(hint.contains("ollama pull llama3.2"));
        let other = missing_model_hint(reqwest::StatusCode::INTERNAL_SERVER_ERROR, "boom", "llama3.2");
        assert!(other.contains("500"));
    }
}
