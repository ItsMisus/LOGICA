//! File allegati a un messaggio. Il frontend passa i percorsi scelti nel
//! dialogo di sistema; qui si tira fuori quello che il modello puo' leggere.
//!
//! Quattro strade, decise dall'estensione e confermate dai primi byte: testo
//! semplice, PDF, DOCX e immagini. Il testo viene tagliato a `MAX_CHARS`; le
//! immagini vengono passate in base64 e le guarda solo un modello che vede.
//! Tutto il resto torna con un errore in italiano accanto, senza fermare gli
//! altri allegati.

use std::fs;
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// Limite per i file di solo testo: oltre, e' un log o un dump, non un
/// allegato da leggere in chat.
const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;

/// PDF e DOCX pesano per la formattazione, non per le parole: il testo che ne
/// esce resta comunque tagliato a `MAX_CHARS`.
const MAX_DOC_BYTES: u64 = 20 * 1024 * 1024;

/// Immagini: in base64 crescono di un terzo, e le API le rifiutano oltre i
/// 5 MB codificati. Tre MB di file stanno sotto con margine.
const MAX_IMAGE_BYTES: u64 = 3 * 1024 * 1024;

/// Caratteri passati al modello per ogni allegato.
const MAX_CHARS: usize = 12_000;

/// Che cosa e' stato letto: serve all'etichetta in chat e a sapere se questo
/// allegato ha bisogno di un modello che vede.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Pdf,
    Docx,
    Xlsx,
    Image,
}

/// Immagine pronta da spedire al modello.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    /// Tipo MIME come lo vogliono le API: `image/png`, `image/jpeg`, ...
    pub media_type: String,
    /// Contenuto del file in base64 standard, senza prefisso `data:`.
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub path: String,
    pub size: u64,
    #[serde(default = "default_kind")]
    pub kind: Kind,
    /// Contenuto testuale, gia' tagliato. `None` per le immagini e per i file
    /// che non si sono potuti leggere.
    pub text: Option<String>,
    /// Immagine in base64. `None` per tutto il resto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Image>,
    /// Il contenuto e' stato tagliato a `MAX_CHARS`.
    pub truncated: bool,
    /// Perche' il file non e' stato letto, in italiano, pronto da mostrare.
    pub error: Option<String>,
}

/// Le chat salvate dalle versioni precedenti hanno allegati senza `kind`:
/// erano tutti testo, perche' era l'unica cosa che si riusciva a leggere.
fn default_kind() -> Kind {
    Kind::Text
}

impl Attachment {
    fn failed(path: &str, name: String, size: u64, kind: Kind, reason: String) -> Self {
        Self {
            name,
            path: path.to_string(),
            size,
            kind,
            text: None,
            image: None,
            truncated: false,
            error: Some(reason),
        }
    }

    fn with_text(path: &str, name: String, size: u64, kind: Kind, content: String) -> Self {
        let truncated = content.chars().count() > MAX_CHARS;
        let text = if truncated { content.chars().take(MAX_CHARS).collect() } else { content };
        Self {
            name,
            path: path.to_string(),
            size,
            kind,
            text: Some(text),
            image: None,
            truncated,
            error: None,
        }
    }
}

/// Legge gli allegati uno per uno: un file illeggibile non ferma gli altri,
/// torna con il suo errore accanto.
pub fn read_all(paths: &[String]) -> Vec<Attachment> {
    paths.iter().map(|p| read_one(p)).collect()
}

/// Un'immagine che arriva dagli appunti invece che dal disco.
///
/// Non ha un percorso, e non ne inventiamo uno: `path` resta vuoto. Il resto
/// del giro (il messaggio, il salvataggio, l'invio al modello) tratta gli
/// allegati per contenuto, non per percorso, quindi non se ne accorge.
pub fn from_image_data(name: &str, media_type: &str, data: &str) -> Result<Attachment, String> {
    if !matches!(media_type, "image/png" | "image/jpeg" | "image/gif" | "image/webp") {
        return Err(format!("Formato immagine non supportato: {media_type}"));
    }
    // Il base64 non si riscrive: si controlla che sia valido e quanto pesa
    // davvero una volta decodificato, che e' il numero che conta per il limite.
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| "Immagine incollata illeggibile.".to_string())?;
    let size = bytes.len() as u64;
    if size > MAX_IMAGE_BYTES {
        let mb = MAX_IMAGE_BYTES / (1024 * 1024);
        return Err(format!("Immagine troppo grande (oltre {mb} MB)."));
    }

    let name = if name.trim().is_empty() { "immagine incollata" } else { name.trim() };
    Ok(Attachment {
        name: name.to_string(),
        path: String::new(),
        size,
        kind: Kind::Image,
        text: None,
        image: Some(Image { media_type: media_type.to_string(), data: data.to_string() }),
        truncated: false,
        error: None,
    })
}

