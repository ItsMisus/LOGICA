//! Ricerca nei documenti dell'utente per significato, non per parole.
//!
//! Gli strumenti in `tools` trovano quello che il modello sa gia' come si
//! chiama: un nome di file, una parola esatta dentro un testo. Non trovano
//! «quel documento in cui avevo scritto le condizioni di rimborso» se dentro
//! non c'e' la parola "rimborso". Qui i documenti delle cartelle autorizzate
//! vengono tagliati a pezzi e trasformati in vettori una volta sola; a ogni
//! domanda si trasforma anche la domanda e si prendono i pezzi piu' vicini.
//!
//! I vettori li calcola **sempre Ollama**, anche quando si risponde con la
//! Claude API: Anthropic non ha un endpoint di embedding, e mandare fuori ogni
//! pezzo dei documenti per indicizzarli sarebbe il contrario di quello che
//! serve qui.
//!
//! L'indice sta in due file perche' i numeri e il testo hanno bisogni diversi:
//! `index.json` e' leggibile a mano e piccolo, `vectors.bin` e' un blocco di
//! `f32` che in JSON peserebbe tre volte tanto e si rileggerebbe piano.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Modello di embedding. Piccolo (137M), gira su CPU, ed e' fatto per questo
/// mestiere: un modello di chat usato come embedder da vettori peggiori.
pub const EMBED_MODEL: &str = "nomic-embed-text";

/// `nomic-embed-text` e' addestrato con questi prefissi e senza rende meno.
/// Non sono decorazione: dicono al modello se sta leggendo una domanda o un
/// documento, e i due casi finiscono in punti diversi dello spazio.
const QUERY_PREFIX: &str = "search_query: ";
const DOCUMENT_PREFIX: &str = "search_document: ";

/// Caratteri per pezzo. Piu' corto perde il contesto della frase, piu' lungo
/// annacqua il vettore: un pezzo che parla di cinque cose non somiglia bene a
/// nessuna delle cinque.
const CHUNK_CHARS: usize = 900;

/// Sovrapposizione fra un pezzo e il successivo, cosi' una frase a cavallo del
/// taglio resta intera almeno in uno dei due.
const CHUNK_OVERLAP: usize = 200;

/// Pezzi tenuti in tutto. Oltre, l'indicizzazione dura troppo e il file dei
/// vettori diventa ingombrante.
const MAX_CHUNKS: usize = 8_000;

/// File piu' grande di cosi' non si indicizza: e' un archivio o un log.
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// Pezzi mandati a Ollama in una sola richiesta.
const EMBED_BATCH: usize = 16;

/// Estratti messi nel prompt per una domanda.
const TOP_K: usize = 5;

/// Quanto deve somigliare un pezzo alla domanda per finire nel prompt.
///
/// Senza questa soglia, ogni domanda tirerebbe dentro i cinque pezzi meno
/// lontani anche quando non c'entrano niente, e il modello risponderebbe a una
/// domanda di cucina citando un preventivo.
const MIN_SCORE: f32 = 0.45;

/// L'indicizzazione di un documento lungo puo' durare minuti su CPU: il
/// timeout e' per richiesta, non per l'intero lavoro.
const EMBED_TIMEOUT: Duration = Duration::from_secs(120);

/* ---------------------------------- tipi ---------------------------------- */

/// Un file gia' indicizzato, com'era quando lo si e' letto. Se data e
/// dimensione non sono cambiate, si salta.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Stamp {
    /// Secondi dall'epoca.
    pub modified: u64,
    pub size: u64,
}

/// Un pezzo di documento. Il vettore non e' qui: sta in `vectors.bin`, alla
/// riga con lo stesso indice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Piece {
    pub path: String,
    /// Quale pezzo del file e', da 0.
    pub ordinal: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    /// Con quale modello sono stati calcolati i vettori. Cambiando modello i
    /// vettori vecchi non sono confrontabili con i nuovi: l'indice si rifa'.
    #[serde(default)]
    pub model: String,
    /// Lunghezza di un vettore, per rileggere `vectors.bin`.
    #[serde(default)]
    pub dim: usize,
    #[serde(default)]
    pub files: HashMap<String, Stamp>,
    #[serde(default)]
    pub pieces: Vec<Piece>,
    /// Quando e' finita l'ultima indicizzazione, in RFC 3339.
    #[serde(default)]
    pub built: String,
}

