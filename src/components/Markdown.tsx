/**
 * Markdown minimo, sufficiente per quello che scrive un modello: titoli,
 * elenchi, citazioni, tabelle, codice e formattazione inline.
 *
 * Niente libreria esterna: il testo arriva token per token durante lo
 * streaming e viene ri-renderizzato a ogni frame, quindi il parser deve
 * essere piccolo e prevedibile. Non c'e' `dangerouslySetInnerHTML`: tutto
 * finisce in nodi React, cosi' il testo del modello non puo' iniettare HTML.
 */
import { memo, useMemo, useState, type ReactNode } from "react";

type Block =
  | { kind: "code"; lang: string; code: string }
  | { kind: "heading"; level: number; text: string }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "quote"; text: string }
  | { kind: "table"; header: string[]; rows: string[][] }
  | { kind: "rule" }
  | { kind: "para"; text: string };

const FENCE = /^\s*```\s*([\w+-]*)\s*$/;
const HEADING = /^(#{1,6})\s+(.*)$/;
const BULLET = /^\s*[-*+]\s+(.*)$/;
const NUMBERED = /^\s*\d+[.)]\s+(.*)$/;
const QUOTE = /^>\s?(.*)$/;
const RULE = /^\s*(?:---+|\*\*\*+|___+)\s*$/;
const TABLE_ROW = /^\s*\|(.+)\|\s*$/;
const TABLE_SEPARATOR = /^\s*\|?[\s:|-]+\|[\s:|-]*$/;

function splitRow(line: string): string[] {
  return line
    .replace(/^\s*\|/, "")
    .replace(/\|\s*$/, "")
    .split("|")
    .map((cell) => cell.trim());
}

function parseBlocks(source: string): Block[] {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let paragraph: string[] = [];

  function flushParagraph() {
    if (paragraph.length > 0) {
      blocks.push({ kind: "para", text: paragraph.join("\n") });
      paragraph = [];
    }
  }

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];

    const fence = line.match(FENCE);
    if (fence) {
      flushParagraph();
      const code: string[] = [];
      i += 1;
      // Durante lo streaming il blocco puo' non essere ancora chiuso: si
      // mostra comunque quello che e' arrivato.
      while (i < lines.length && !FENCE.test(lines[i])) {
        code.push(lines[i]);
        i += 1;
      }
      blocks.push({ kind: "code", lang: fence[1] ?? "", code: code.join("\n") });
      continue;
    }

    if (line.trim() === "") {
      flushParagraph();
      continue;
    }

    const heading = line.match(HEADING);
    if (heading) {
      flushParagraph();
      blocks.push({ kind: "heading", level: heading[1].length, text: heading[2] });
      continue;
    }

    if (RULE.test(line)) {
      flushParagraph();
      blocks.push({ kind: "rule" });
      continue;
    }

    // Tabella: riga di intestazione seguita dal separatore |---|---|.
    if (TABLE_ROW.test(line) && i + 1 < lines.length && TABLE_SEPARATOR.test(lines[i + 1])) {
      flushParagraph();
      const header = splitRow(line);
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && TABLE_ROW.test(lines[i])) {
        rows.push(splitRow(lines[i]));
        i += 1;
      }
      i -= 1;
      blocks.push({ kind: "table", header, rows });
      continue;
    }

    const bullet = line.match(BULLET);
    const numbered = line.match(NUMBERED);
    if (bullet || numbered) {
      flushParagraph();
      const ordered = numbered !== null;
      const items: string[] = [];
      while (i < lines.length) {
        const current = lines[i];
        const match = ordered ? current.match(NUMBERED) : current.match(BULLET);
        if (match) {
          items.push(match[1]);
        } else if (/^\s+\S/.test(current) && items.length > 0) {
          // Riga di continuazione dell'elemento precedente.
          items[items.length - 1] += `\n${current.trim()}`;
        } else {
          break;
        }
        i += 1;
      }
      i -= 1;
      blocks.push({ kind: "list", ordered, items });
      continue;
    }

    const quote = line.match(QUOTE);
    if (quote) {
      flushParagraph();
      const quoted: string[] = [quote[1]];
      i += 1;
      while (i < lines.length && QUOTE.test(lines[i])) {
        quoted.push(lines[i].match(QUOTE)![1]);
        i += 1;
      }
      i -= 1;
      blocks.push({ kind: "quote", text: quoted.join("\n") });
      continue;
    }

    paragraph.push(line);
  }

  flushParagraph();
  return blocks;
}

/* --------------------------------- inline --------------------------------- */

const INLINE =
  /(`[^`]+`)|(\*\*[^*]+\*\*)|(__[^_]+__)|(\*[^*\n]+\*)|(~~[^~]+~~)|(\[[^\]]+\]\([^)\s]+\))|(https?:\/\/[^\s<>()]+)/g;