/// Estensione in minuscolo, vuota se non c'e'.
fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Che tipo di file e', dall'estensione. I byte confermano dopo: un `.pdf`
/// che non comincia per `%PDF` non lo legge nessun estrattore.
fn kind_of(path: &Path) -> Kind {
    match extension(path).as_str() {
        "pdf" => Kind::Pdf,
        "docx" => Kind::Docx,
        "xlsx" => Kind::Xlsx,
        "png" | "jpg" | "jpeg" | "gif" | "webp" => Kind::Image,
        _ => Kind::Text,
    }
}

/// Tipo MIME dell'immagine. Le API accettano solo questi quattro formati.
fn media_type(path: &Path) -> Option<&'static str> {
    match extension(path).as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Il tetto di dimensione dipende da cosa stiamo leggendo.
fn max_bytes(kind: Kind) -> u64 {
    match kind {
        Kind::Text => MAX_TEXT_BYTES,
        Kind::Pdf | Kind::Docx | Kind::Xlsx => MAX_DOC_BYTES,
        Kind::Image => MAX_IMAGE_BYTES,
    }
}

fn read_one(path: &str) -> Attachment {
    let file = Path::new(path);
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    let kind = kind_of(file);

    let size = match fs::metadata(file) {
        Ok(meta) if meta.is_dir() => {
            return Attachment::failed(path, name, 0, kind, "e' una cartella, non un file".to_string())
        }
        Ok(meta) => meta.len(),
        Err(e) => return Attachment::failed(path, name, 0, kind, format!("non accessibile: {e}")),
    };

    let limit = max_bytes(kind);
    if size > limit {
        let mb = limit / (1024 * 1024);
        return Attachment::failed(path, name, size, kind, format!("troppo grande (oltre {mb} MB)"));
    }

    let bytes = match fs::read(file) {
        Ok(b) => b,
        Err(e) => return Attachment::failed(path, name, size, kind, format!("non leggibile: {e}")),
    };

    match kind {
        Kind::Image => match media_type(file) {
            Some(mime) => Attachment {
                name,
                path: path.to_string(),
                size,
                kind,
                text: None,
                image: Some(Image {
                    media_type: mime.to_string(),
                    data: base64::engine::general_purpose::STANDARD.encode(&bytes),
                }),
                truncated: false,
                error: None,
            },
            None => {
                Attachment::failed(path, name, size, kind, "formato immagine non supportato".to_string())
            }
        },
        Kind::Pdf => match pdf_text(&bytes) {
            Ok(text) => Attachment::with_text(path, name, size, kind, text),
            Err(e) => Attachment::failed(path, name, size, kind, e),
        },
        Kind::Docx => match docx_text(&bytes) {
            Ok(text) => Attachment::with_text(path, name, size, kind, text),
            Err(e) => Attachment::failed(path, name, size, kind, e),
        },
        Kind::Xlsx => match xlsx_text(&bytes) {
            Ok(text) => Attachment::with_text(path, name, size, kind, text),
            Err(e) => Attachment::failed(path, name, size, kind, e),
        },
        Kind::Text => match plain_text(bytes) {
            Ok(text) => Attachment::with_text(path, name, size, kind, text),
            Err(e) => Attachment::failed(path, name, size, kind, e),
        },
    }
}

/* ------------------------------ testo semplice ---------------------------- */

/// UTF-8 se si puo', altrimenti Windows-1252: e' la codifica di quasi tutti i
/// file di testo europei scritti su Windows, e comprende Latin-1.
///
/// Un byte zero resta il segnale piu' affidabile che il file non e' testo:
/// nessuna delle due codifiche lo produce in un documento vero.
fn plain_text(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.contains(&0) {
        return Err("file binario: niente testo da leggere".to_string());
    }
    match String::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(e) => {
            let raw = e.into_bytes();
            let (text, _, had_errors) = encoding_rs::WINDOWS_1252.decode(&raw);
            if had_errors {
                Err("codifica non riconosciuta".to_string())
            } else {
                Ok(text.into_owned())
            }
        }
    }
}

