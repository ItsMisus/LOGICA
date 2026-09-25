//! Gli strumenti che l'assistente puo' usare per guardare davvero sul disco.
//!
//! Fino a qui il modello poteva solo dire "non ho accesso ai tuoi file". Il
//! recinto per farlo davvero esisteva gia' (`server::guard`, scritto per
//! l'accesso da browser): qui lo si riusa tale e quale, cosi' la finestra
//! desktop e la rete hanno un solo confine e non due implementazioni che
//! divergono.
//!
//! Tutti gli strumenti sono in **sola lettura**: elencano, leggono, cercano.
//! Niente sposta, rinomina o cancella. Una scrittura decisa da un modello
//! locale da 7B senza una conferma davanti agli occhi dell'utente e' un
//! rischio che non vale la comodita'.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::server::guard;

/// Voci mostrate per una cartella. Oltre, il modello non legge comunque e il
/// contesto si riempie di nomi.
const MAX_ENTRIES: usize = 200;

/// Risultati riportati da una ricerca (per nome o per contenuto).
const MAX_RESULTS: usize = 40;

/// Quanto si scende sotto la cartella di partenza in una ricerca ricorsiva.
const MAX_DEPTH: usize = 6;

/// File aperti in tutto da una singola ricerca: un recinto grande con dentro
/// `node_modules` bloccherebbe la risposta per minuti.
const MAX_VISITED: usize = 4_000;

/// Byte letti da un file per cercarci dentro. Oltre e' quasi certamente un
/// binario o un log enorme, e in entrambi i casi non serve.
const MAX_SCAN_BYTES: u64 = 2 * 1024 * 1024;

/// Cartelle che in una ricerca sono solo rumore e costano moltissimo tempo.
const SKIP_DIRS: &[&str] =
    &["node_modules", ".git", "target", "dist", "build", ".venv", "__pycache__", ".next", ".cache"];

/// Passi di strumento in un solo turno. Serve un tetto: un modello confuso
/// rifarebbe la stessa `list_dir` all'infinito.
pub const MAX_STEPS: usize = 6;

/// Una chiamata a uno strumento, come esce dal modello.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// Id assegnato da Claude (`toolu_...`). Ollama non ne manda: resta vuoto.
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

impl ToolCall {
    fn arg(&self, key: &str) -> Option<String> {
        self.input[key].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    /// Come `arg`, ma per i percorsi: passa da `tidy_path`.
    fn path_arg(&self, key: &str) -> Option<String> {
        self.arg(key).map(|p| tidy_path(&p)).filter(|p| !p.is_empty())
    }
}

/// Raddrizza un percorso scritto da un modello.
///
/// Su Windows i modelli piccoli raddoppiano le barre rovesciate — scrivono
/// `C:\\Users\\...` dentro un JSON che le raddoppia a sua volta — e quello che
/// arriva qui e' un percorso che non esiste. Dentro un percorso vero due barre
/// di fila non compaiono mai: compaiono solo in testa, nei percorsi di rete,
/// e quelle si lasciano stare.
fn tidy_path(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('"');
    let (prefix, rest) = match trimmed.strip_prefix(r"\\") {
        Some(rest) => (r"\\", rest),
        None => ("", trimmed),
    };

    let mut out = String::with_capacity(rest.len());
    let mut after_backslash = false;
    for c in rest.chars() {
        if c == '\\' {
            if !after_backslash {
                out.push(c);
            }
            after_backslash = true;
        } else {
            out.push(c);
            after_backslash = false;
        }
    }
    format!("{prefix}{out}")
}

/* ------------------------------ dichiarazioni ----------------------------- */

/// Le definizioni degli strumenti nel formato di Claude. Ollama usa lo stesso
/// JSON Schema avvolto in `{"type":"function","function":{...}}`, quindi si
/// scrive una volta sola e si converte.
fn definitions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "list_dir",
            "description": "Elenca il contenuto di una cartella: nomi, se sono file o cartelle, dimensione e data di modifica. Usalo per esplorare prima di leggere.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Percorso assoluto della cartella." }
                },
                "required": ["path"]
            }
        }),
        serde_json::json!({
            "name": "read_file",
            "description": "Legge il contenuto testuale di un file. Funziona su testo, sorgenti, PDF e DOCX. Il testo lungo viene tagliato.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Percorso assoluto del file." }
                },
                "required": ["path"]
            }
        }),
        serde_json::json!({
            "name": "find_files",
            "description": "Cerca file e cartelle il cui nome contiene un testo, scendendo nelle sottocartelle.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Pezzo di nome da cercare, senza asterischi." },
                    "path": { "type": "string", "description": "Cartella da cui partire. Se manca, tutte le cartelle autorizzate." }
                },
                "required": ["name"]
            }
        }),
        serde_json::json!({
            "name": "search_in_files",
            "description": "Cerca un testo dentro i file e riporta il file e la riga in cui compare.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Testo da cercare dentro i file." },
                    "path": { "type": "string", "description": "Cartella da cui partire. Se manca, tutte le cartelle autorizzate." }
                },
                "required": ["text"]
            }
        }),
    ]
}

/// Formato Claude: le definizioni cosi' come sono.
pub fn claude_tools() -> Vec<serde_json::Value> {
    definitions()
}

/// Formato Ollama: `input_schema` si chiama `parameters` e sta dentro
/// `function`.
pub fn ollama_tools() -> Vec<serde_json::Value> {
    definitions()
        .into_iter()
        .map(|d| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": d["name"],
                    "description": d["description"],
                    "parameters": d["input_schema"],
                }
            })
        })
        .collect()
}

