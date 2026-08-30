/**
 * La palette dei comandi: Ctrl+K, si scrive, si preme Invio.
 *
 * Non sa niente di cosa fa l'app: riceve una lista di voci già pronte e le
 * filtra. Chi decide quali voci esistono è la pagina, che è anche l'unica che
 * sa aprire una conversazione o cambiare motore.
 */
import { useEffect, useMemo, useRef, useState } from "react";

export interface Command {
  id: string;
  label: string;
  /** Sotto quale intestazione compare: "Vai a", "Motore", "Conversazione". */
  group: string;
  /** Scorciatoia o dettaglio, a destra della voce. */
  hint?: string;
  run: () => void;
}

interface Props {
  commands: Command[];
  onClose: () => void;
}

/**
 * Corrispondenza per sottosequenza: "nch" trova "Nuova chat". È come si
 * comporta ogni palette che l'utente ha già usato altrove, e su una lista di
 * poche decine di voci non serve niente di più furbo.
 */
export function matchesLabel(label: string, query: string): boolean {
  if (query.length === 0) return true;
  const text = label.toLowerCase();
  let at = 0;
  for (const letter of query.toLowerCase()) {
    if (letter === " ") continue;
    at = text.indexOf(letter, at);
    if (at === -1) return false;
    at += 1;
  }
  return true;
}

export default function CommandPalette({ commands, onClose }: Props) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const list = useRef<HTMLDivElement | null>(null);

  const visible = useMemo(
    () => commands.filter((c) => matchesLabel(c.label, query) || matchesLabel(c.group, query)),
    [commands, query],
  );

  // Filtrando, la voce selezionata può finire fuori dalla lista.
  useEffect(() => {
    setSelected(0);
  }, [query]);

  // La riga scelta con le frecce deve restare visibile anche quando la lista
  // è più lunga della finestra.
  useEffect(() => {
    list.current?.querySelector<HTMLElement>('[data-selected="true"]')?.scrollIntoView({
      block: "nearest",
    });
  }, [selected]);

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelected((i) => (visible.length === 0 ? 0 : (i + 1) % visible.length));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelected((i) => (visible.length === 0 ? 0 : (i - 1 + visible.length) % visible.length));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const chosen = visible[selected];
      if (chosen) {
        onClose();
        chosen.run();
      }
    }
  }

  let lastGroup = "";

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="palette"
        role="dialog"
        aria-label="Comandi"
        onClick={(e) => e.stopPropagation()}
      >
        <input
          className="palette-input"
          value={query}
          autoFocus
          placeholder="Cerca un comando o una conversazione…"
          aria-label="Cerca un comando"
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
        />

        <div className="palette-list" ref={list}>
          {visible.length === 0 && <p className="palette-empty">Niente che somigli a «{query}».</p>}
          {visible.map((command, i) => {
            const header = command.group !== lastGroup ? command.group : null;
            lastGroup = command.group;
            return (
              <div key={command.id}>
                {header && <div className="palette-group">{header}</div>}
                <button
                  className={`palette-item ${i === selected ? "selected" : ""}`}
                  data-selected={i === selected}
                  // `mousedown` e non `click`: il campo perde il fuoco prima
                  // che il click arrivi, e la palette si chiuderebbe da sola.
                  onMouseDown={(e) => {
                    e.preventDefault();
                    onClose();
                    command.run();
                  }}
                  onMouseEnter={() => setSelected(i)}
                >
                  <span>{command.label}</span>
                  {command.hint && <span className="palette-hint">{command.hint}</span>}
                </button>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