/* ----------------------------------- PDF ---------------------------------- */

/// `pdf-extract` lavora su file scritti da decine di produttori diversi e su
/// quelli malfatti va in panico invece di tornare errore: qui il panico si
/// ferma e diventa un messaggio, altrimenti si porta giu' l'app per un
/// allegato.
fn pdf_text(bytes: &[u8]) -> Result<String, String> {
    if !bytes.starts_with(b"%PDF") {
        return Err("non sembra un PDF".to_string());
    }
    let extracted = catch_unwind(AssertUnwindSafe(|| pdf_extract::extract_text_from_mem(bytes)))
        .map_err(|_| "PDF illeggibile: il testo non si e' potuto estrarre".to_string())?
        .map_err(|e| format!("PDF illeggibile: {e}"))?;

    let text = squeeze_blank_lines(&extracted);
    if text.trim().is_empty() {
        // Un PDF fatto di pagine scannerizzate non ha testo dentro: senza OCR
        // non c'e' niente da leggere, e dirlo aiuta piu' di una stringa vuota.
        return Err("il PDF non contiene testo (probabilmente e' una scansione)".to_string());
    }
    Ok(text)
}

/* ---------------------------------- DOCX ---------------------------------- */

/// Un `.docx` e' uno zip: il testo sta in `word/document.xml`, dentro i tag
/// `<w:t>`. Si legge quello e si buttano via i tag, tenendo i paragrafi.
fn docx_text(bytes: &[u8]) -> Result<String, String> {
    if !bytes.starts_with(b"PK") {
        return Err("non sembra un DOCX".to_string());
    }
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| format!("DOCX illeggibile: {e}"))?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|_| "DOCX senza documento dentro: forse e' un .doc rinominato".to_string())?
        .read_to_string(&mut xml)
        .map_err(|e| format!("DOCX illeggibile: {e}"))?;

    let text = squeeze_blank_lines(&strip_word_xml(&xml));
    if text.trim().is_empty() {
        return Err("il documento non contiene testo".to_string());
    }
    Ok(text)
}

/// Toglie i tag di Word tenendo la struttura: fine paragrafo e interruzione
/// di riga diventano un a capo, il tabulatore un tab.
fn strip_word_xml(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len() / 2);
    let mut rest = xml;

    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let end = match after.find('>') {
            Some(i) => i,
            // Tag mai chiuso: da qui in poi non e' piu' XML, si smette.
            None => return unescape_xml(&out),
        };
        let tag = &after[1..end];
        if tag.starts_with("/w:p") || tag.starts_with("w:br") {
            out.push('\n');
        } else if tag.starts_with("w:tab") {
            out.push('\t');
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);

    unescape_xml(&out)
}

/* ---------------------------------- XLSX ---------------------------------- */

/// Righe lette da un foglio. Un modello non ragiona su diecimila righe di
/// tabella, e il testo viene comunque tagliato dopo: meglio fermarsi qui e
/// dirlo, che riempire il contesto con la meta' di un foglio.
const MAX_SHEET_ROWS: usize = 2_000;

/// Celle lette da una riga.
const MAX_ROW_CELLS: usize = 60;