/// Gli strumenti hanno senso solo se c'e' almeno una cartella autorizzata:
/// senza, ogni chiamata tornerebbe lo stesso rifiuto e il modello ci
/// sbatterebbe contro per sei giri.
pub fn available() -> bool {
    !guard::roots().is_empty()
}

/// I nomi delle cartelle autorizzate, in minuscolo.
///
/// Servono al freno che decide se dichiarare gli strumenti: quando l'utente
/// nomina una di queste ("cosa c'e' in Documenti?") la richiesta riguarda il
/// disco anche senza la parola "file". I nomi corti si buttano via: una
/// cartella che si chiama "AI" comparirebbe dentro mezzo vocabolario.
pub fn roots_names() -> Vec<String> {
    guard::roots()
        .iter()
        .filter_map(|r| r.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .filter(|n| n.chars().count() >= 4)
        .collect()
}

/// Il pezzo di prompt che dice al modello dove puo' guardare. Senza l'elenco,
/// il modello inventa percorsi plausibili e si prende un rifiuto dopo l'altro.
pub fn roots_block() -> String {
    let list: Vec<String> = guard::roots().iter().map(|r| format!("- {}", display_path(r))).collect();
    format!(
        "STRUMENTI SUI FILE\nHai accesso in sola lettura a queste cartelle e a tutto quello che contengono:\n{}\n\nQuando la domanda riguarda file, cartelle o documenti dell'utente, guarda davvero con gli strumenti invece di dire che non puoi.

Non inventare percorsi: un percorso indovinato non esiste e ti fa perdere un giro. Se l'utente nomina un file senza dire dove sta, cercalo con find_files; se nomina una cartella, aprila con list_dir. Solo quando hai il percorso vero, letto da uno di quei due, usa read_file.

Nei percorsi scrivi una barra rovesciata sola: C:\\cartella\\file.txt, non C:\\\\cartella\\\\file.txt.

Non puoi scrivere, spostare o cancellare niente: se te lo chiedono, dillo.",
        list.join("\n")
    )
}

/// Quanti percorsi entrano nell'elenco messo nel prompt. Oltre questo numero
/// l'elenco costa piu' contesto di quanto valga, e il modello smette di
/// leggerlo fino in fondo.
const MAX_TREE_FILES: usize = 120;

/// L'elenco dei file che ci sono davvero nelle cartelle autorizzate.
///
/// Il primo anello della catena — trovare il file — e' quello dove i modelli
/// piccoli si fermano: chiedono `find_files` con la parola sbagliata, non
/// trovano niente e concludono "non so dove si trova" invece di riprovare.
/// L'elenco costa una visita del disco e nessuna chiamata al modello, quindi
/// il primo anello lo fa il programma: al modello resta `read_file` su un
/// percorso che esiste di sicuro.
pub fn tree_block() -> String {
    let roots = guard::roots();
    if roots.is_empty() {
        return String::new();
    }
    let mut files: Vec<String> = Vec::new();
    let mut total = 0usize;
    let mut visited = 0usize;
    for base in &roots {
        walk(base, 0, &mut visited, &mut |entry: &Path, is_dir: bool| {
            if is_dir {
                return;
            }
            total += 1;
            if files.len() < MAX_TREE_FILES {
                files.push(display_path(entry));
            }
        });
    }
    if files.is_empty() {
        return "I FILE CHE CI SONO\nLe cartelle autorizzate sono vuote: non c'e' niente da aprire, e dirlo e' la risposta giusta.".to_string();
    }
    files.sort_by_key(|p| p.to_lowercase());
    let mut out = String::from(
        "I FILE CHE CI SONO\nQuesti sono i percorsi veri dei file nelle cartelle autorizzate, letti dal disco adesso:\n",
    );
    for row in &files {
        out.push_str(row);
        out.push('\n');
    }
    if total > files.len() {
        out.push_str(&format!("... altri {} file non elencati.\n", total - files.len()));
    }
    out.push_str(
        "\nSe il file che serve e' in questo elenco, chiama subito read_file con quel percorso: non cercarlo prima con find_files, e non dire che non sai dove si trova. Se non c'e', puo' essere fra quelli non elencati: cercalo con find_files o search_in_files.",
    );
    out
}

/// Parole che compaiono in ogni domanda e non dicono niente su cosa cercare.
/// Senza questa lista la ricerca parte da "quanto" e riporta mezza cartella.
const STOP_WORDS: [&str; 67] = [
    "quanto", "quanta", "quanti", "quante", "quale", "quali", "cosa", "come", "dove", "chi",
    "perche", "quando", "dimmi", "dammi", "scrivimi", "trova", "cerca", "apri", "apre", "aprilo",
    "elenca", "elencami", "sommami", "file", "files", "cartella", "cartelle", "documento",
    "documenti", "della", "delle", "dello", "degli", "dalla", "dalle", "nella", "nelle", "nello",
    "negli", "questo", "questa", "quello", "quella", "sono", "essere", "stato", "servono",
    "serve", "supera", "compare", "ancora", "ultimo", "ultima", "ultime", "ultimi", "volta",
    "adesso", "oggi", "miei", "mia", "mio", "mie",
    // Articoli elisi: da "dell'auto" esce "dell", che come sottostringa
    // combacia con "della"/"delle" in mezzo mondo e falsa il punteggio.
    "dell", "dall", "sull", "nell", "coll",
];

/// Quante parole della domanda si portano dietro nella ricerca, e quante
/// righe si riportano al modello.
const MAX_HINT_WORDS: usize = 8;
const MAX_HINT_LINES: usize = 14;

/// Quante finestre al massimo puo' contribuire un singolo file, cosi' che una
/// domanda su piu' file non venga soffocata dal file che combacia di piu'.
const MAX_HINT_PER_FILE: usize = 3;

/// Righe raccolte prima di scegliere le migliori. Serve un tetto perche' una
/// parola comune dentro un recinto grande le trova a migliaia.
const MAX_HINT_CANDIDATES: usize = 400;

/// Le parole della domanda con cui ha senso cercare, ridotte alla radice.
///
/// La radice e' i primi sei caratteri: l'italiano declina, e chi chiede delle
/// "fatture" ha in un file la parola "fattura". Tagliare corto costa qualche
/// riga di troppo; non tagliare costa la riga giusta.
fn hint_words(question: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for raw in question.to_lowercase().split(|c: char| !c.is_alphanumeric()) {
        if raw.chars().count() < 4 || STOP_WORDS.contains(&raw) {
            continue;
        }
        let stem: String = raw.chars().take(6).collect();
        if !words.contains(&stem) {
            words.push(stem);
        }
        if words.len() >= MAX_HINT_WORDS {
            break;
        }
    }
    words
}

/// Cosa e' stato cercato, da mostrare accanto alla risposta: le stesse parole
/// che ha usato la ricerca, non una parafrasi.
pub fn hint_summary(question: &str) -> String {
    hint_words(question).join(", ")
}

/// Le righe dei file dell'utente che parlano di quello che ha chiesto.
///
/// E' `search_in_files` fatto dal programma invece che dal modello, e per
/// tutte le parole della domanda in una passata sola. Serve al secondo anello
/// della catena, quello dove i modelli piccoli si fermano: aprono un file, non
/// trovano dentro tutto quello che serve e concludono "non ho trovato" invece
/// di aprire il secondo. Qui le righe dei due file ci sono gia' tutte e due.
///
/// Le righe sono isolate apposta: chi legge deve poter decidere se gli basta
/// o se il file va aperto davvero.
///
/// Una riga del prefetch: `file:numero: testo`, col testo tagliato perche' una
/// riga lunghissima non mangi il posto delle altre.
/// La domanda chiede una cifra? Solo allora si portano su le righe con un
/// importo dai file gia' promossi: per "di che colore sono le finestre" un
/// prezzo non c'entra niente.
fn wants_amount(question: &str) -> bool {
    const SEGNI: [&str; 12] = [
        "spes", "cost", "somma", "paga", "prezz", "total", "euro", "import", "canone", "tariff",
        "budget", "quant",
    ];
    let low = question.to_lowercase();
    SEGNI.iter().any(|s| low.contains(s)) || question.contains('\u{20ac}')
}

/// Una riga che porta un importo: una cifra accanto a "euro" o al simbolo. Non
/// una data e non un numero qualsiasi - "4 marzo 2027" non e' un costo.
fn is_money_line(line: &str) -> bool {
    let low = line.to_lowercase();
    if !low.contains("euro") && !line.contains('\u{20ac}') {
        return false;
    }
    line.chars().any(|c| c.is_ascii_digit())
}

fn row_at(entry: &Path, line_index: usize, line: &str) -> String {
    format!(
        "{}:{}: {}",
        display_path(entry),
        line_index + 1,
        line.trim().chars().take(200).collect::<String>()
    )
}

pub fn hints_block(question: &str) -> String {
    let words = hint_words(question);
    if words.is_empty() {
        return String::new();
    }
    let roots = guard::roots();
    if roots.is_empty() {
        return String::new();
    }

    // Riga, e quante parole diverse della domanda contiene: una riga che le
    // contiene tutte e due vale piu' di due righe che ne contengono una.
    //
    // Si raccoglie molto piu' di quanto se ne terra': il tetto vero e' quello
    // sulle righe riportate, e tagliare gia' qui vorrebbe dire scegliere le
    // prime incontrate invece delle migliori.
    // Ogni riga che c'entra si porta dietro una piccola finestra: se stessa e
    // fino a due righe non vuote subito sotto. Il numero che risponde alla
    // domanda quasi mai sta sulla riga con la parola chiave ("manutenzione
    // della caldaia") ma su quella dopo ("Costo: 118 euro"): senza la finestra
    // il modello vede il nome della cosa e non la sua cifra, e va ad aprire il
    // file per una cosa che gli avremmo gia' potuto dare.
    // Quando la domanda chiede una cifra, in un file che c'entra la cifra e'
    // quasi sempre la risposta - ma puo' stare lontana dalla parola chiave
    // ("Costo: 118 euro" otto righe sotto "manutenzione della caldaia"), fuori
    // dalla finestra. Allora, nei soli file che hanno gia' una trovata, si
    // portano su anche le righe che contengono un importo.
    let money = wants_amount(question);
    let mut hits: Vec<(usize, String, Vec<String>)> = Vec::new();
    let mut visited = 0usize;
    for base in &roots {
        walk(base, 0, &mut visited, &mut |entry: &Path, is_dir: bool| {
            if is_dir || hits.len() >= MAX_HINT_CANDIDATES {
                return;
            }
            let Ok(meta) = entry.metadata() else { return };
            if meta.len() > MAX_SCAN_BYTES {
                return;
            }
            let Ok(content) = std::fs::read(entry) else { return };
            if content.iter().take(1024).any(|b| *b == 0) {
                return;
            }
            let text = String::from_utf8_lossy(&content);
            let lines: Vec<&str> = text.lines().collect();
            let file = display_path(entry);
            let mut file_hits: Vec<(usize, Vec<String>)> = Vec::new();
            for n in 0..lines.len() {
                let lower = lines[n].to_lowercase();
                let score = words.iter().filter(|w| lower.contains(w.as_str())).count();
                if score == 0 {
                    continue;
                }
                let mut window = vec![row_at(entry, n, lines[n])];
                let mut added = 0;
                for l in lines.iter().skip(n + 1) {
                    if added == 2 {
                        break;
                    }
                    if l.trim().is_empty() {
                        continue;
                    }
                    // La riga seguente conta come contorno solo se non e' gia'
                    // una sua trovata: due parole chiave attaccate restano due
                    // finestre distinte, non una che mangia l'altra.
                    if words.iter().any(|w| l.to_lowercase().contains(w.as_str())) {
                        break;
                    }
                    added += 1;
                    window.push(row_at(entry, n + added, l));
                }
                file_hits.push((score, window));
            }
            if file_hits.is_empty() {
                return;
            }
            // Solo ora, a file gia' promosso: le sue righe con un importo.
            if money {
                for (n, l) in lines.iter().enumerate() {
                    if is_money_line(l) {
                        file_hits.push((1, vec![row_at(entry, n, l)]));
                    }
                }
            }
            for (score, window) in file_hits {
                hits.push((score, file.clone(), window));
            }
        });
    }

    if hits.is_empty() {
        return String::new();
    }
    // Le righe che parlano di piu' cose della domanda per prime: quando il
    // tetto taglia, taglia quelle che c'entrano di meno.
    hits.sort_by_key(|a| std::cmp::Reverse(a.0));

    // Il tetto e' anche per file, non solo complessivo. Una domanda che chiede
    // tre cose da tre file ("tagliando, caldaia, alloggio") va persa se un
    // solo file, con piu' righe che combaciano, si prende tutti i posti: il
    // modello riceve due cifre su tre e si ferma a chiedere la terza. Con un
    // tetto per file ognuno dei tre entra.
    let mut per_file: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut kept: Vec<&Vec<String>> = Vec::new();
    for (_, file, window) in &hits {
        if kept.len() >= MAX_HINT_LINES {
            break;
        }
        let count = per_file.entry(file.as_str()).or_insert(0);
        if *count >= MAX_HINT_PER_FILE {
            continue;
        }
        *count += 1;
        kept.push(window);
    }

    let mut out = String::from(
        "RIGHE CHE PARLANO DELLA DOMANDA\nLe ha cercate l'app nei file dell'utente, prima di passarti la domanda:\n",
    );
    // Una riga di contorno di una finestra puo' essere la riga chiave di
    // un'altra: si stampa una volta sola, cosi' il conteggio non inganna.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for window in &kept {
        for row in window.iter() {
            if seen.insert(row.as_str()) {
                out.push_str(row);
                out.push('\n');
            }
        }
    }
    out.push_str(
        "\nSono righe staccate dal loro file: se bastano, rispondi con queste e cita il file. Se la domanda chiede piu' cose e qui c'e' solo una parte, apri con read_file gli altri file che possono contenere il resto prima di rispondere. Non dire che non hai trovato niente finche' non li hai aperti.",
    );
    out
}

