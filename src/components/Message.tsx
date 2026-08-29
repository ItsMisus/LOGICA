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
}

/** Coda del tooltip di un allegato: da dove viene il testo che ha letto il
 *  modello, e se e' stato tagliato. */
function label(file: Attachment): string {
  const kind = file.kind === "pdf" ? " - testo estratto dal PDF" : file.kind === "docx" ? " - testo estratto dal documento" : "";
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

export default function Message({ message, streaming, canRegenerate, onRegenerate }: Props) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(message.content);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard negata dal sistema: si lascia perdere in silenzio.
    }
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
              {images.map((file) => (
                <img
                  key={file.path}
                  className="bubble-image"
                  src={`data:${file.image?.media_type};base64,${file.image?.data}`}
                  alt={file.name}
                  title={file.path}
                />
              ))}
            </div>
          )}
          {others.length > 0 && (
            <div className="bubble-files">
              {others.map((file) => (
                <span
                  key={file.path}
                  className={`file-chip ${file.error ? "failed" : ""}`}
                  title={file.error ? `${file.path} - ${file.error}` : `${file.path}${label(file)}`}
                >
                  {file.name}
                </span>
              ))}
            </div>
          )}
          {message.content}
        </div>
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

      {!streaming && (
        <div className="turn-actions">
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
