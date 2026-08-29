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
        Kind::Pdf | Kind::Docx => MAX_DOC_BYTES,
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