/// Gli strumenti che esistono davvero. Serve a distinguere una chiamata
/// scritta a mano da un modello che si e' inventato lo strumento: `llama3.1:8b`
/// produce `{"name": "math", ...}`, e `math` non e' mai esistito.
pub fn exists(name: &str) -> bool {
    definitions().iter().any(|d| d["name"].as_str() == Some(name))
}

/// Legge il JSON di una chiamata scritta a mano, anche quando le barre dei
/// percorsi Windows non sono raddoppiate.
///
/// `llama3.1:8b` scrive tutte e due le forme. In `web-06` esce
/// `"C:\Users\franc"`, giusto; in `chat-07` esce `"C:\Users\franc"`, che JSON
/// non e': `\U` non e' una sequenza di escape valida. Rifiutare la seconda
/// forma vorrebbe dire lasciar passare a schermo proprio le chiamate che
/// questo modulo esiste per fermare — chat-07, chat-14 e chat-20 sono tutte
/// cosi'.
///
/// Quindi due passate. La prima e' quella stretta, e vince sempre quando il
/// testo e' JSON valido: un `\n` scritto apposta resta un a capo. Solo se
/// fallisce si riprova considerando ogni barra solitaria come una barra vera.
/// La barra davanti a una virgoletta o a un'altra barra si lascia com'e':
/// raddoppiarla chiuderebbe la stringa nel punto sbagliato.
fn leggi(testo: &str) -> Option<serde_json::Value> {
    if let Ok(valore) = serde_json::from_str(testo) {
        return Some(valore);
    }
    serde_json::from_str(&barre_letterali(testo)).ok()
}