/**
 * L'indirizzo di un link, se è di uno schema che si può aprire.
 *
 * React protegge dall'HTML iniettato ma non da un `href`: `javascript:` in un
 * link scritto dal modello sarebbe codice che parte al clic dell'utente. Qui
 * passano solo gli schemi che in una risposta hanno senso; tutto il resto
 * resta testo, visibile e inerte.
 */
function safeHref(url: string): string | null {
  const scheme = url.match(/^\s*([a-zA-Z][a-zA-Z0-9+.-]*):/);
  if (!scheme) {
    // Nessuno schema: è un percorso relativo, non porta da nessuna parte.
    return null;
  }
  return ["http", "https", "mailto"].includes(scheme[1].toLowerCase()) ? url.trim() : null;
}

function inline(text: string, keyPrefix: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  let match: RegExpExecArray | null;
  let n = 0;

  INLINE.lastIndex = 0;
  while ((match = INLINE.exec(text)) !== null) {
    if (match.index > last) out.push(text.slice(last, match.index));
    const token = match[0];
    const key = `${keyPrefix}i${n++}`;

    if (token.startsWith("`")) {
      out.push(
        <code key={key} className="md-code">
          {token.slice(1, -1)}
        </code>,
      );
    } else if (token.startsWith("**") || token.startsWith("__")) {
      out.push(<strong key={key}>{token.slice(2, -2)}</strong>);
    } else if (token.startsWith("~~")) {
      out.push(<del key={key}>{token.slice(2, -2)}</del>);
    } else if (token.startsWith("*")) {
      out.push(<em key={key}>{token.slice(1, -1)}</em>);
    } else if (token.startsWith("[")) {
      const link = token.match(/^\[([^\]]+)\]\(([^)\s]+)\)$/);
      const href = link && safeHref(link[2]);
      out.push(
        link && href ? (
          <a key={key} href={href} target="_blank" rel="noreferrer" title={href}>
            {link[1]}
          </a>
        ) : (
          token
        ),
      );
    } else {
      out.push(
        <a key={key} href={token} target="_blank" rel="noreferrer">
          {token}
        </a>,
      );
    }
    last = match.index + token.length;
  }

  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** Testo con a capo interni: nel markdown restano tali dentro il paragrafo. */
function inlineMultiline(text: string, keyPrefix: string): ReactNode[] {
  return text.split("\n").flatMap((line, i) => {
    const nodes = inline(line, `${keyPrefix}l${i}`);
    return i === 0 ? nodes : [<br key={`${keyPrefix}br${i}`} />, ...nodes];
  });
}

/* ---------------------------------- codice --------------------------------- */

function CodeBlock({ lang, code }: { lang: string; code: string }) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard negata: meglio non fare nulla che mostrare un errore.
    }
  }

  return (
    <div className="md-codeblock">
      <div className="md-codeblock-head">
        <span className="md-lang">{lang || "testo"}</span>
        <button className="md-copy" onClick={() => void copy()}>
          {copied ? "Copiato" : "Copia"}
        </button>
      </div>
      <pre>
        <code>{code}</code>
      </pre>
    </div>
  );
}

/* --------------------------------- render --------------------------------- */

function renderBlock(block: Block, key: string): ReactNode {
  switch (block.kind) {
    case "code":
      return <CodeBlock key={key} lang={block.lang} code={block.code} />;
    case "heading": {
      const Tag = `h${Math.min(block.level + 1, 6)}` as "h2" | "h3" | "h4" | "h5" | "h6";
      return <Tag key={key}>{inline(block.text, key)}</Tag>;
    }
    case "list":
      return block.ordered ? (
        <ol key={key}>
          {block.items.map((item, i) => (
            <li key={i}>{inlineMultiline(item, `${key}-${i}`)}</li>
          ))}
        </ol>
      ) : (
        <ul key={key}>
          {block.items.map((item, i) => (
            <li key={i}>{inlineMultiline(item, `${key}-${i}`)}</li>
          ))}
        </ul>
      );
    case "quote":
      return <blockquote key={key}>{inlineMultiline(block.text, key)}</blockquote>;
    case "table":
      return (
        <div key={key} className="md-table-wrap">
          <table>
            <thead>
              <tr>
                {block.header.map((cell, i) => (
                  <th key={i}>{inline(cell, `${key}h${i}`)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {block.rows.map((row, r) => (
                <tr key={r}>
                  {row.map((cell, c) => (
                    <td key={c}>{inline(cell, `${key}r${r}c${c}`)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case "rule":
      return <hr key={key} />;
    case "para":
      return <p key={key}>{inlineMultiline(block.text, key)}</p>;
  }
}

function Markdown({ text }: { text: string }) {
  const blocks = useMemo(() => parseBlocks(text), [text]);
  return <div className="md">{blocks.map((block, i) => renderBlock(block, `b${i}`))}</div>;
}

export default memo(Markdown);
