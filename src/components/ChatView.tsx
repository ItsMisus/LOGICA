import { useEffect, useRef } from "react";

import Composer from "./Composer";
import Message from "./Message";
import type { Attachment, ChatEngine, ChatMessage } from "../types";

interface Props {
  messages: ChatMessage[];
  /** Testo che sta arrivando dal modello, ancora fuori da `messages`. */
  streamingText: string | null;
  waiting: boolean;
  /** Ricerca o pagina in corso, mostrata mentre non c'e' ancora testo. */
  statusText: string | null;
  online: boolean;
  engineLabel: string;
  /** Motore scelto per questa chat, vuoto se vale quello delle impostazioni. */
  engine: ChatEngine;
  /** Il motore delle impostazioni, voce "predefinito" del menu. */
  defaultLabel: string;
  /** Modelli Ollama installati, per il menu del composer. */
  engineChoices: string[];
  claudeReady: boolean;
  onPickEngine: (engine: ChatEngine) => void;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  onSend: (text: string, attachmentPaths: string[], pasted: Attachment[]) => void;
  onStop: () => void;
  onRegenerate: () => void;
  /** Riscrive un messaggio dell'utente e riparte da li'. */
  onEdit: (messageId: string, text: string) => void;
  /** Passa a un'altra risposta fra quelle date alla stessa domanda. */
  onPickVariant: (messageId: string, index: number) => void;
  /** Errori dei componenti figli (dialogo allegati) mostrati come notifica. */
  onError: (message: string) => void;
}

const STARTERS = [
  "Spiegami una cosa che non capisco",
  "Aiutami a scrivere un messaggio",
  "Rivedi questo codice con me",
  "Ragioniamo su una decisione",
];

function greeting(): string {
  const hour = new Date().getHours();
  if (hour < 5) return "Ancora sveglio";
  if (hour < 13) return "Buongiorno";
  if (hour < 18) return "Buon pomeriggio";
  return "Buonasera";
}

/** Distanza dal fondo entro cui si continua a seguire il testo che arriva. */
const FOLLOW_THRESHOLD = 120;

export default function ChatView({
  messages,
  streamingText,
  waiting,
  statusText,
  online,
  engineLabel,
  engine,
  defaultLabel,
  engineChoices,
  claudeReady,
  onPickEngine,
  sidebarOpen,
  onToggleSidebar,
  onSend,
  onStop,
  onRegenerate,
  onEdit,
  onPickVariant,
  onError,
}: Props) {
  const scroller = useRef<HTMLDivElement | null>(null);
  // Se l'utente e' risalito a leggere, lo streaming non deve riportarlo giu'.
  const follow = useRef(true);

  useEffect(() => {
    const el = scroller.current;
    if (!el || !follow.current) return;
    el.scrollTop = el.scrollHeight;
  }, [messages, streamingText, waiting, statusText]);

  function onScroll() {
    const el = scroller.current;
    if (!el) return;
    follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_THRESHOLD;
  }

  const empty = messages.length === 0 && streamingText === null && !waiting;
  const streaming = streamingText !== null || waiting;
  const lastAssistant = [...messages].reverse().find((m) => m.role === "ai");

  const composer = (
    <Composer
      streaming={streaming}
      disabled={false}
      placeholder={online ? "Scrivi a Logica..." : "Nessun modello attivo: controlla le impostazioni"}
      engineLabel={engineLabel}
      engine={engine}
      defaultLabel={defaultLabel}
      engineChoices={engineChoices}
      claudeReady={claudeReady}
      onPickEngine={onPickEngine}
      online={online}
      autoFocus
      onSend={(text, paths, pasted) => {
        follow.current = true;
        onSend(text, paths, pasted);
      }}
      onStop={onStop}
      onError={onError}
    />
  );

  return (
    <section className="chat">
      <header className="chat-head">
        <button
          className="icon-btn"
          onClick={onToggleSidebar}
          title={sidebarOpen ? "Nascondi le conversazioni" : "Mostra le conversazioni"}
          aria-label="Conversazioni"
        >
          ☰
        </button>
      </header>

      {empty ? (
        <div className="hero">
          <h1 className="hero-title">{greeting()}</h1>
          <div className="hero-composer">{composer}</div>
          <div className="starters">
            {STARTERS.map((text, index) => (
              <button key={text} className="starter" onClick={() => onSend(text, [], [])}>
                <span className="starter-num" aria-hidden="true">
                  {(index + 1).toString().padStart(2, "0")}
                </span>
                {text}
              </button>
            ))}
          </div>
        </div>
      ) : (
        <>
          <div className="thread" ref={scroller} onScroll={onScroll}>
            <div className="thread-inner">
              {messages.map((message) => (
                <Message
                  key={message.id}
                  message={message}
                  canRegenerate={!streaming && message.id === lastAssistant?.id}
                  onRegenerate={onRegenerate}
                  canEdit={!streaming && message.role === "user"}
                  onEdit={(text) => onEdit(message.id, text)}
                  onPickVariant={(index) => onPickVariant(message.id, index)}
                />
              ))}

              {streamingText !== null && (
                <Message
                  message={{
                    id: "streaming",
                    role: "ai",
                    content: streamingText,
                    timestamp: new Date().toISOString(),
                    type: "text",
                  }}
                  streaming
                />
              )}

              {waiting && streamingText === null && (
                <div className="turn assistant">
                  {statusText && <div className="working">{statusText}</div>}
                  <div className="thinking" aria-label="Sto scrivendo">
                    <span className="dot" />
                    <span className="dot" />
                    <span className="dot" />
                  </div>
                </div>
              )}
            </div>
          </div>
          {composer}
        </>
      )}
    </section>
  );
}