fn barre_letterali(testo: &str) -> String {
    let mut fuori = String::with_capacity(testo.len() + 16);
    let mut dentro_stringa = false;
    let mut resto = testo.chars().peekable();
    while let Some(c) = resto.next() {
        match c {
            '"' => {
                dentro_stringa = !dentro_stringa;
                fuori.push(c);
            }
            '\\' if dentro_stringa => match resto.peek() {
                // Gia' a posto, o una virgoletta protetta: si copiano tutte e
                // due e non si tocca niente.
                Some('\\') | Some('"') => {
                    fuori.push(c);
                    fuori.push(resto.next().unwrap());
                }
                _ => fuori.push_str("\\\\"),
            },
            _ => fuori.push(c),
        }
    }
    fuori
}

/// Una chiamata a uno strumento che il modello ha scritto nel testo invece di
/// mandarla nel campo apposta.
///
/// Non e' un caso raro da manuale: nella misura delle cento richieste,
/// `llama3.1:8b` l'ha fatto quattro volte solo fra le domande di chiacchiera, e
/// il risultato finiva stampato in chat cosi' com'era — `{"name": "read_file",
/// "parameters": {...}}` al posto di una risposta. Riconoscerla serve a due
/// cose: eseguirla quando nomina uno strumento vero, e comunque non mostrarla
/// mai all'utente.
///
/// I nomi del contenitore degli argomenti sono tre perche' tre se ne vedono:
/// `parameters` e' quello che scrive llama, `arguments` quello del formato
/// vero di Ollama, `input` quello di Claude.
pub fn handwritten_call(text: &str) -> Option<ToolCall> {
    let mut clean = text.trim();
    // Qualche modello la incornicia come blocco di codice.
    if let Some(rest) = clean.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        clean = rest.trim().trim_end_matches("```").trim();
    }

    let value: serde_json::Value = leggi(clean)?;
    // Con o senza l'involucro `function` del formato di Ollama.
    let body = if value.get("function").is_some() { &value["function"] } else { &value };

    let name = body["name"].as_str()?;
    if !exists(name) {
        return None;
    }

    let input = ["parameters", "arguments", "input"]
        .iter()
        .find_map(|k| match &body[*k] {
            // Gli argomenti a volte sono una stringa da riaprire, come nel
            // formato vero.
            serde_json::Value::String(t) => leggi(t),
            v @ serde_json::Value::Object(_) => Some(v.clone()),
            _ => None,
        })
        .unwrap_or_else(|| serde_json::json!({}));

    Some(ToolCall { id: String::new(), name: name.to_string(), input })
}

