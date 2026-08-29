//! Accesso a internet per l'assistente: cerca, scarica, estrae testo.
//!
//! Due vincoli hanno deciso la forma di questo modulo:
//!
//! 1. Deve funzionare uguale con Claude e con un 7B locale. Per questo non si
//!    usa il tool `web_search` nativo dell'API di Claude: la ricerca la fa
//!    l'app e il risultato entra nel prompt come testo, cosi' il percorso e'
//!    lo stesso per tutti i motori.
//! 2. Niente chiavi API da configurare. Si usano le pagine HTML dei motori,
//!    che non chiedono registrazione. Il prezzo e' doppio: il parsing e'
//!    fragile (se cambiano le classi CSS la ricerca torna vuota, non
//!    sbagliata) e un motore puo' rispondere con una pagina di blocco se
//!    riceve troppe richieste di fila. Per questo i motori sono due, provati
//!    in ordine: quando uno blocca, l'altro di solito risponde.
//!
//! Le pagine vengono ridotte a testo: niente tag, niente script, niente CSS.

use std::net::IpAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Endpoint HTML (non JavaScript) di DuckDuckGo.
const DUCKDUCKGO_URL: &str = "https://html.duckduckgo.com/html/";

/// Bing, la riserva: quando DuckDuckGo blocca, questo di solito risponde.
const BING_URL: &str = "https://www.bing.com/search";

/// Senza uno User-Agent da browser molti siti rispondono 403.
const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

const SEARCH_TIMEOUT: Duration = Duration::from_secs(15);
const PAGE_TIMEOUT: Duration = Duration::from_secs(15);

/// Oltre questa dimensione la pagina non viene nemmeno letta: e' quasi sempre
/// un file, non un articolo.
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;

/// Caratteri di testo tenuti per ogni pagina. Due pagine da 4.000 caratteri
/// piu' i riassunti stanno negli 8k di contesto che si chiedono a Ollama;
/// piu' testo farebbe cadere fuori finestra l'inizio della conversazione.
const MAX_PAGE_CHARS: usize = 4_000;

/// Un risultato della lista di ricerca, prima di aprire la pagina.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Una pagina davvero letta, gia' ridotta a testo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub title: String,
    pub url: String,
    pub text: String,
}

/// Quello che l'assistente riceve: le pagine aperte e i risultati della
/// ricerca, che il frontend puo' mostrare come fonti sotto la risposta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Research {
    pub query: String,
    pub pages: Vec<Page>,
    pub hits: Vec<SearchHit>,
}

fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(USER_AGENT)
        // Un redirect verso un indirizzo interno aggirerebbe il controllo su
        // `is_public`: si seguono pochi salti e si ricontrolla ogni volta.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 4 {
                attempt.stop()
            } else if is_public(attempt.url()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|e| format!("Client HTTP non inizializzabile: {e}"))
}

/* --------------------------------- ricerca -------------------------------- */

/// I motori interrogati, in ordine di preferenza.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    DuckDuckGo,
    Bing,
}

impl Engine {
    fn name(self) -> &'static str {
        match self {
            Engine::DuckDuckGo => "DuckDuckGo",
            Engine::Bing => "Bing",
        }
    }
}

/// Cerca sul primo motore che risponde. Una ricerca senza risultati non e' un
/// errore: torna una lista vuota. L'errore arriva solo se nessuno dei due
/// motori e' stato raggiunto.
pub async fn search(query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let mut problems: Vec<String> = Vec::new();
    for engine in [Engine::DuckDuckGo, Engine::Bing] {
        match ask_engine(engine, query, limit).await {
            Ok(hits) if !hits.is_empty() => return Ok(hits),
            Ok(_) => problems.push(format!("{}: nessun risultato", engine.name())),
            Err(e) => problems.push(e),
        }
    }

    // Tutti e due senza risultati: la ricerca e' andata, la risposta e' vuota.
    if problems.iter().all(|p| p.contains("nessun risultato")) {
        return Ok(Vec::new());
    }
    Err(problems.join("; "))
}

