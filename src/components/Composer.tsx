import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { isDesktop } from "../api";

interface Props {
  /** Vero mentre la risposta e' in arrivo: il pulsante diventa "interrompi". */
  streaming: boolean;
  disabled: boolean;
  placeholder: string;
  /** Etichetta del motore mostrata nella barra, es. "llama3.1:8b". */
  engineLabel: string;
  /** Motore raggiungibile: colora il quadratino di stato accanto al modello. */
  online: boolean;
  autoFocus?: boolean;
  onSend: (text: string, attachmentPaths: string[]) => void;
  onStop: () => void;
  /** Il dialogo di sistema non si e' aperto: lo segnala la pagina. */
  onError: (message: string) => void;
}

/** Altezza massima del campo prima di iniziare a scorrere. */
const MAX_HEIGHT = 220;

/** Nome del file dal percorso, senza dipendere dal separatore di sistema. */
function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export default function Composer({
  streaming,
  disabled,
  placeholder,
  engineLabel,
  online,
  autoFocus,
  onSend,
  onStop,
  onError,
}: Props) {
  const [draft, setDraft] = useState("");
  // Percorsi scelti nel dialogo: il contenuto lo legge il backend all'invio.
  const [files, setFiles] = useState<string[]>([]);
  const field = useRef<HTMLTextAreaElement | null>(null);

  // Il campo cresce con il testo, come in una chat vera.
  useEffect(() => {
    const el = field.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT)}px`;
  }, [draft]);

  useEffect(() => {
    if (autoFocus) field.current?.focus();
  }, [autoFocus]);

  async function pickFiles() {
    try {
      const chosen = await open({ multiple: true, title: "Allega file" });
      if (!chosen) return;
      const paths = Array.isArray(chosen) ? chosen : [chosen];
      // Stesso file scelto due volte: si tiene una copia sola.
      setFiles((prev) => [...prev, ...paths.filter((p) => !prev.includes(p))]);
      field.current?.focus();
    } catch (e) {
      onError(`Allegati non aperti: ${String(e)}`);
    }
  }

  function send() {
    const text = draft.trim();
    if ((!text && files.length === 0) || streaming || disabled) return;
    setDraft("");
    setFiles([]);
    onSend(text, files);
  }

  const canSend = !disabled && (draft.trim().length > 0 || files.length > 0);

  return (
    <div className="composer">
      <div className="composer-box">
        {files.length > 0 && (
          <div className="composer-files">
            {files.map((path) => (
              <span key={path} className="file-chip" title={path}>
                {baseName(path)}
                <button
                  className="file-chip-del"
                  aria-label={`Togli ${baseName(path)}`}
                  onClick={() => setFiles((prev) => prev.filter((p) => p !== path))}
                >
                  ✕
                </button>
              </span>
            ))}
          </div>
        )}

        <textarea
          ref={field}
          className="composer-input"
          rows={1}
          value={draft}
          placeholder={placeholder}
          aria-label="Messaggio"
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            // Invio manda, Shift+Invio va a capo.
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              send();
            }
          }}
        />

        <div className="composer-actions">
          {/* Da browser non c'e' dialogo di sistema, e gli allegati vengono
              letti sul PC per percorso: un pulsante qui non avrebbe niente da
              aprire. */}
          {isDesktop && (
            <button
              className="composer-add"
              onClick={() => void pickFiles()}
              title="Allega file"
              aria-label="Allega file"
            >
              +
            </button>
          )}

          <span className="composer-engine" title={online ? "motore attivo" : "nessun modello attivo"}>
            <span className={`engine-dot ${online ? "on" : "off"}`} aria-hidden="true" />
            {engineLabel}
          </span>

          {streaming ? (
            <button className="square-btn stop" onClick={onStop} title="Interrompi la risposta" aria-label="Interrompi">
              <span className="square" />
            </button>
          ) : (
            <button
              className="square-btn send"
              onClick={send}
              disabled={!canSend}
              title="Invia (Invio)"
              aria-label="Invia"
            >
              →
            </button>
          )}
        </div>
      </div>
      <p className="composer-hint">Invio per inviare · Shift+Invio per andare a capo</p>
    </div>
  );
}