/* -------------------------------- esecuzione ------------------------------ */

/// Riga mostrata in chat mentre lo strumento gira. Il modello ci mette
/// qualche secondo per ogni passo: senza, sembra fermo.
pub fn status_line(call: &ToolCall) -> String {
    match call.name.as_str() {
        "list_dir" => format!("Guardo in {}", call.path_arg("path").unwrap_or_default()),
        "read_file" => format!("Leggo {}", call.path_arg("path").unwrap_or_default()),
        "find_files" => format!("Cerco file: {}", call.arg("name").unwrap_or_default()),
        "search_in_files" => format!("Cerco nei file: {}", call.arg("text").unwrap_or_default()),
        other => format!("Uso {other}"),
    }
}

/// Esegue una chiamata e torna il testo da rimandare al modello.
///
/// Gli errori non sono `Err`: tornano come testo, perche' "quella cartella non
/// esiste" e' un'informazione con cui il modello puo' correggersi al passo
/// dopo, mentre un errore duro interromperebbe la risposta.
pub fn run(call: &ToolCall) -> String {
    match call.name.as_str() {
        "list_dir" => match call.path_arg("path") {
            Some(path) => list_dir(&path),
            None => "Errore: manca il parametro 'path'.".to_string(),
        },
        "read_file" => match call.path_arg("path") {
            Some(path) => read_file(&path),
            None => "Errore: manca il parametro 'path'.".to_string(),
        },
        "find_files" => match call.arg("name") {
            Some(name) => find_files(&name, call.path_arg("path")),
            None => "Errore: manca il parametro 'name'.".to_string(),
        },
        "search_in_files" => match call.arg("text") {
            Some(text) => search_in_files(&text, call.path_arg("path")),
            None => "Errore: manca il parametro 'text'.".to_string(),
        },
        other => format!("Errore: lo strumento '{other}' non esiste."),
    }
}

fn list_dir(path: &str) -> String {
    let dir = match guard::ensure_within(path) {
        Ok(d) => d,
        Err(e) => return format!("Errore: {e}"),
    };
    if !dir.exists() {
        return missing(&dir);
    }
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => return format!("Errore: cartella non leggibile ({e})."),
    };

    let mut rows: Vec<(bool, String)> = Vec::new();
    let mut total = 0usize;
    for entry in entries.flatten() {
        total += 1;
        if rows.len() >= MAX_ENTRIES {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = entry.metadata().ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let row = if is_dir {
            format!("{name}/")
        } else {
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let when = meta.as_ref().and_then(modified_on).unwrap_or_default();
            format!("{name}  {}  {when}", human_size(size))
        };
        rows.push((is_dir, row));
    }

    if rows.is_empty() {
        return format!("{}: cartella vuota.", display_path(&dir));
    }
    // Cartelle prima: e' l'ordine in cui si esplora.
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));

    let mut out = format!("{} ({total} voci)\n", display_path(&dir));
    for (_, row) in &rows {
        out.push_str(row);
        out.push('\n');
    }
    if total > rows.len() {
        out.push_str(&format!("... altre {} voci non elencate.\n", total - rows.len()));
    }
    out
}