async fn ask_engine(engine: Engine, query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
    let client = client(SEARCH_TIMEOUT)?;
    let request = match engine {
        Engine::DuckDuckGo => {
            client.post(DUCKDUCKGO_URL).form(&[("q", query), ("kl", "it-it")])
        }
        Engine::Bing => client.get(BING_URL).query(&[("q", query), ("setlang", "it")]),
    };

    let res = request
        .send()
        .await
        .map_err(|e| format!("{} non raggiungibile: {e}", engine.name()))?;

    let status = res.status();
    // DuckDuckGo risponde 202 con una pagina di blocco quando arrivano troppe
    // ricerche di fila dallo stesso indirizzo: non e' un errore di rete.
    if status == reqwest::StatusCode::ACCEPTED {
        return Err(format!("{} ha bloccato temporaneamente le ricerche da questo computer", engine.name()));
    }
    if !status.is_success() {
        return Err(format!("{} ha risposto {status}", engine.name()));
    }

    let html = res.text().await.map_err(|e| format!("{}: risposta illeggibile: {e}", engine.name()))?;
    Ok(match engine {
        Engine::DuckDuckGo => parse_duckduckgo(&html, limit),
        Engine::Bing => parse_bing(&html, limit),
    })
}

/// Estrae i risultati dall'HTML di DuckDuckGo. Nessun parser HTML: si cercano
/// le due classi che marcano titolo e riassunto, nell'ordine in cui appaiono.
fn parse_duckduckgo(html: &str, limit: usize) -> Vec<SearchHit> {
    let mut out: Vec<SearchHit> = Vec::new();
    let mut rest = html;

    while out.len() < limit {
        let Some(start) = rest.find("class=\"result__a\"") else { break };
        let anchor_start = rest[..start].rfind("<a ").unwrap_or(start);
        let after = &rest[anchor_start..];

        let Some(open_end) = after.find('>') else { break };
        let Some(close) = after[open_end..].find("</a>") else { break };
        let title = html_to_text(&after[open_end + 1..open_end + close]);
        let href = attribute(&after[..open_end], "href");

        // Il riassunto sta sempre dopo il titolo, prima del risultato seguente.
        let tail = &after[open_end + close..];
        let next = tail.find("class=\"result__a\"").unwrap_or(tail.len());
        let snippet = tail[..next]
            .find("class=\"result__snippet\"")
            .and_then(|s| {
                let block = &tail[s..next];
                let open = block.find('>')?;
                let end = block[open..].find("</a>").or_else(|| block[open..].find("</div>"))?;
                Some(html_to_text(&block[open + 1..open + end]))
            })
            .unwrap_or_default();

        push_hit(&mut out, title, href.as_deref().and_then(clean_link), snippet);
        rest = tail;
    }

    out
}

/// Bing: ogni risultato e' un `<li class="b_algo">` con il titolo in `<h2>` e
/// il riassunto in un `<p>`.
fn parse_bing(html: &str, limit: usize) -> Vec<SearchHit> {
    let mut out: Vec<SearchHit> = Vec::new();
    let mut rest = html;

    while out.len() < limit {
        let Some(start) = rest.find("class=\"b_algo\"") else { break };
        let block_start = &rest[start..];
        let next = block_start[1..].find("class=\"b_algo\"").map(|p| p + 1).unwrap_or(block_start.len());
        let block = &block_start[..next];

        if let Some((title, href)) = bing_title(block) {
            let snippet = bing_snippet(block);
            push_hit(&mut out, title, clean_link(&href), snippet);
        }
        rest = &block_start[next..];
    }

    out
}

fn bing_title(block: &str) -> Option<(String, String)> {
    let h2 = block.find("<h2")?;
    let anchor = block[h2..].find("<a ")? + h2;
    let open_end = block[anchor..].find('>')? + anchor;
    let close = block[open_end..].find("</a>")? + open_end;
    let title = html_to_text(&block[open_end + 1..close]);
    let href = attribute(&block[anchor..open_end], "href")?;
    (!title.is_empty()).then_some((title, href))
}

fn bing_snippet(block: &str) -> String {
    let Some(start) = block.find("<p ").or_else(|| block.find("<p>")) else {
        return String::new();
    };
    let after = &block[start..];
    let Some(open_end) = after.find('>') else { return String::new() };
    let end = after[open_end..].find("</p>").map(|p| p + open_end).unwrap_or(after.len());
    html_to_text(&after[open_end + 1..end])
}

