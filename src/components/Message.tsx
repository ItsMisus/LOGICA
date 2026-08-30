import { useState } from "react";

import Markdown from "./Markdown";
import type { Attachment, ChatMessage } from "../types";

interface Props {
  message: ChatMessage;
  /** Il messaggio si sta ancora scrivendo: cursore lampeggiante, niente azioni. */
  streaming?: boolean;
  /** Ultima risposta dell'assistente: e' l'unica che ha senso rigenerare. */
  canRegenerate?: boolean;
  onRegenerate?: () => void;
  /** Il messaggio dell'utente si puo' riscrivere: fermo il modello, si'. */
  canEdit?: boolean;
  /** Manda di nuovo questo turno con il testo cambiato. */
  onEdit?: (text: string) => void;
  /** Passa a un'altra risposta fra quelle date alla stessa domanda. */
  onPickVariant?: (index: number) => void;
}

/** Coda del tooltip di un allegato: da dove viene il testo che ha letto il
 *  modello, e se e' stato tagliato. */
function label(file: Attachment): string {
  const kind =
    file.kind === "pdf"
      ? " - testo estratto dal PDF"
      : file.kind === "docx"
        ? " - testo estratto dal documento"
        : file.kind === "xlsx"
          ? " - celle del foglio di calcolo"
          : "";
  return file.truncated ? `${kind} (troncato)` : kind;
}

/** Nel chip della fonte sta il sito, non l'indirizzo intero. */
function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return url;
  }
}

/** Come si chiamano, per chi legge, gli strumenti sui file. */
const TOOL_LABELS: Record<string, string> = {
  list_dir: "Cartella",
  read_file: "Letto",
  find_files: "Cercato",
  search_in_files: "Cercato nei file",
};

/** Nel chip sta il nome del file, non tutto il percorso: quello resta nel
 *  titolo al passaggio del mouse. */
function shortPath(detail: string): string {
  const parts = detail.split(/[\\/]/).filter(Boolean);
  return parts.length > 1 ? parts[parts.length - 1] : detail;
}

export default function Message({
  message,
  streaming,
  canRegenerate,
  onRegenerate,
  canEdit,
  onEdit,
  onPickVariant,
}: Props) {
  const [copied, setCopied] = useState(false);
  /** Testo in riscrittura: `null` quando il messaggio si sta solo leggendo. */
  const [draft, setDraft] = useState<string | null>(null);

  async function copy() {
    try {
      await navigator.clipboard.writeText(message.content);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard negata dal sistema: si lascia perdere in silenzio.
    }
  }

  /** Manda il testo riscritto: da qui in poi la conversazione riparte. */
  function resend() {
    const text = draft?.trim() ?? "";
    if (text.length === 0 || !onEdit) return;
    setDraft(null);
    onEdit(text);
  }

  if (message.role === "user") {
    const files = message.attachments ?? [];
    const images = files.filter((f) => f.image);
    const others = files.filter((f) => !f.image);
    return (
      <div className="turn user">
        <div className="user-bubble">
          {images.length > 0 && (
            <div className="bubble-images">
              {/* Un'immagine incollata non ha percorso: la chiave la da'
                  la posizione, o due incollate si sovrapporrebbero. */}
              {images.map((file, i) => (
                <img
                  key={file.path || `incollata-${i}`}
                  className="bubble-image"
                  src={`data:${file.image?.media_type};base64,${file.image?.data}`}
                  alt={file.name}
                  title={file.path || file.name}
                />
              ))}
            </div>
          )}
          {others.length > 0 && (
            <div className="bubble-files">
              {others.map((file, i) => (
                <span
                  key={file.path || `allegato-${i}`}
                  className={`file-chip ${file.error ? "failed" : ""}`}
                  title={file.error ? `${file.path} - ${file.error}` : `${file.path}${label(file)}`}
                >
                  {file.name}
                </span>
              ))}
            </div>
          )}
          {draft === null ? (
            message.content
          ) : (
            <div className="bubble-edit">
              <textarea
                className="bubble-edit-field"
                value={draft}
                autoFocus
                aria-label="Riscrivi il messaggio"
                rows={Math.min(12, draft.split("\n").length + 1)}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  // Invio manda, Shift+Invio va a capo: come nel composer.
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    resend();
                  } else if (e.key === "Escape") {
                    e.preventDefault();
                    setDraft(null);
                  }
                }}
              />
              <div className="bubble-edit-actions">
                <button className="tool-btn" onClick={() => setDraft(null)}>
                  Annulla
                </button>
                <button className="tool-btn primary" onClick={resend} disabled={draft.trim().length === 0}>
                  Invia di nuovo
                </button>
              </div>
            </div>
          )}
        </div>

        {draft === null && canEdit && onEdit && (
          <div className="turn-actions user-actions">
            <button className="tool-btn" onClick={() => setDraft(message.content)}>
              Modifica
            </button>
          </div>
        )}
      </div>
    );
  }

  // Errore di rete o di configurazione: si distingue da una risposta vera.
  if (message.type === "notification") {
    return (
      <div className="turn assistant">
        <div className="notice">{message.content}</div>
      </div>
    );
  }

  // Quante risposte sono state date a questa domanda, e quale si sta
  // guardando. Un messaggio senza rami ne ha una sola e le frecce non
  // compaiono.
  const variants = message.variants?.length ?? 1;
  const current = message.variant ?? 0;

  return (
    <div className="turn assistant">
      <div className="assistant-body">
        <Markdown text={message.content} />
        {streaming && <span className="caret" aria-hidden="true" />}
      </div>

      {!streaming && (message.sources?.length ?? 0) > 0 && (
        <div className="sources">
          <span className="sources-label">Fonti</span>
          {message.sources?.map((source, index) => (
            <a
              key={source.url}
              className="source-chip"
              href={source.url}
              target="_blank"
              rel="noreferrer noopener"
              title={source.snippet || source.url}
            >
              <span className="source-num">{index + 1}</span>
              {hostOf(source.url)}
            </a>
          ))}
        </div>
      )}

      {!streaming && (message.tools?.length ?? 0) > 0 && (
        <div className="sources">
          <span className="sources-label">Sul disco</span>
          {message.tools?.map((used, index) => (
            <span
              key={`${used.name}-${index}`}
              className="source-chip"
              title={`${TOOL_LABELS[used.name] ?? used.name}: ${used.detail}`}
            >
              {TOOL_LABELS[used.name] ?? used.name} {shortPath(used.detail)}
            </span>
          ))}
        </div>
      )}

      {!streaming && (
        <div className="turn-actions">
          {variants > 1 && onPickVariant && (
            <span className="variants" title="Risposte date alla stessa domanda">
              <button
                className="tool-btn"
                aria-label="Risposta precedente"
                onClick={() => onPickVariant((current - 1 + variants) % variants)}
              >
                ‹
              </button>
              <span className="variants-count">
                {current + 1}/{variants}
              </span>
              <button
                className="tool-btn"
                aria-label="Risposta successiva"
                onClick={() => onPickVariant((current + 1) % variants)}
              >
                ›
              </button>
            </span>
          )}
          <button className="tool-btn" onClick={() => void copy()}>
            {copied ? "Copiato" : "Copia"}
          </button>
          {canRegenerate && onRegenerate && (
            <button className="tool-btn" onClick={onRegenerate}>
              Rigenera
            </button>
          )}
        </div>
      )}
    </div>
  );
}