impl Default for Index {
    fn default() -> Self {
        Self {
            model: EMBED_MODEL.to_string(),
            dim: 0,
            files: HashMap::new(),
            pieces: Vec::new(),
            built: String::new(),
        }
    }
}

/// Cosa mostrare nelle impostazioni senza caricare i vettori.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStatus {
    pub files: usize,
    pub pieces: usize,
    pub built: String,
    pub model: String,
}

/// Un estratto pertinente alla domanda.
#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub path: String,
    pub text: String,
    pub score: f32,
}

/* --------------------------------- su disco -------------------------------- */

fn dir() -> Result<PathBuf, String> {
    Ok(crate::storage::logica_dir()?.join("index"))
}

fn index_path() -> Result<PathBuf, String> {
    Ok(dir()?.join("index.json"))
}

fn vectors_path() -> Result<PathBuf, String> {
    Ok(dir()?.join("vectors.bin"))
}

pub fn load() -> Index {
    let Ok(file) = index_path() else { return Index::default() };
    let Ok(data) = std::fs::read_to_string(&file) else { return Index::default() };
    serde_json::from_str(&data).unwrap_or_default()
}

pub fn status() -> IndexStatus {
    let index = load();
    IndexStatus {
        files: index.files.len(),
        pieces: index.pieces.len(),
        built: index.built,
        model: index.model,
    }
}