fn push_hit(out: &mut Vec<SearchHit>, title: String, url: Option<String>, snippet: String) {
    let Some(url) = url else { return };
    if title.is_empty() || out.iter().any(|h| h.url == url) {
        return;
    }
    out.push(SearchHit { title, url, snippet });
}

/// Valore di un attributo dentro un tag di apertura: `href="..."`.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

/// I due motori nascondono l'indirizzo vero dietro un proprio redirect:
/// DuckDuckGo in chiaro (`/l/?uddg=...`), Bing in base64 (`/ck/a?...&u=a1...`).
fn clean_link(href: &str) -> Option<String> {
    let href = decode_entities(href);

    let direct = if href.contains("/l/?") || href.contains("uddg=") {
        percent_decode(&query_param(&href, "uddg")?)
    } else if href.contains("bing.com/ck/") {
        // Il valore inizia con "a1", poi base64url dell'indirizzo.
        let encoded = percent_decode(&query_param(&href, "u")?);
        let payload = encoded.strip_prefix("a1").unwrap_or(&encoded);
        String::from_utf8(base64url_decode(payload)?).ok()?
    } else {
        href
    };

    let direct = match direct.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => direct,
    };
    if direct.starts_with("http://") || direct.starts_with("https://") {
        Some(direct)
    } else {
        None
    }
}

/// Base64url senza dipendenze: bastano trenta righe e nessun crate in piu'.
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for ch in input.bytes() {
        if ch == b'=' {
            continue;
        }
        // I due alfabeti differiscono solo negli ultimi due caratteri.
        let ch = match ch {
            b'+' => b'-',
            b'/' => b'_',
            other => other,
        };
        let value = TABLE.iter().position(|c| *c == ch)? as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

fn query_param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or(url);
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

/* --------------------------------- pagine --------------------------------- */

/// Scarica una pagina e ne torna il testo. Solo http/https, solo host
/// pubblici, solo contenuto testuale.
pub async fn fetch_page(url: &str) -> Result<Page, String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| format!("Indirizzo non valido: {url}"))?;
    if !is_public(&parsed) {
        return Err("Indirizzo non consentito: e' un host locale o di rete interna.".to_string());
    }

    let res = client(PAGE_TIMEOUT)?
        .get(parsed.clone())
        .send()
        .await
        .map_err(|e| format!("{url} non raggiungibile: {e}"))?;

    if !res.status().is_success() {
        return Err(format!("{url} ha risposto {}", res.status()));
    }

    let content_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    if !content_type.is_empty()
        && !content_type.contains("text/html")
        && !content_type.contains("text/plain")
        && !content_type.contains("xml")
        && !content_type.contains("json")
    {
        return Err(format!("{url} non e' una pagina di testo ({content_type})"));
    }

    // Dopo i redirect l'indirizzo buono e' quello finale: e' quello che va
    // mostrato come fonte e ricontrollato.
    let landed = res.url().clone();
    if !is_public(&landed) {
        return Err("Indirizzo non consentito: e' un host locale o di rete interna.".to_string());
    }

    let bytes = res.bytes().await.map_err(|e| format!("{url} illeggibile: {e}"))?;
    if bytes.len() > MAX_PAGE_BYTES {
        return Err(format!("{url} e' troppo grande da leggere"));
    }
    let html = String::from_utf8_lossy(&bytes);

    let title = page_title(&html).unwrap_or_else(|| landed.host_str().unwrap_or(url).to_string());
    let text = truncate(&html_to_text(&strip_blocks(&html)), MAX_PAGE_CHARS);
    if text.trim().is_empty() {
        return Err(format!("{url} non contiene testo leggibile"));
    }

    Ok(Page { title, url: landed.to_string(), text })
}

