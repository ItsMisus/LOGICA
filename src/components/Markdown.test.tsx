/**
 * Il parser Markdown è scritto a mano ed è il pezzo di frontend con più
 * probabilità di rompersi in silenzio: riceve testo di un modello, riga per
 * riga, mentre arriva. Qui si rende in HTML statico e si guarda cosa esce —
 * niente jsdom, niente libreria di test dei componenti: `renderToStaticMarkup`
 * basta e non aggiunge tre dipendenze per leggere una stringa.
 */
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import Markdown from "./Markdown";

function render(text: string): string {
  return renderToStaticMarkup(<Markdown text={text} />);
}

describe("Markdown", () => {
  it("rende titoli, elenchi e citazioni", () => {
    const html = render("# Titolo\n\n- primo\n- secondo\n\n> citazione");
    // I titoli scendono di un livello: l'h1 della pagina resta uno solo, e non
    // se lo prende una risposta del modello.
    expect(html).toContain("<h2");
    expect(html).toContain("Titolo");
    expect(html).toContain("<ul");
    expect(html).toContain("primo");
    expect(html).toContain("<blockquote");
  });

  it("distingue un elenco numerato da uno puntato", () => {
    expect(render("1. uno\n2. due")).toContain("<ol");
    expect(render("- uno\n- due")).toContain("<ul");
  });

  it("rende una tabella con la sua intestazione", () => {
    const html = render("| Nome | Ruolo |\n|---|---|\n| Ada | prima |");
    expect(html).toContain("<table");
    expect(html).toContain("<th");
    expect(html).toContain("Ada");
  });

  it("tiene il codice così com'è, senza interpretarlo", () => {
    const html = render("```rust\nlet x = 1 < 2;\n```");
    expect(html).toContain("<pre");
    // Il minore resta un carattere di testo: se fosse finito nell'HTML come
    // tag, qui si vedrebbe `<`, non `&lt;`.
    expect(html).toContain("let x = 1 &lt; 2;");
  });

  it("mostra un blocco di codice ancora aperto, come durante lo streaming", () => {
    const html = render("```python\nprint(1)");
    expect(html).toContain("print(1)");
  });

  it("rende grassetto, corsivo e codice inline", () => {
    const html = render("**forte** e *piano* e `codice`");
    expect(html).toContain("<strong>forte</strong>");
    expect(html).toContain("<em>piano</em>");
    expect(html).toContain(">codice</code>");
  });

  it("rende un link come link, con il rel che protegge la finestra", () => {
    const html = render("[esempio](https://esempio.it)");
    expect(html).toContain('href="https://esempio.it"');
    expect(html).toContain("noreferrer");
  });

  /**
   * La proprietà che conta più di tutte le altre: quello che scrive il modello
   * non deve poter diventare HTML. Se un giorno qualcuno introduce
   * `dangerouslySetInnerHTML` per comodità, questo test cade.
   */
  it("non lascia iniettare HTML al modello", () => {
    const html = render('Guarda: <img src=x onerror="alert(1)"> e <script>alert(2)</script>');
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<script");
    expect(html).toContain("&lt;script&gt;");
  });

  /**
   * React protegge dall'HTML iniettato, non da un `href`: un link
   * `javascript:` scritto dal modello sarebbe codice che parte al clic
   * dell'utente. Deve restare testo.
   */
  it("non lascia passare un link javascript:", () => {
    const html = render("[clicca](javascript:alert(1))");
    expect(html).not.toContain("href=");
    expect(html).toContain("clicca");
  });

  it("un link normale continua a essere un link", () => {
    expect(render("[ok](https://esempio.it)")).toContain('href="https://esempio.it"');
    expect(render("[scrivi](mailto:a@b.it)")).toContain('href="mailto:a@b.it"');
  });

  it("regge il testo vuoto", () => {
    expect(() => render("")).not.toThrow();
  });
});