/// I vettori, uno per pezzo, nello stesso ordine di `index.pieces`.
fn load_vectors(dim: usize, count: usize) -> Result<Vec<Vec<f32>>, String> {
    if dim == 0 || count == 0 {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(vectors_path()?).map_err(|e| format!("Vettori illeggibili: {e}"))?;
    let expected = dim * count * 4;
    if bytes.len() != expected {
        return Err(format!(
            "Indice incoerente: {} byte di vettori invece di {expected}. Rifallo dalle impostazioni.",
            bytes.len()
        ));
    }
    Ok(bytes
        .chunks_exact(dim * 4)
        .map(|row| {
            row.chunks_exact(4)
                .map(|n| f32::from_le_bytes([n[0], n[1], n[2], n[3]]))
                .collect()
        })
        .collect())
}

fn save(index: &Index, vectors: &[Vec<f32>]) -> Result<(), String> {
    let folder = dir()?;
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("Impossibile creare {}: {e}", folder.display()))?;

    let mut bytes: Vec<u8> = Vec::with_capacity(vectors.len() * index.dim * 4);
    for vector in vectors {
        for value in vector {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    // I vettori prima: se l'app muore in mezzo, un `vectors.bin` piu' nuovo
    // dell'indice non fa danni, mentre un indice che promette pezzi senza
    // vettori si rifiuterebbe di caricare.
    let vec_tmp = vectors_path()?.with_extension("bin.tmp");
    std::fs::write(&vec_tmp, &bytes).map_err(|e| format!("Scrittura dei vettori fallita: {e}"))?;
    std::fs::rename(&vec_tmp, vectors_path()?)
        .map_err(|e| format!("Salvataggio dei vettori fallito: {e}"))?;

    let data = serde_json::to_string(index)
        .map_err(|e| format!("Serializzazione dell'indice fallita: {e}"))?;
    let tmp = index_path()?.with_extension("json.tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("Scrittura dell'indice fallita: {e}"))?;
    std::fs::rename(&tmp, index_path()?)
        .map_err(|e| format!("Salvataggio dell'indice fallito: {e}"))
}

pub fn clear() -> Result<(), String> {
    let _ = std::fs::remove_file(vectors_path()?);
    let _ = std::fs::remove_file(index_path()?);
    Ok(())
}

/* -------------------------------- embedding ------------------------------- */

/// Vettori per un gruppo di testi, da Ollama.
///
/// `/api/embed` accetta piu' testi in una richiesta ed e' l'endpoint attuale;
/// le versioni precedenti hanno solo `/api/embeddings`, uno per volta. Si
/// prova il primo e si ripiega sul secondo, perche' l'alternativa e' dire
/// all'utente di aggiornare Ollama per una funzione che senza aggiornare
/// funzionerebbe lo stesso, solo piu' piano.
async fn embed(url: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    let client = reqwest::Client::builder()
        .timeout(EMBED_TIMEOUT)
        .build()
        .map_err(|e| format!("Client HTTP non inizializzabile: {e}"))?;
    let base = url.trim_end_matches('/');

    let res = client
        .post(format!("{base}/api/embed"))
        .json(&serde_json::json!({ "model": EMBED_MODEL, "input": texts }))
        .send()
        .await
        .map_err(|e| format!("Ollama non raggiungibile su {url}: {e}"))?;

    if res.status().is_success() {
        let value: serde_json::Value =
            res.json().await.map_err(|e| format!("Risposta di Ollama illeggibile: {e}"))?;
        let rows = value["embeddings"]
            .as_array()
            .ok_or_else(|| "Ollama non ha restituito vettori.".to_string())?;
        return rows.iter().map(as_vector).collect();
    }

    if res.status() == reqwest::StatusCode::NOT_FOUND {
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            out.push(embed_one(&client, base, text).await?);
        }
        return Ok(out);
    }

    let status = res.status();
    let detail = res.text().await.unwrap_or_default();
    Err(missing_embed_model_hint(status, &detail))
}

async fn embed_one(
    client: &reqwest::Client,
    base: &str,
    text: &str,
) -> Result<Vec<f32>, String> {
    let res = client
        .post(format!("{base}/api/embeddings"))
        .json(&serde_json::json!({ "model": EMBED_MODEL, "prompt": text }))
        .send()
        .await
        .map_err(|e| format!("Ollama non raggiungibile: {e}"))?;
    if !res.status().is_success() {
        let status = res.status();
        let detail = res.text().await.unwrap_or_default();
        return Err(missing_embed_model_hint(status, &detail));
    }
    let value: serde_json::Value =
        res.json().await.map_err(|e| format!("Risposta di Ollama illeggibile: {e}"))?;
    as_vector(&value["embedding"])
}

/// Il caso di gran lunga piu' comune: il modello di embedding non e' stato
/// scaricato. "404" da solo non dice cosa fare.
fn missing_embed_model_hint(status: reqwest::StatusCode, detail: &str) -> String {
    if status == reqwest::StatusCode::NOT_FOUND || detail.contains("not found") {
        format!("Il modello di embedding \"{EMBED_MODEL}\" non e' installato in Ollama. Scaricalo con `ollama pull {EMBED_MODEL}`.")
    } else {
        format!("Ollama ha risposto {status} agli embedding: {}", detail.trim())
    }
}

/// I vettori si normalizzano subito: cosi' la somiglianza coseno e' un
/// prodotto scalare e basta, e non si rifanno due radici quadrate per ogni
/// pezzo a ogni domanda.
fn as_vector(value: &serde_json::Value) -> Result<Vec<f32>, String> {
    let raw = value.as_array().ok_or_else(|| "Vettore assente nella risposta.".to_string())?;
    let mut vector: Vec<f32> = raw.iter().filter_map(|n| n.as_f64().map(|v| v as f32)).collect();
    if vector.is_empty() {
        return Err("Vettore vuoto nella risposta.".to_string());
    }
    normalize(&mut vector);
    Ok(vector)
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector.iter_mut() {
            *value /= norm;
        }
    }
}

/// Su vettori normalizzati il coseno e' il prodotto scalare.
fn similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/* ------------------------------ indicizzazione ----------------------------- */

/// Taglia un testo in pezzi sovrapposti.
fn split(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= CHUNK_CHARS {
        let single = text.trim();
        return if single.is_empty() { Vec::new() } else { vec![single.to_string()] };
    }
    let step = CHUNK_CHARS - CHUNK_OVERLAP;
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + CHUNK_CHARS).min(chars.len());
        let piece: String = chars[start..end].iter().collect();
        let piece = piece.trim();
        if !piece.is_empty() {
            out.push(piece.to_string());
        }
        if end == chars.len() {
            break;
        }
        start += step;
    }
    out
}