/// Un `.xlsx` e' uno zip come il `.docx`, con due differenze che contano: il
/// testo delle celle non sta nel foglio ma in una tabella condivisa
/// (`sharedStrings.xml`, a cui le celle rimandano per indice), e i fogli sono
/// piu' di uno.
///
/// Esce come TSV: una riga per riga, celle separate da tabulatore. E' il
/// formato in cui un modello riconosce una tabella senza che gliela si debba
/// spiegare, e costa molti meno caratteri di un Markdown allineato.
fn xlsx_text(bytes: &[u8]) -> Result<String, String> {
    if !bytes.starts_with(b"PK") {
        return Err("non sembra un XLSX".to_string());
    }
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| format!("XLSX illeggibile: {e}"))?;

    let shared = read_zip_entry(&mut zip, "xl/sharedStrings.xml")
        .map(|xml| shared_strings(&xml))
        .unwrap_or_default();

    // I fogli si leggono in ordine di nome, che nei file scritti da Excel e'
    // anche l'ordine delle schede.
    let mut sheets: Vec<String> = zip
        .file_names()
        .filter(|n| n.starts_with("xl/worksheets/") && n.ends_with(".xml"))
        .map(|n| n.to_string())
        .collect();
    if sheets.is_empty() {
        return Err("XLSX senza fogli dentro: forse e' un .xls rinominato".to_string());
    }
    sheets.sort();

    let mut out = String::new();
    for sheet in &sheets {
        let Some(xml) = read_zip_entry(&mut zip, sheet) else { continue };
        let rows = sheet_rows(&xml, &shared);
        if rows.trim().is_empty() {
            continue;
        }
        if sheets.len() > 1 {
            let name = sheet.rsplit('/').next().unwrap_or(sheet).trim_end_matches(".xml");
            out.push_str(&format!("--- foglio {name} ---\n"));
        }
        out.push_str(&rows);
        out.push('\n');
    }

    let text = squeeze_blank_lines(&out);
    if text.trim().is_empty() {
        return Err("il foglio di calcolo non contiene testo".to_string());
    }
    Ok(text)
}

fn read_zip_entry<R: std::io::Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> Option<String> {
    let mut xml = String::new();
    zip.by_name(name).ok()?.read_to_string(&mut xml).ok()?;
    Some(xml)
}

/// La tabella condivisa: un elemento `<si>` per stringa, con dentro uno o piu'
/// `<t>` (Excel spezza in piu' pezzi una cella con formattazione mista).
fn shared_strings(xml: &str) -> Vec<String> {
    blocks(xml, "<si", "</si>").map(|inner| texts_in(inner)).collect()
}

/// Le righe di un foglio, come TSV.
fn sheet_rows(xml: &str, shared: &[String]) -> String {
    let mut out = String::new();
    let mut rows = 0usize;

    for row in blocks(xml, "<row", "</row>") {
        if rows >= MAX_SHEET_ROWS {
            out.push_str("[...] foglio tagliato: altre righe non lette\n");
            break;
        }
        let cells: Vec<String> = blocks(row, "<c", "</c>")
            .take(MAX_ROW_CELLS)
            .map(|cell| cell_value(cell, shared))
            .collect();
        // Una riga di sole celle vuote non aggiunge niente.
        if cells.iter().any(|c| !c.is_empty()) {
            out.push_str(&cells.join("\t"));
            out.push('\n');
            rows += 1;
        }
    }
    out
}

/// Il contenuto di una cella. `t="s"` vuol dire che dentro `<v>` non c'e' il
/// testo ma il numero di riga della tabella condivisa; `t="inlineStr"` che il
/// testo sta li' dentro; tutto il resto e' un numero o una data gia' scritta.
fn cell_value(cell: &str, shared: &[String]) -> String {
    let head = cell.split('>').next().unwrap_or("");
    let kind = attribute(head, "t").unwrap_or_default();

    if kind == "inlineStr" {
        return texts_in(cell);
    }
    let raw = blocks(cell, "<v", "</v>").next().map(inner_text).unwrap_or_default();
    if kind == "s" {
        return raw
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|i| shared.get(i))
            .cloned()
            .unwrap_or_default();
    }
    raw
}

/// Il valore di un attributo dentro il tag di apertura.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// I blocchi `<tag ...>...</tag>`, ognuno dall'inizio del tag di apertura alla
/// fine del contenuto. Un elemento vuoto (`<c r="A1"/>`) non ha chiusura: si
/// salta, e infatti non contiene niente.
fn blocks<'a>(xml: &'a str, open: &'a str, close: &'a str) -> impl Iterator<Item = &'a str> {
    let mut rest = xml;
    std::iter::from_fn(move || loop {
        let start = rest.find(open)?;
        let after = &rest[start..];
        let Some(end) = after.find(close) else {
            rest = "";
            return None;
        };
        // `<row` non deve agganciare `<rowBreaks`: dopo il nome del tag ci
        // vuole uno spazio, una chiusura o una barra.
        let next = after[open.len()..].chars().next();
        let is_tag = matches!(next, Some(' ') | Some('>') | Some('/'));
        rest = &after[end + close.len()..];
        if is_tag {
            return Some(&after[..end]);
        }
    })
}

/// Il testo dentro un frammento, tolti tutti i tag.
fn inner_text(fragment: &str) -> String {
    let body = match fragment.find('>') {
        Some(i) => &fragment[i + 1..],
        None => return String::new(),
    };
    unescape_xml(body)
}