fn read_file(path: &str) -> String {
    let file = match guard::ensure_within(path) {
        Ok(f) => f,
        Err(e) => return format!("Errore: {e}"),
    };
    if !file.exists() {
        return missing(&file);
    }
    if file.is_dir() {
        return format!("Errore: {} e' una cartella. Usa list_dir.", display_path(&file));
    }
    // Stessa lettura degli allegati: PDF, DOCX e testo non UTF-8 sono gia'
    // risolti li' una volta sola.
    let read = crate::attachments::read_all(&[file.to_string_lossy().to_string()]);
    let Some(attachment) = read.into_iter().next() else {
        return "Errore: lettura fallita.".to_string();
    };
    if let Some(err) = attachment.error {
        return format!("Errore: {err}");
    }
    if attachment.image.is_some() {
        return format!(
            "{} e' un'immagine: gli strumenti non la possono guardare. L'utente puo' allegarla al messaggio.",
            attachment.name
        );
    }
    match attachment.text {
        Some(text) if !text.trim().is_empty() => {
            let cut =
                if attachment.truncated { "\n[...] file tagliato: continua oltre." } else { "" };
            format!("{} ({})\n\n{text}{cut}", display_path(&file), human_size(attachment.size))
        }
        _ => format!("{}: nessun testo leggibile.", display_path(&file)),
    }
}

fn find_files(needle: &str, path: Option<String>) -> String {
    let bases = match starting_points(path) {
        Ok(b) => b,
        Err(e) => return format!("Errore: {e}"),
    };
    let needle = needle.to_lowercase();
    let mut found: Vec<String> = Vec::new();
    let mut visited = 0usize;

    for base in &bases {
        walk(base, 0, &mut visited, &mut |entry: &Path, is_dir: bool| {
            if found.len() >= MAX_RESULTS {
                return;
            }
            let name =
                entry.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            if name.contains(&needle) {
                let suffix = if is_dir { "/" } else { "" };
                found.push(format!("{}{suffix}", display_path(entry)));
            }
        });
    }

    if found.is_empty() {
        return format!("Nessun file o cartella con \"{needle}\" nel nome.");
    }
    let mut out = format!("{} risultati per \"{needle}\":\n", found.len());
    for row in &found {
        out.push_str(row);
        out.push('\n');
    }
    if found.len() >= MAX_RESULTS {
        out.push_str("(elenco tagliato: restringi la ricerca)\n");
    }
    out
}

fn search_in_files(needle: &str, path: Option<String>) -> String {
    let bases = match starting_points(path) {
        Ok(b) => b,
        Err(e) => return format!("Errore: {e}"),
    };
    let lower = needle.to_lowercase();
    let mut hits: Vec<String> = Vec::new();
    let mut visited = 0usize;

    for base in &bases {
        walk(base, 0, &mut visited, &mut |entry: &Path, is_dir: bool| {
            if is_dir || hits.len() >= MAX_RESULTS {
                return;
            }
            let Ok(meta) = entry.metadata() else { return };
            if meta.len() > MAX_SCAN_BYTES {
                return;
            }
            let Ok(content) = std::fs::read(entry) else { return };
            // I binari non si cercano: un PNG pieno di byte casuali produce
            // "righe" lunghe migliaia di caratteri.
            if content.iter().take(1024).any(|b| *b == 0) {
                return;
            }
            let text = String::from_utf8_lossy(&content);
            for (n, line) in text.lines().enumerate() {
                if hits.len() >= MAX_RESULTS {
                    return;
                }
                if line.to_lowercase().contains(&lower) {
                    hits.push(format!(
                        "{}:{}: {}",
                        display_path(entry),
                        n + 1,
                        line.trim().chars().take(200).collect::<String>()
                    ));
                }
            }
        });
    }

    if hits.is_empty() {
        return format!("\"{needle}\" non compare in nessun file letto.");
    }
    let mut out = format!("{} righe con \"{needle}\":\n", hits.len());
    for row in &hits {
        out.push_str(row);
        out.push('\n');
    }
    if hits.len() >= MAX_RESULTS {
        out.push_str("(elenco tagliato: restringi la ricerca)\n");
    }
    out
}

/// Un percorso che non esiste e' quasi sempre un percorso indovinato. Dirlo
/// cosi' — invece di lasciar passare il "file non trovato" del sistema, che un
/// modello legge come "non hai i permessi" — gli fa fare il passo giusto
/// invece di rispondere che non ci riesce.
fn missing(path: &Path) -> String {
    let parent = path
        .parent()
        .map(|p| format!(" Elenca prima {} con list_dir,", display_path(p)))
        .unwrap_or_default();
    format!(
        "Errore: {} non esiste. Non hai i permessi: hai indovinato il percorso.{parent} oppure cerca il file per nome con find_files.",
        display_path(path)
    )
}

/* -------------------------------- utilita' -------------------------------- */

/// Da dove parte una ricerca: la cartella chiesta, verificata, oppure tutte le
/// radici autorizzate quando non e' stata indicata.
fn starting_points(path: Option<String>) -> Result<Vec<PathBuf>, String> {
    match path {
        Some(p) => guard::ensure_within(&p).map(|p| vec![p]),
        None => {
            let roots = guard::roots();
            if roots.is_empty() {
                Err("nessuna cartella autorizzata.".to_string())
            } else {
                Ok(roots)
            }
        }
    }
}