/// I file da indicizzare, dentro le cartelle autorizzate.
fn indexable_files() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut visited = 0usize;
    for root in crate::server::guard::roots() {
        crate::tools::walk_files(&root, &mut visited, &mut |path: &Path| {
            if is_document(path) {
                found.push(path.to_path_buf());
            }
        });
    }
    found
}

/// Solo quello che ha davvero un testo dentro. Le immagini le legge un modello
/// che vede, non un embedder.
fn is_document(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else { return false };
    if meta.len() == 0 || meta.len() > MAX_FILE_BYTES {
        return false;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg"
        | "mp3" | "mp4" | "mov" | "avi" | "mkv" | "wav" | "zip" | "rar" | "7z" | "gz"
        | "exe" | "dll" | "bin" | "iso" | "db" | "sqlite")
}

fn stamp_of(path: &Path) -> Option<Stamp> {
    let meta = path.metadata().ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(Stamp { modified, size: meta.len() })
}

/// Rifa' l'indice dei documenti nelle cartelle autorizzate.
///
/// Incrementale: un file con data e dimensione invariate non viene riletto ne'
/// rimandato a Ollama. `on_progress` racconta a che punto e', perche' la prima
/// indicizzazione su CPU dura minuti.
pub async fn build(
    ollama_url: &str,
    on_progress: &mut impl FnMut(usize, usize, &str),
) -> Result<IndexStatus, String> {
    if crate::server::guard::roots().is_empty() {
        return Err("Nessuna cartella autorizzata: aggiungine una da Impostazioni prima di indicizzare.".to_string());
    }

    let previous = load();
    // Modello cambiato: i vettori vecchi non sono confrontabili con i nuovi.
    let reusable = previous.model == EMBED_MODEL;
    let old_vectors = if reusable {
        load_vectors(previous.dim, previous.pieces.len()).unwrap_or_default()
    } else {
        Vec::new()
    };
    let keep_old = reusable && old_vectors.len() == previous.pieces.len();

    let files = indexable_files();
    let total = files.len();

    let mut index = Index { dim: if keep_old { previous.dim } else { 0 }, ..Index::default() };
    let mut vectors: Vec<Vec<f32>> = Vec::new();

    for (done, path) in files.iter().enumerate() {
        if index.pieces.len() >= MAX_CHUNKS {
            break;
        }
        let key = path.to_string_lossy().to_string();
        let Some(stamp) = stamp_of(path) else { continue };
        on_progress(done, total, &key);

        // Invariato dall'ultima volta: si riprendono i pezzi e i vettori gia'
        // calcolati invece di rifare il giro da Ollama.
        if keep_old && previous.files.get(&key) == Some(&stamp) {
            for (i, piece) in previous.pieces.iter().enumerate() {
                if piece.path == key {
                    index.pieces.push(piece.clone());
                    vectors.push(old_vectors[i].clone());
                }
            }
            index.files.insert(key, stamp);
            continue;
        }

        let read = crate::attachments::read_all(std::slice::from_ref(&key));
        let Some(text) = read.into_iter().next().and_then(|a| a.text) else { continue };
        let parts = split(&text);
        if parts.is_empty() {
            index.files.insert(key, stamp);
            continue;
        }

        for batch in parts.chunks(EMBED_BATCH) {
            let prefixed: Vec<String> =
                batch.iter().map(|p| format!("{DOCUMENT_PREFIX}{p}")).collect();
            let computed = embed(ollama_url, &prefixed).await?;
            for (piece, vector) in batch.iter().zip(computed) {
                if index.dim == 0 {
                    index.dim = vector.len();
                }
                if vector.len() != index.dim {
                    return Err("Ollama ha restituito vettori di lunghezze diverse.".to_string());
                }
                index.pieces.push(Piece {
                    path: key.clone(),
                    ordinal: index.pieces.len(),
                    text: piece.clone(),
                });
                vectors.push(vector);
            }
        }
        index.files.insert(key, stamp);
    }

    index.built = chrono::Utc::now().to_rfc3339();
    on_progress(total, total, "");
    save(&index, &vectors)?;
    Ok(IndexStatus {
        files: index.files.len(),
        pieces: index.pieces.len(),
        built: index.built,
        model: index.model,
    })
}

