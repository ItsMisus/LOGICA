import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { call, isDesktop } from "../api";
import type { Attachment, ChatEngine } from "../types";

interface Props {
  /** Vero mentre la risposta e' in arrivo: il pulsante diventa "interrompi". */
  streaming: boolean;
  disabled: boolean;
  placeholder: string;
  /** Etichetta del motore mostrata nella barra, es. "llama3.1:8b". */
  engineLabel: string;
  /** Motore scelto per questa chat; vuoto = quello delle impostazioni. */
  engine: ChatEngine;
  /** Come si chiama il motore delle impostazioni: e' la voce "predefinito" del
   *  menu, e resta quella anche quando la chat ne ha scelto un altro. */
  defaultLabel: string;
  /** Modelli Ollama installati. Vuoto quando Ollama non risponde: allora
   *  l'etichetta resta un'etichetta e non diventa un menu con una voce sola. */
  engineChoices: string[];
  claudeReady: boolean;
  onPickEngine: (engine: ChatEngine) => void;
  /** Motore raggiungibile: colora il quadratino di stato accanto al modello. */
  online: boolean;
  autoFocus?: boolean;
  onSend: (text: string, attachmentPaths: string[], pasted: Attachment[]) => void;
  onStop: () => void;
  /** Il dialogo di sistema non si e' aperto: lo segnala la pagina. */
  onError: (message: string) => void;
}

/** Altezza massima del campo prima di iniziare a scorrere. */
const MAX_HEIGHT = 220;

/**
 * Cosa il backend sa leggere davvero, in cima al dialogo di sistema. L'ultima
 * voce lascia scegliere qualunque file: un `.conf` o un `.ini` sono testo lo
 * stesso, e vietarlo qui vorrebbe dire indovinare al posto dell'utente.
 */
const FILTERS = [
  { name: "Allegati leggibili", extensions: ["txt", "md", "csv", "json", "xml", "yml", "yaml", "log", "pdf", "docx", "xlsx", "png", "jpg", "jpeg", "gif", "webp"] },
  { name: "Immagini", extensions: ["png", "jpg", "jpeg", "gif", "webp"] },
  { name: "Documenti", extensions: ["pdf", "docx", "xlsx"] },
  { name: "Tutti i file", extensions: ["*"] },
];

/** Il contenuto di un file in base64, senza il prefisso `data:`. */
function base64Of(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("Immagine non letta dagli appunti."));
    reader.onload = () => {
      const result = String(reader.result);
      resolve(result.slice(result.indexOf(",") + 1));
    };
    reader.readAsDataURL(file);
  });
}

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
  engine,
  defaultLabel,
  engineChoices,
  claudeReady,
  onPickEngine,
  online,
  autoFocus,
  onSend,
  onStop,
  onError,
}: Props) {
  const [draft, setDraft] = useState("");
  // Percorsi scelti nel dialogo o trascinati: il contenuto lo legge il backend
  // all'invio.
  const [files, setFiles] = useState<string[]>([]);
  // Immagini incollate: non hanno un percorso, quindi sono gia' state lette al
  // momento dell'incolla e viaggiano gia' pronte.
  const [pasted, setPasted] = useState<Attachment[]>([]);
  const [dragging, setDragging] = useState(false);
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
      const chosen = await open({ multiple: true, title: "Allega file", filters: FILTERS });
      if (!chosen) return;
      const paths = Array.isArray(chosen) ? chosen : [chosen];
      // Stesso file scelto due volte: si tiene una copia sola.
      setFiles((prev) => [...prev, ...paths.filter((p) => !prev.includes(p))]);
      field.current?.focus();
    } catch (e) {
      onError(`Allegati non aperti: ${String(e)}`);
    }
  }

  /**
   * Trascinare un file nella finestra. Non e' il drag-and-drop del browser: la
   * webview di Tauri lo intercetta e passa i percorsi veri, che e' quello che
   * serve, perche' gli allegati li legge il backend per percorso. Da browser
   * quei percorsi non esisterebbero, quindi li' non si aggancia niente.
   */
  useEffect(() => {
    if (!isDesktop) return;
    let stop: (() => void) | undefined;
    void (async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      stop = await getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "over") {
          setDragging(true);
          return;
        }
        setDragging(false);
        if (event.payload.type !== "drop") return;
        const paths = event.payload.paths;
        setFiles((prev) => [...prev, ...paths.filter((p) => !prev.includes(p))]);
        field.current?.focus();
      });
    })().catch(() => undefined);
    return () => stop?.();
  }, []);

  /**
   * Incollare uno screenshot. Un'immagine negli appunti non ha un percorso da
   * dare al backend: si manda il contenuto, e torna un allegato gia' letto.
   * E' l'unico allegato che funziona anche da browser.
   */
  async function pasteImages(event: React.ClipboardEvent<HTMLTextAreaElement>) {
    const images = Array.from(event.clipboardData.files).filter((f) =>
      f.type.startsWith("image/"),
    );
    if (images.length === 0) return;
    // C'e' un'immagine: il testo degli appunti, se c'e', non deve finire anche
    // dentro il campo.
    event.preventDefault();

    for (const image of images) {
      try {
        const data = await base64Of(image);
        const attached = await call<Attachment>("attach_image", {
          name: image.name || "immagine incollata",
          mediaType: image.type,
          data,
        });
        setPasted((prev) => [...prev, attached]);
      } catch (e) {
        onError(String(e));
      }
    }
  }

  function send() {
    const text = draft.trim();
    if ((!text && files.length === 0 && pasted.length === 0) || streaming || disabled) return;
    setDraft("");
    setFiles([]);
    setPasted([]);
    onSend(text, files, pasted);
  }

  const canSend =
    !disabled && (draft.trim().length > 0 || files.length > 0 || pasted.length > 0);

  return (
    <div className="composer">
      <div className={`composer-box ${dragging ? "dragging" : ""}`}>
        {(files.length > 0 || pasted.length > 0) && (
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
            {pasted.map((image, i) => (
              <span key={`incollata-${i}`} className="file-chip" title="immagine incollata">
                {image.name}
                <button
                  className="file-chip-del"
                  aria-label={`Togli ${image.name}`}
                  onClick={() => setPasted((prev) => prev.filter((_, at) => at !== i))}
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
          onPaste={(e) => void pasteImages(e)}
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
            {engineChoices.length > 0 || claudeReady ? (
              <select
                className="engine-pick"
                value={engine}
                aria-label="Motore di questa conversazione"
                title="Vale solo per questa conversazione"
                onChange={(e) => onPickEngine(e.target.value)}
              >
                <option value="">{defaultLabel} (predefinito)</option>
                {claudeReady && <option value="claude">claude-api</option>}
                {engineChoices.map((model) => (
                  <option key={model} value={`ollama:${model}`}>
                    {model}
                  </option>
                ))}
              </select>
            ) : (
              engineLabel
            )}
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