/// Visita ricorsiva dei soli file, per chi deve leggerli tutti: l'indice dei
/// documenti. Stessi salti di cartella, tetti molto piu' alti — qui non si sta
/// rispondendo a una domanda, si sta preparando un indice, e un file saltato
/// vuol dire un documento che poi non si trovera' mai.
pub fn walk_files(dir: &Path, visited: &mut usize, on_file: &mut impl FnMut(&Path)) {
    walk_with(dir, 0, MAX_INDEX_DEPTH, MAX_INDEX_VISITS, visited, &mut |path, is_dir| {
        if !is_dir {
            on_file(path);
        }
    });
}

/// Profondita' e file visitati durante l'indicizzazione.
const MAX_INDEX_DEPTH: usize = 12;
const MAX_INDEX_VISITS: usize = 50_000;

/// Visita ricorsiva con i freni: profondita', numero di file visitati e le
/// cartelle che non servono mai.
fn walk(dir: &Path, depth: usize, visited: &mut usize, on_entry: &mut impl FnMut(&Path, bool)) {
    walk_with(dir, depth, MAX_DEPTH, MAX_VISITED, visited, on_entry);
}

fn walk_with(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    max_visited: usize,
    visited: &mut usize,
    on_entry: &mut impl FnMut(&Path, bool),
) {
    if depth > max_depth || *visited >= max_visited {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if *visited >= max_visited {
            return;
        }
        *visited += 1;
        let path = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let name = entry.file_name().to_string_lossy().to_string();
        if is_dir && SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        on_entry(&path, is_dir);
        if is_dir {
            walk_with(&path, depth + 1, max_depth, max_visited, visited, on_entry);
        }
    }
}

/// Il prefisso `\\?\` che Windows mette nei percorsi canonici e' corretto ma
/// illeggibile, e il modello poi lo ricopia nelle risposte.
fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn modified_on(meta: &std::fs::Metadata) -> Option<String> {
    let modified = meta.modified().ok()?;
    let stamp: chrono::DateTime<chrono::Local> = modified.into();
    Some(stamp.format("%d/%m/%Y").to_string())
}

/* -------------------------------- risposta -------------------------------- */

/// Quello che il frontend mostra sotto la risposta: cosa ha guardato l'app.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
pub struct ToolUse {
    pub name: String,
    /// Il parametro principale, gia' pronto da mostrare (un percorso, un testo
    /// cercato).
    pub detail: String,
}

