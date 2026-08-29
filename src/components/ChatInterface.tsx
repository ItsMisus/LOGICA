import { useEffect, useRef, useState } from "react";

import OrganizationProposalBlock from "./OrganizationProposalBlock";
import type { ChatMessage, OrganizationProposal, PlanPreview } from "../types";

interface Props {
  messages: ChatMessage[];
  /** Proposta ancora approvabile: quella dell'ultimo blocco, se non e' stata applicata. */
  activeProposalId: string | null;
  preview: PlanPreview | null;
  isLoading: boolean;
  busy: boolean;
  /** Testo di stato durante scansione/spostamento, es. "1.240 file scansionati". */
  progressLabel: string;
  online: boolean;
  engineLabel: string;
  undoAvailable: boolean;
  folders: string[];
  fileCount: number;
  /** Cartelle monitorate: scorciatoie mostrate finche' la chat e' vuota. */
  suggestions: string[];
  onSendMessage: (message: string) => void;
  onProposalChange: (p: OrganizationProposal) => void;
  onApprove: () => void;
  onCancelProposal: () => void;
  onUndo: () => void;
  onCancelApply: () => void;
  onAddFolder: () => void;
  onOpenSettings: () => void;
}

function Bubble({ message }: { message: ChatMessage }) {
  const isUser = message.role === "user";
  const time = new Date(message.timestamp);
  return (
    <div className={`bubble-row ${isUser ? "user" : "ai"}`}>
      {!isUser && <div className="avatar" aria-hidden="true">L</div>}
      <div className={`bubble ${message.type === "notification" ? "notification" : ""}`}>
        {message.content.split("\n").map((line, i) => (
          <p key={i}>{line}</p>
        ))}
        <span className="time">
          {Number.isNaN(time.getTime()) ? "" : time.toLocaleTimeString("it-IT", { hour: "2-digit", minute: "2-digit" })}
        </span>
      </div>
    </div>
  );
}

export default function ChatInterface({
  messages,
  activeProposalId,
  preview,
  isLoading,
  busy,
  progressLabel,
  online,
  engineLabel,
  undoAvailable,
  folders,
  fileCount,
  suggestions,
  onSendMessage,
  onProposalChange,
  onApprove,
  onCancelProposal,
  onUndo,
  onCancelApply,
  onAddFolder,
  onOpenSettings,
}: Props) {
  const [draft, setDraft] = useState("");
  const bottomRef = useRef<HTMLDivElement | null>(null);

  // Auto-scroll: la conversazione resta ancorata all'ultimo messaggio.
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [messages, isLoading, progressLabel]);

  function send() {
    const text = draft.trim();
    if (!text || isLoading || busy) return;
    setDraft("");
    onSendMessage(text);
  }

  return (
    <section className="chat">
      <header className="chat-head">
        <div>
          <h1>Logica</h1>
          <span className={`status ${online ? "on" : "off"}`}>
            {online ? `online · ${engineLabel}` : "offline · nessun modello AI"}
          </span>
        </div>
        <div className="row">
          {folders.length > 0 && (
            <span className="badge" title={folders.join("\n")}>
              {folders.length === 1 ? folders[0].split(/[\\/]/).pop() : `${folders.length} cartelle`} ·{" "}
              {fileCount.toLocaleString("it-IT")} file
            </span>
          )}
          <button className="btn ghost tiny" onClick={onOpenSettings} aria-label="Impostazioni">
            ⚙️
          </button>
        </div>
      </header>

      <div className="messages">
        {messages.map((message) =>
          message.type === "proposal_block" && message.proposalData ? (
            <OrganizationProposalBlock
              key={message.id}
              proposal={message.proposalData}
              preview={message.id === activeProposalId ? preview : null}
              active={message.id === activeProposalId}
              busy={busy}
              onProposalChange={onProposalChange}
              onApprove={onApprove}
              onCancel={onCancelProposal}
            />
          ) : (
            <Bubble key={message.id} message={message} />
          ),
        )}

        {(isLoading || busy) && (
          <div className="bubble-row ai">
            <div className="avatar" aria-hidden="true">L</div>
            <div className="bubble typing">
              <span className="dot" />
              <span className="dot" />
              <span className="dot" />
              {progressLabel && <span className="progress-label">{progressLabel}</span>}
              {/* Interrompere durante lo spostamento riporta i file indietro (rollback). */}
              {busy && (
                <button className="btn ghost tiny danger" onClick={onCancelApply}>
                  Interrompi
                </button>
              )}
            </div>
          </div>
        )}

        <div ref={bottomRef} />
      </div>

      {suggestions.length > 0 && messages.length <= 1 && (
        <div className="suggestions">
          <span className="hint">Cartelle monitorate:</span>
          {suggestions.map((folder) => (
            <button
              key={folder}
              className="btn chip"
              disabled={isLoading || busy}
              title={folder}
              onClick={() => onSendMessage(`organizza "${folder}"`)}
            >
              📁 {folder.split(/[\\/]/).pop() || folder}
            </button>
          ))}
        </div>
      )}

      <footer className="composer">
        <button className="btn ghost" onClick={onAddFolder} disabled={busy} title="Aggiungi una cartella">
          📁
        </button>
        <textarea
          className="input composer-input"
          rows={1}
          value={draft}
          placeholder="Scrivi a Logica: “organizza Downloads”"
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
        <button className="btn primary" onClick={send} disabled={isLoading || busy || !draft.trim()}>
          Invia
        </button>
        <button className="btn ghost" onClick={onUndo} disabled={!undoAvailable || busy} title="Annulla l'ultima operazione">
          ↩ Undo
        </button>
      </footer>
    </section>
  );
}