/// Blocca localhost, IP privati e host senza punti (nomi di rete locale):
/// il modello non deve poter far bussare l'app a servizi interni, compreso
/// il server di LOGICA stesso.
fn is_public(url: &reqwest::Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else { return false };
    let host = host.trim_matches(|c| c == '[' || c == ']').to_lowercase();

    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                let o = v4.octets();
                // 100.64.0.0/10 e' lo spazio dei provider (CGNAT): non e' internet pubblico.
                let cgnat = o[0] == 100 && (64..128).contains(&o[1]);
                !(v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_broadcast()
                    || v4.is_documentation()
                    || v4.is_unspecified()
                    || cgnat)
            }
            IpAddr::V6(v6) => {
                !(v6.is_loopback() || v6.is_unspecified() || v6.segments()[0] & 0xfe00 == 0xfc00)
            }
        };
    }

    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || !host.contains('.')
    {
        return false;
    }
    true
}

fn page_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title")?;
    let open = html[start..].find('>')? + start;
    let end = lower[open..].find("</title>")? + open;
    let title = html_to_text(&html[open + 1..end]);
    (!title.is_empty()).then_some(title)
}

/* --------------------------------- testo ---------------------------------- */

/// Toglie i blocchi il cui contenuto non e' testo da leggere.
fn strip_blocks(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let tags = ["script", "style", "noscript", "svg", "head"];

    loop {
        let lower = rest.to_lowercase();
        let Some((pos, tag)) =
            tags.iter().filter_map(|t| find_tag(&lower, t, 0).map(|p| (p, *t))).min()
        else {
            break;
        };
        out.push_str(&rest[..pos]);
        match find_closing(&lower, tag, pos) {
            Some(end) => rest = &rest[end..],
            // Tag mai chiuso: il resto del documento non e' recuperabile.
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Posizione del tag di apertura `<nome`, a partire da `from`. Il controllo
/// sul carattere che segue il nome serve a non far combaciare `<head` con
/// `<header`, errore che troncherebbe meta' pagina.
fn find_tag(lower: &str, tag: &str, from: usize) -> Option<usize> {
    let needle = format!("<{tag}");
    let mut at = from;
    while let Some(found) = lower[at..].find(&needle) {
        let start = at + found;
        let after = lower[start + needle.len()..].chars().next();
        match after {
            None => return None,
            Some(c) if c.is_whitespace() || c == '>' || c == '/' => return Some(start),
            _ => at = start + needle.len(),
        }
    }
    None
}

/// Fine del tag di chiusura corrispondente (indice subito dopo `>`).
fn find_closing(lower: &str, tag: &str, from: usize) -> Option<usize> {
    let closing = format!("</{tag}>");
    lower[from..].find(&closing).map(|p| from + p + closing.len())
}

/// HTML a testo: via i tag, i blocchi diventano a capo, gli spazi si
/// compattano. Non e' un parser, e non deve esserlo: serve solo un testo
/// leggibile da dare al modello.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut chars = html.chars().peekable();
    let mut tag = String::new();

    while let Some(c) = chars.next() {
        if c != '<' {
            out.push(c);
            continue;
        }
        tag.clear();
        for t in chars.by_ref() {
            if t == '>' {
                break;
            }
            tag.push(t);
        }
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        if matches!(
            name.as_str(),
            "p" | "br"
                | "div"
                | "li"
                | "tr"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "section"
                | "article"
                | "header"
                | "footer"
                | "table"
                | "blockquote"
                | "pre"
        ) {
            out.push('\n');
        }
    }

    collapse(&decode_entities(&out))
}

/// Spazi ripetuti a uno, righe vuote multiple a una sola.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0usize;
    for line in text.lines() {
        let compact = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if compact.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
            out.push('\n');
        } else {
            blank = 0;
            out.push_str(&compact);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

/// Solo le entita' che compaiono davvero nelle pagine, piu' quelle numeriche.
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        // Il ';' si cerca nei primi caratteri, non nei primi byte: tagliare a
        // meta' una lettera accentata farebbe esplodere lo slice.
        let end = after.char_indices().take(12).find(|(_, c)| *c == ';').map(|(i, _)| i);
        let Some(end) = end else {
            out.push('&');
            rest = &after[1..];
            continue;
        };
        let entity = &after[1..end];
        let decoded = match entity {
            "amp" => Some("&".to_string()),
            "lt" => Some("<".to_string()),
            "gt" => Some(">".to_string()),
            "quot" => Some("\"".to_string()),
            "apos" | "#39" => Some("'".to_string()),
            "nbsp" | "#160" => Some(" ".to_string()),
            _ => entity
                .strip_prefix('#')
                .and_then(|n| match n.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => n.parse::<u32>().ok(),
                })
                .and_then(char::from_u32)
                .map(|c| c.to_string()),
        };
        match decoded {
            Some(text) => {
                out.push_str(&text);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() && text.is_char_boundary(i + 3) => {
                match u8::from_str_radix(&text[i + 1..i + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect::<String>() + "\n[...]"
}

/* ------------------------------- orchestrazione --------------------------- */

/// Cerca e apre le prime `pages` pagine. Le pagine che non si aprono vengono
/// saltate: restano comunque titolo e riassunto nei risultati.
pub async fn research(query: &str, pages: usize) -> Result<Research, String> {
    let hits = search(query, 6).await?;
    let mut opened: Vec<Page> = Vec::new();
    for hit in hits.iter() {
        if opened.len() >= pages {
            break;
        }
        if let Ok(page) = fetch_page(&hit.url).await {
            opened.push(page);
        }
    }
    Ok(Research { query: query.to_string(), pages: opened, hits })
}

/// Il blocco di testo che finisce nel prompt.
pub fn as_prompt_block(research: &Research) -> String {
    let mut block = format!(
        "RISULTATI DAL WEB (ricerca \"{}\", eseguita adesso dall'app)\n",
        research.query
    );
    for (i, hit) in research.hits.iter().enumerate() {
        block.push_str(&format!("[{}] {} - {}\n{}\n", i + 1, hit.title, hit.url, hit.snippet));
    }
    for page in &research.pages {
        block.push_str(&format!(
            "\n--- Contenuto di {} ({}) ---\n{}\n",
            page.title, page.url, page.text
        ));
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_are_read_from_duckduckgo_html() {
        let html = r#"
        <div class="result results_links">
          <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fesempio.it%2Fmeteo&amp;rut=abc">Meteo <b>Roma</b></a>
          <a class="result__snippet" href="x">Previsioni per oggi &amp; domani</a>
        </div>
        <div class="result results_links">
          <a rel="nofollow" class="result__a" href="https://altro.it/pagina">Altro sito</a>
          <a class="result__snippet" href="y">Secondo risultato</a>
        </div>"#;

        let hits = parse_duckduckgo(html, 5);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://esempio.it/meteo");
        assert_eq!(hits[0].title, "Meteo Roma");
        assert_eq!(hits[0].snippet, "Previsioni per oggi & domani");
        assert_eq!(hits[1].url, "https://altro.it/pagina");

        // HTML che non contiene risultati non deve inventarne.
        assert!(parse_duckduckgo("<html><body>niente</body></html>", 5).is_empty());
    }

    #[test]
    fn limit_is_respected() {
        let many: String = (0..5)
            .map(|i| format!(r#"<a class="result__a" href="https://a.it/{i}">N{i}</a>"#))
            .collect();
        assert_eq!(parse_duckduckgo(&many, 3).len(), 3);

        // Lo stesso link ripetuto conta una volta sola.
        let one = r#"<a class="result__a" href="https://a.it/1">Uno</a>"#.repeat(5);
        assert_eq!(parse_duckduckgo(&one, 3).len(), 1);
    }

    #[test]
    fn html_becomes_readable_text() {
        let html = "<html><head><title>T</title><style>p{color:red}</style></head><body><h1>Titolo</h1><p>Prima riga</p><script>var x = 1 < 2;</script><p>Seconda &amp; ultima</p></body></html>";
        let text = html_to_text(&strip_blocks(html));
        assert!(text.contains("Titolo"));
        assert!(text.contains("Prima riga"));
        assert!(text.contains("Seconda & ultima"));
        assert!(!text.contains("color:red"), "CSS rimasto: {text}");
        assert!(!text.contains("var x"), "script rimasto: {text}");
        assert_eq!(page_title(html).as_deref(), Some("T"));
    }

    #[test]
    fn results_are_read_from_bing_html() {
        // Un link di Bing: indirizzo vero in base64url dietro /ck/a.
        let encoded = "aHR0cHM6Ly93d3cuZXNlbXBpby5pdC9zcG9ydA";
        let html = format!(
            r#"<li class="b_algo"><h2><a href="https://www.bing.com/ck/a?!&amp;&amp;p=1&amp;u=a1{encoded}&amp;ntb=1">Titolo <strong>vero</strong></a></h2><div class="b_caption"><p class="b_lineclamp2">Riassunto del risultato</p></div></li>
            <li class="b_algo"><h2><a href="https://diretto.it/pagina">Secondo</a></h2><p>Altro riassunto</p></li>"#
        );

        let hits = parse_bing(&html, 5);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://www.esempio.it/sport");
        assert_eq!(hits[0].title, "Titolo vero");
        assert_eq!(hits[0].snippet, "Riassunto del risultato");
        assert_eq!(hits[1].url, "https://diretto.it/pagina");

        assert!(parse_bing("<html>nessun risultato</html>", 5).is_empty());
    }

    #[test]
    fn base64url_decodes_bing_links() {
        assert_eq!(base64url_decode("TE9HSUNB").expect("decode"), b"LOGICA");
        assert_eq!(base64url_decode("aHR0cHM6Ly9hLml0Lz8xPTImMz00").expect("decode"), b"https://a.it/?1=2&3=4");
        assert!(base64url_decode("non base64 !").is_none());
    }

    #[test]
    fn header_is_not_mistaken_for_head() {
        let html = "<html><head><title>T</title></head><body><header>Menu</header><p>Il testo che conta</p></body></html>";
        let text = html_to_text(&strip_blocks(html));
        assert!(text.contains("Il testo che conta"), "pagina troncata su <header>: {text}");
        assert!(!text.contains('T') || !text.starts_with('T'));
    }

    #[test]
    fn entities_and_percent_are_decoded() {
        assert_eq!(decode_entities("a &amp; b &#233; c &nbsp;d &x"), "a & b é c  d &x");
        assert_eq!(percent_decode("https%3A%2F%2Fx.it%2Fa+b"), "https://x.it/a b");
        // Una & seguita da testo accentato: niente slice a meta' carattere.
        assert_eq!(decode_entities("Tizio & Caio perché così"), "Tizio & Caio perché così");
        assert_eq!(decode_entities("&più; &"), "&più; &");
        assert_eq!(percent_decode("città%"), "città%");
    }

    #[test]
    fn internal_addresses_are_refused() {
        for url in ["https://esempio.it", "http://93.184.216.34/x", "https://sub.dominio.co.uk"] {
            assert!(is_public(&reqwest::Url::parse(url).unwrap()), "{url} dovrebbe passare");
        }
        let internal = [
            "http://localhost:11434/api/chat",
            "http://127.0.0.1/",
            "https://192.168.1.10/",
            "http://10.0.0.5:8443/",
            "http://169.254.169.254/latest/meta-data/",
            "https://[::1]/",
            "http://nas/",
            "https://stampante.local/",
            "file:///C:/Windows/win.ini",
        ];
        for url in internal {
            assert!(!is_public(&reqwest::Url::parse(url).unwrap()), "{url} doveva essere bloccato");
        }
    }

    /// Ricerca piu' lettura delle pagine, contro internet vero:
    /// `cargo test -- --ignored live_research`
    #[test]
    #[ignore]
    fn live_research_opens_pages() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime tokio");
        let found = runtime.block_on(research("capitale del portogallo wikipedia", 2)).expect("ricerca");
        assert!(!found.pages.is_empty(), "nessuna pagina aperta");
        assert!(found.pages[0].text.chars().count() > 200, "testo troppo corto");
        assert!(as_prompt_block(&found).contains("http"));
    }

    /// Ricerca vera: `cargo test -- --ignored live_search`
    #[test]
    #[ignore]
    fn live_search_returns_results() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime tokio");
        let hits =
            runtime.block_on(search("chi ha vinto il giro d'italia 2024", 5)).expect("ricerca");
        assert!(!hits.is_empty(), "nessun risultato: il parsing di DuckDuckGo e' da aggiornare");
        assert!(hits[0].url.starts_with("http"));
    }
}