/* --------------------------------- ricerca -------------------------------- */

/// Gli estratti piu' vicini alla domanda. Vuoto quando l'indice non c'e' o
/// quando niente somiglia abbastanza: in quel caso non si mette niente nel
/// prompt, invece di infilarci i cinque pezzi meno lontani.
pub async fn search(question: &str, ollama_url: &str) -> Vec<Hit> {
    let index = load();
    if index.pieces.is_empty() || index.dim == 0 {
        return Vec::new();
    }
    let Ok(vectors) = load_vectors(index.dim, index.pieces.len()) else { return Vec::new() };

    let query = format!("{QUERY_PREFIX}{question}");
    let Ok(embedded) = embed(ollama_url, &[query]).await else { return Vec::new() };
    let Some(needle) = embedded.into_iter().next() else { return Vec::new() };
    if needle.len() != index.dim {
        return Vec::new();
    }

    let mut scored: Vec<(f32, usize)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (similarity(&needle, v), i))
        .filter(|(score, _)| *score >= MIN_SCORE)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));

    scored
        .into_iter()
        .take(TOP_K)
        .map(|(score, i)| Hit {
            path: index.pieces[i].path.clone(),
            text: index.pieces[i].text.clone(),
            score,
        })
        .collect()
}

/// Gli estratti come blocco di prompt.
pub fn as_prompt_block(hits: &[Hit]) -> String {
    let body: String = hits
        .iter()
        .map(|h| format!("--- {} ---\n{}\n", h.path, h.text))
        .collect();
    format!(
        "ESTRATTI DAI DOCUMENTI DELL'UTENTE\nPezzi di file che stanno nelle sue cartelle e che somigliano alla domanda. Sono estratti, non file interi: se la risposta qui non c'e', dillo invece di completarla a memoria. Cita il file da cui prendi ogni cosa.\n{body}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_text_stays_one_piece() {
        assert_eq!(split("due parole"), vec!["due parole"]);
        assert!(split("   ").is_empty());
    }

    #[test]
    fn a_long_text_is_cut_with_an_overlap() {
        let text: String = "a".repeat(2_000);
        let parts = split(&text);
        assert!(parts.len() > 1);
        assert_eq!(parts[0].chars().count(), CHUNK_CHARS);
        // I pezzi coprono tutto: l'ultimo arriva in fondo.
        let covered = CHUNK_CHARS + (parts.len() - 1) * (CHUNK_CHARS - CHUNK_OVERLAP);
        assert!(covered >= 2_000);
    }

    #[test]
    fn normalized_vectors_make_the_cosine_a_dot_product() {
        let mut a = vec![3.0, 4.0];
        normalize(&mut a);
        assert!((a[0] - 0.6).abs() < 1e-6);
        assert!((similarity(&a, &a) - 1.0).abs() < 1e-6);

        let mut b = vec![-4.0, 3.0];
        normalize(&mut b);
        assert!(similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn binaries_and_media_are_not_documents() {
        assert!(!is_document(Path::new("foto.jpg")));
        assert!(!is_document(Path::new("app.exe")));
        // Un file che non esiste non ha metadati: non si indicizza.
        assert!(!is_document(Path::new("non_esiste_davvero.txt")));
    }

    #[test]
    fn a_missing_embed_model_says_what_to_type() {
        let hint = missing_embed_model_hint(reqwest::StatusCode::NOT_FOUND, "");
        assert!(hint.contains("ollama pull nomic-embed-text"));
    }

    #[test]
    fn an_index_whose_vectors_do_not_match_is_refused() {
        // dim 0 o nessun pezzo: niente da leggere, niente errore.
        assert!(load_vectors(0, 0).unwrap().is_empty());
    }
}