impl ToolUse {
    pub fn of(call: &ToolCall) -> Self {
        let detail = call
            .path_arg("path")
            .or_else(|| call.arg("name"))
            .or_else(|| call.arg("text"))
            .unwrap_or_default();
        Self { name: call.name.clone(), detail }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dalla_domanda_restano_solo_le_parole_che_cercano_qualcosa() {
        // Le parole che stanno in ogni domanda non devono cercare niente.
        assert_eq!(hint_words("Quanto ho speso per l'alloggio a Lisbona?"), ["speso", "allogg", "lisbon"]);
        // La radice di sei caratteri fa incontrare il singolare col plurale:
        // chi chiede delle "fatture" ha in casa una "fattura".
        assert!(hint_words("Quale delle mie fatture supera i mille euro?").contains(&"fattur".to_string()));
        // Una domanda che non nomina niente non fa partire nessuna ricerca.
        assert!(hint_words("Quanto fa 17 per 23?").is_empty());
    }

    /// Le quattro che `llama3.1:8b` ha davvero prodotto nella misura, copiate
    /// dal giornale. Due nominano uno strumento vero, due no.
    #[test]
    fn a_call_written_in_the_text_is_recognised() {
        let c = handwritten_call(
            r#"{"name": "read_file", "parameters": {"path": "C:\Temp\acqua.txt"}}"#,
        )
        .expect("read_file non riconosciuta");
        assert_eq!(c.name, "read_file");
        assert_eq!(c.arg("path").as_deref(), Some(r"C:\Temp\acqua.txt".to_string()).as_deref());

        let c = handwritten_call(
            r#"{"name": "search_in_files", "parameters": {"path": "C:\Temp", "text": "effimero"}}"#,
        )
        .expect("search_in_files non riconosciuta");
        assert_eq!(c.name, "search_in_files");
        assert_eq!(c.arg("text").as_deref(), Some("effimero"));
    }

    /// Il caso che conta di piu': lo strumento non esiste. Eseguirlo non si
    /// puo', e far finta che sia una chiamata vera lascerebbe l'utente senza
    /// risposta a "quanto fa 17 per 23".
    #[test]
    fn an_invented_tool_is_not_a_call() {
        assert!(handwritten_call(r#"{"name": "math", "parameters": {"a": "17", "b": "23"}}"#)
            .is_none());
        assert!(handwritten_call(r#"{"name": "send_email", "parameters": {}}"#).is_none());
    }

    #[test]
    fn the_three_shapes_of_arguments_all_arrive() {
        // `parameters`: quello che scrive llama.
        assert!(handwritten_call(r#"{"name":"list_dir","parameters":{"path":"C:\x"}}"#).is_some());
        // `arguments` come stringa da riaprire: il formato vero di Ollama.
        let c = handwritten_call(r#"{"name":"list_dir","arguments":"{\"path\":\"C:\\x\"}"}"#)
            .expect("arguments come stringa");
        assert_eq!(c.arg("path").as_deref(), Some(r"C:\x".to_string()).as_deref());
        // `input`: il formato di Claude, dentro l'involucro `function`.
        assert!(handwritten_call(r#"{"function":{"name":"list_dir","input":{"path":"C:\x"}}}"#)
            .is_some());
    }

    /// Le due forme che llama produce davvero per lo stesso percorso. Quella
    /// con la barra singola non e' JSON valido, ma e' quella che esce piu'
    /// spesso: se non la si legge, la chiamata finisce stampata a schermo.
    #[test]
    fn a_windows_path_is_read_in_both_forms() {
        let singola = handwritten_call(
            r#"{"name": "read_file", "parameters": {"path": "C:\Users\franc\nota.txt"}}"#,
        )
        .expect("barra singola, la forma di chat-07");
        let doppia = handwritten_call(
            r#"{"name": "read_file", "parameters": {"path": "C:\\Users\\franc\\nota.txt"}}"#,
        )
        .expect("barra doppia, la forma di web-06");
        assert_eq!(singola.arg("path"), doppia.arg("path"));
        assert_eq!(singola.arg("path").as_deref(), Some(r"C:\Users\franc\nota.txt"));
    }

    #[test]
    fn a_call_inside_a_code_fence_is_still_a_call() {
        let c = handwritten_call("```json\n{\"name\": \"list_dir\", \"parameters\": {\"path\": \"C:\\x\"}}\n```")
            .expect("dentro il blocco di codice");
        assert_eq!(c.name, "list_dir");
    }

    /// Una risposta normale non deve mai essere scambiata per una chiamata.
    #[test]
    fn ordinary_prose_is_left_alone() {
        assert!(handwritten_call("Fa 391.").is_none());
        assert!(handwritten_call("Nel file c'e' scritto {\"a\": 1}, cioe' un oggetto.").is_none());
        assert!(handwritten_call("").is_none());
        assert!(handwritten_call("{}").is_none());
    }

    #[test]
    fn only_the_tools_that_exist_are_known() {
        assert!(exists("list_dir"));
        assert!(exists("read_file"));
        assert!(exists("find_files"));
        assert!(exists("search_in_files"));
        assert!(!exists("math"));
        assert!(!exists("write_file"));
    }

    #[test]
    fn ollama_tools_wrap_the_same_schema() {
        let claude = claude_tools();
        let ollama = ollama_tools();
        assert_eq!(claude.len(), ollama.len());
        assert_eq!(ollama[0]["type"], "function");
        assert_eq!(ollama[0]["function"]["name"], claude[0]["name"]);
        assert_eq!(ollama[0]["function"]["parameters"], claude[0]["input_schema"]);
    }

    #[test]
    fn unknown_tool_is_an_answer_not_a_crash() {
        let call = ToolCall {
            id: String::new(),
            name: "delete_everything".to_string(),
            input: serde_json::json!({}),
        };
        assert!(run(&call).starts_with("Errore:"));
    }

    #[test]
    fn missing_argument_is_reported_to_the_model() {
        let call = ToolCall {
            id: String::new(),
            name: "list_dir".to_string(),
            input: serde_json::json!({}),
        };
        assert!(run(&call).contains("path"));
    }

    /// Un modello locale su Windows raddoppia le barre: se non si stringono,
    /// ogni `read_file` finisce su un percorso che non esiste.
    #[test]
    fn doubled_backslashes_are_squeezed_back() {
        assert_eq!(tidy_path(r"C:\\Users\\franc\\note.txt"), r"C:\Users\franc\note.txt");
        assert_eq!(tidy_path(r"C:\Users\franc\note.txt"), r"C:\Users\franc\note.txt");
        // Un percorso di rete comincia davvero con due barre: quelle restano.
        assert_eq!(tidy_path(r"\\server\condivisa\a.txt"), r"\\server\condivisa\a.txt");
        // Virgolette e spazi che il modello si porta dietro.
        assert_eq!(tidy_path("  \"/dati/a.txt\"  "), "/dati/a.txt");
    }

    #[test]
    fn a_guessed_path_is_told_apart_from_a_denied_one() {
        let call = ToolCall {
            id: String::new(),
            name: "read_file".to_string(),
            input: serde_json::json!({ "path": r"C:\\Users\\franc\\note.txt" }),
        };
        // Fuori dal recinto o inesistente, il percorso qui non c'e': quello che
        // conta e' che il messaggio non dica "permessi", che il modello legge
        // come "arrenditi".
        let answer = run(&call);
        assert!(!answer.to_lowercase().contains("permess") || answer.contains("hai indovinato"));
    }

    #[test]
    fn missing_says_what_to_do_next() {
        let text = missing(Path::new(r"C:\dati\inventato.txt"));
        assert!(text.contains("non esiste"));
        assert!(text.contains("list_dir"));
        assert!(text.contains("find_files"));
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn windows_verbatim_prefix_is_hidden() {
        assert_eq!(display_path(Path::new(r"\\?\C:\Users\a")), r"C:\Users\a");
        assert_eq!(display_path(Path::new("/home/a")), "/home/a");
    }

    #[test]
    fn tool_use_picks_the_main_argument() {
        let call = ToolCall {
            id: "t1".to_string(),
            name: "search_in_files".to_string(),
            input: serde_json::json!({ "text": "fattura" }),
        };
        assert_eq!(
            ToolUse::of(&call),
            ToolUse { name: "search_in_files".into(), detail: "fattura".into() }
        );
    }
}