/// Il testo di tutti i `<t>` dentro un frammento, attaccati: e' cosi' che una
/// cella con formattazione mista torna una stringa sola.
fn texts_in(fragment: &str) -> String {
    blocks(fragment, "<t", "</t>").map(inner_text).collect::<Vec<_>>().join("")
}

/// Le cinque entita' XML predefinite: sono le uniche che Word scrive nel
/// testo dei paragrafi.
fn unescape_xml(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        // `&amp;` per ultima, o le altre si ri-decodificherebbero a catena.
        .replace("&amp;", "&")
}

/* --------------------------------- comune --------------------------------- */

/// I documenti convertiti escono pieni di righe vuote: due o piu' di fila
/// diventano una sola, cosi' il contesto del modello non si riempie di niente.
fn squeeze_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blanks = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push_str(trimmed);
        out.push('\n');
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("logica_allegati_{tag}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("cartella temporanea");
        dir
    }

    #[test]
    fn reads_text_and_reports_binary() {
        let dir = temp_dir("misti");

        let testo = dir.join("note.txt");
        fs::write(&testo, "ciao mondo").expect("scrittura");

        let binario = dir.join("immagine.bin");
        let mut f = fs::File::create(&binario).expect("creazione");
        f.write_all(&[0x89, 0x50, 0x00, 0x01]).expect("scrittura");

        let letti = read_all(&[
            testo.to_string_lossy().to_string(),
            binario.to_string_lossy().to_string(),
            dir.join("assente.txt").to_string_lossy().to_string(),
        ]);

        assert_eq!(letti[0].name, "note.txt");
        assert_eq!(letti[0].kind, Kind::Text);
        assert_eq!(letti[0].text.as_deref(), Some("ciao mondo"));
        assert!(!letti[0].truncated);
        assert!(letti[0].error.is_none());

        assert!(letti[1].text.is_none());
        assert!(letti[1].error.as_deref().unwrap().contains("binario"));

        assert!(letti[2].error.is_some());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncates_long_files() {
        let dir = temp_dir("lungo");
        let lungo = dir.join("lungo.md");
        fs::write(&lungo, "a".repeat(MAX_CHARS + 500)).expect("scrittura");

        let letti = read_all(&[lungo.to_string_lossy().to_string()]);
        assert!(letti[0].truncated);
        assert_eq!(letti[0].text.as_ref().unwrap().chars().count(), MAX_CHARS);

        let _ = fs::remove_dir_all(&dir);
    }

    /// Un file scritto da Blocco note in Europa occidentale: `perche'` con la
    /// e accentata e' 0xE9, che in UTF-8 non vuol dire niente.
    #[test]
    fn falls_back_to_windows_1252() {
        let dir = temp_dir("codifica");
        let file = dir.join("appunti.txt");
        fs::write(&file, [b'p', b'e', b'r', b'c', b'h', 0xE9]).expect("scrittura");

        let letti = read_all(&[file.to_string_lossy().to_string()]);
        assert_eq!(letti[0].text.as_deref(), Some("perché"));
        assert!(letti[0].error.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn images_come_back_in_base64() {
        let dir = temp_dir("immagini");
        // Un PNG di un pixel: basta che i byte tornino identici.
        let png: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let file = dir.join("pixel.png");
        fs::write(&file, png).expect("scrittura");

        let letti = read_all(&[file.to_string_lossy().to_string()]);
        assert_eq!(letti[0].kind, Kind::Image);
        assert!(letti[0].text.is_none());
        let image = letti[0].image.as_ref().expect("immagine");
        assert_eq!(image.media_type, "image/png");
        assert_eq!(
            base64::engine::general_purpose::STANDARD.decode(&image.data).expect("base64"),
            png
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// Un `.docx` vero e' uno zip con dentro `word/document.xml`: qui se ne
    /// costruisce uno minimo, cosi' il test non porta un binario nel repo.
    #[test]
    fn reads_the_text_of_a_docx() {
        let dir = temp_dir("docx");
        let file = dir.join("lettera.docx");

        let xml = "<w:document><w:body><w:p><w:r><w:t>Primo paragrafo</w:t></w:r></w:p>\
                   <w:p><w:r><w:t>Secondo &amp; ultimo</w:t></w:r></w:p></w:body></w:document>";
        let buffer = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(buffer);
        zip.start_file::<_, ()>("word/document.xml", zip::write::SimpleFileOptions::default())
            .expect("voce zip");
        zip.write_all(xml.as_bytes()).expect("scrittura xml");
        let finished = zip.finish().expect("chiusura zip").into_inner();
        fs::write(&file, finished).expect("scrittura docx");

        let letti = read_all(&[file.to_string_lossy().to_string()]);
        assert_eq!(letti[0].kind, Kind::Docx);
        assert_eq!(letti[0].error, None);
        assert_eq!(letti[0].text.as_deref(), Some("Primo paragrafo\nSecondo & ultimo"));

        let _ = fs::remove_dir_all(&dir);
    }

    /// Un `.xlsx` minimo: le stringhe stanno in una tabella a parte e le celle
    /// ci rimandano per indice, che e' la differenza che conta rispetto al
    /// `.docx`.
    #[test]
    fn reads_the_cells_of_an_xlsx() {
        let dir = temp_dir("xlsx");
        let file = dir.join("conti.xlsx");

        let shared = "<sst><si><t>Voce</t></si><si><t>Importo</t></si>\
                      <si><t>Affitto &amp; spese</t></si></sst>";
        let sheet = "<worksheet><sheetData>\
            <row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\" t=\"s\"><v>1</v></c></row>\
            <row r=\"2\"><c r=\"A2\" t=\"s\"><v>2</v></c><c r=\"B2\"><v>750</v></c></row>\
            <row r=\"3\"/>\
            </sheetData></worksheet>";

        let buffer = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(buffer);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file::<_, ()>("xl/sharedStrings.xml", options).expect("voce zip");
        zip.write_all(shared.as_bytes()).expect("scrittura");
        zip.start_file::<_, ()>("xl/worksheets/sheet1.xml", options).expect("voce zip");
        zip.write_all(sheet.as_bytes()).expect("scrittura");
        let finished = zip.finish().expect("chiusura zip").into_inner();
        fs::write(&file, finished).expect("scrittura xlsx");

        let letti = read_all(&[file.to_string_lossy().to_string()]);
        assert_eq!(letti[0].kind, Kind::Xlsx);
        assert_eq!(letti[0].error, None);
        // TSV: e' il formato in cui un modello riconosce una tabella senza
        // spiegazioni. La riga vuota non compare.
        assert_eq!(letti[0].text.as_deref(), Some("Voce\tImporto\nAffitto & spese\t750"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fake_xlsx_is_refused() {
        let dir = temp_dir("finto_xlsx");
        let file = dir.join("finto.xlsx");
        fs::write(&file, "non sono uno zip").expect("scrittura");

        let letti = read_all(&[file.to_string_lossy().to_string()]);
        assert!(letti[0].error.as_deref().unwrap().contains("non sembra un XLSX"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pasted_image_needs_no_path() {
        // Un PNG di un pixel: quello che conta qui e' che il base64 sia valido.
        let png = base64::engine::general_purpose::STANDARD.encode([0x89, 0x50, 0x4E, 0x47]);
        let attached = from_image_data("", "image/png", &png).expect("immagine accettata");
        assert_eq!(attached.path, "");
        assert_eq!(attached.name, "immagine incollata");
        assert_eq!(attached.kind, Kind::Image);
        assert_eq!(attached.image.expect("immagine").media_type, "image/png");
    }

    #[test]
    fn a_pasted_image_of_the_wrong_kind_is_refused() {
        assert!(from_image_data("x.bmp", "image/bmp", "AAAA").is_err());
        assert!(from_image_data("x.png", "image/png", "non base64!!").is_err());
    }

    /// Estensione giusta, contenuto no: si dice cosa non torna invece di
    /// mandare l'estrattore a sbattere.
    #[test]
    fn a_fake_pdf_is_refused() {
        let dir = temp_dir("finto_pdf");
        let file = dir.join("finto.pdf");
        fs::write(&file, "non sono un PDF").expect("scrittura");

        let letti = read_all(&[file.to_string_lossy().to_string()]);
        assert_eq!(letti[0].kind, Kind::Pdf);
        assert!(letti[0].error.as_deref().unwrap().contains("non sembra un PDF"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn blank_lines_are_squeezed() {
        assert_eq!(squeeze_blank_lines("a\n\n\n\nb\n\n"), "a\n\nb");
    }
}
