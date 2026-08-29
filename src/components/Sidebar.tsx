import { useEffect, useMemo, useRef, useState } from "react";

import type { ChatMatch, ChatMetadata, ExportFormat } from "../types";

interface Props {
  chats: ChatMetadata[];
  currentChatId: string;
  open: boolean;
  onNewChat: () => void;
  onSelectChat: (id: string) => void;
  onDeleteChat: (id: string) => void;
  onRenameChat: (id: string, title: string) => void;
  onDuplicateChat: (id: string) => void;
  onExportChat: (id: string, format: ExportFormat) => void;
  /** Cerca dentro le conversazioni: il lavoro lo fa il backend. */
  onSearch: (query: string) => Promise<ChatMatch[]>;
  onOpenSettings: () => void;
}

type Group = { label: string; chats: ChatMetadata[] };

/** Versione mostrata in fondo alla barra: tenuta allineata a package.json. */
const APP_VERSION = "0.1";

/** Scorciatoia della nuova chat: su Mac si scrive col simbolo del comando. */
const NEW_CHAT_SHORTCUT = navigator.userAgent.includes("Mac") ? "⌘N" : "Ctrl N";

/**
 * Attesa prima di cercare. La ricerca rilegge i file delle conversazioni:
 * farlo a ogni tasto premuto vorrebbe dire rileggerli cinque volte per una
 * parola di cinque lettere.
 */
const SEARCH_DELAY = 200;

/** Raggruppa le conversazioni per quanto sono recenti, come le app di chat. */
function groupByAge(chats: ChatMetadata[]): Group[] {
  const now = new Date();
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const day = 86_400_000;

  const groups: Group[] = [
    { label: "Oggi", chats: [] },
    { label: "Ieri", chats: [] },
    { label: "Ultimi 7 giorni", chats: [] },
    { label: "Prima", chats: [] },
  ];

  for (const chat of chats) {
    const time = new Date(chat.updated).getTime();
    if (Number.isNaN(time)) {
      groups[3].chats.push(chat);
    } else if (time >= startOfToday) {
      groups[0].chats.push(chat);
    } else if (time >= startOfToday - day) {
      groups[1].chats.push(chat);
    } else if (time >= startOfToday - 7 * day) {
      groups[2].chats.push(chat);
    } else {
      groups[3].chats.push(chat);
    }
  }

  return groups.filter((g) => g.chats.length > 0);
}

/** Ora per le conversazioni di oggi, data per quelle piu' vecchie. */
function stamp(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (n: number) => n.toString().padStart(2, "0");
  const now = new Date();
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  return sameDay
    ? `${pad(date.getHours())}:${pad(date.getMinutes())}`
    : `${pad(date.getDate())}.${pad(date.getMonth() + 1)}`;
}

export default function Sidebar({
  chats,
  currentChatId,
  open,
  onNewChat,
  onSelectChat,
  onDeleteChat,
  onRenameChat,
  onDuplicateChat,
  onExportChat,
  onSearch,
  onOpenSettings,
}: Props) {
  const [query, setQuery] = useState("");
  /** Risultati della ricerca nel contenuto: `null` = nessuna ricerca in corso. */
  const [matches, setMatches] = useState<ChatMatch[] | null>(null);
  const [searching, setSearching] = useState(false);
  /** Chat con il menu delle azioni aperto. */
  const [menuFor, setMenuFor] = useState<string | null>(null);
  /** Chat in fase di rinomina, con il testo che si sta scrivendo. */
  const [renaming, setRenaming] = useState<{ id: string; title: string } | null>(null);
  // Prima conferma dell'eliminazione: la voce cambia testo, non apre dialoghi.
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);

  const renameField = useRef<HTMLInputElement | null>(null);

  // La ricerca parte quando si smette di scrivere. Una risposta che arriva
  // dopo che la domanda e' cambiata viene buttata: altrimenti l'ultima lenta
  // sovrascriverebbe la piu' recente.
  useEffect(() => {
    const needle = query.trim();
    if (needle.length === 0) {
      setMatches(null);
      setSearching(false);
      return;
    }

    let cancelled = false;
    setSearching(true);
    const timer = setTimeout(() => {
      void onSearch(needle)
        .then((found) => {
          if (!cancelled) setMatches(found);
        })
        .catch(() => {
          if (!cancelled) setMatches([]);
        })
        .finally(() => {
          if (!cancelled) setSearching(false);
        });
    }, SEARCH_DELAY);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [query, onSearch]);

  useEffect(() => {
    if (renaming) renameField.current?.select();
  }, [renaming]);

  // Chiudere il menu con Escape: e' li' che la mano va per prima.
  useEffect(() => {
    if (!menuFor) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        setMenuFor(null);
        setPendingDelete(null);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [menuFor]);

  const groups = useMemo(() => groupByAge(chats), [chats]);

  function closeMenu() {
    setMenuFor(null);
    setPendingDelete(null);
  }

  function confirmRename() {
    if (!renaming) return;
    const title = renaming.title.trim();
    if (title.length > 0) onRenameChat(renaming.id, title);
    setRenaming(null);
  }

  /** Una riga della lista, uguale fra risultati e conversazioni recenti. */
  function row(chat: ChatMetadata, snippet?: string) {
    const editing = renaming?.id === chat.id;
    return (
      <div key={chat.id} className={`chat-item ${chat.id === currentChatId ? "active" : ""}`}>
        <div className="chat-item-row">
          <span className="chat-item-dot" aria-hidden="true" />

          {editing ? (
            <input
              ref={renameField}
              className="chat-item-rename"
              value={renaming.title}
              aria-label="Nuovo titolo"
              onChange={(e) => setRenaming({ id: chat.id, title: e.target.value })}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  confirmRename();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  setRenaming(null);
                }
              }}
              onBlur={confirmRename}
            />
          ) : (
            <button className="chat-item-main" title={chat.title} onClick={() => onSelectChat(chat.id)}>
              {chat.title}
            </button>
          )}

          <span className="chat-item-time">{stamp(chat.updated)}</span>

          <button
            className={`chat-item-more ${menuFor === chat.id ? "open" : ""}`}
            aria-label={`Azioni per ${chat.title}`}
            aria-expanded={menuFor === chat.id}
            onClick={() => {
              setPendingDelete(null);
              setMenuFor((id) => (id === chat.id ? null : chat.id));
            }}
          >
            ⋯
          </button>
        </div>

        {snippet && <p className="chat-item-snippet">{snippet}</p>}

        {menuFor === chat.id && (
          <div className="chat-menu" role="menu">
            <button
              role="menuitem"
              onClick={() => {
                setRenaming({ id: chat.id, title: chat.title });
                closeMenu();
              }}
            >
              Rinomina
            </button>
            <button
              role="menuitem"
              onClick={() => {
                onDuplicateChat(chat.id);
                closeMenu();
              }}
            >
              Duplica
            </button>
            <button
              role="menuitem"
              onClick={() => {
                onExportChat(chat.id, "markdown");
                closeMenu();
              }}
            >
              Esporta in Markdown
            </button>
            <button
              role="menuitem"
              onClick={() => {
                onExportChat(chat.id, "json");
                closeMenu();
              }}
            >
              Esporta in JSON
            </button>
            <button
              role="menuitem"
              className={`danger ${pendingDelete === chat.id ? "confirm" : ""}`}
              onClick={() => {
                if (pendingDelete === chat.id) {
                  onDeleteChat(chat.id);
                  closeMenu();
                } else {
                  setPendingDelete(chat.id);
                }
              }}
            >
              {pendingDelete === chat.id ? "Sicuro? Elimina" : "Elimina"}
            </button>
          </div>
        )}
      </div>
    );
  }

  return (
    <aside className={`sidebar ${open ? "" : "collapsed"}`} aria-hidden={!open}>
      <div className="brand">
        <span className="brand-mark" aria-hidden="true" />
        <span className="brand-name">Logica</span>
      </div>

      <button className="new-chat" onClick={onNewChat}>
        <span className="plus" aria-hidden="true">
          +
        </span>
        Nuova chat
        <span className="shortcut">{NEW_CHAT_SHORTCUT}</span>
      </button>

      <div className="search-box">
        <span className="glass" aria-hidden="true">
          ⌕
        </span>
        <input
          className="search"
          value={query}
          placeholder="Cerca nei messaggi"
          aria-label="Cerca nelle conversazioni"
          onChange={(e) => setQuery(e.target.value)}
        />
        {query.length > 0 && (
          <button className="search-clear" aria-label="Annulla la ricerca" onClick={() => setQuery("")}>
            ✕
          </button>
        )}
      </div>

      <nav className="chat-list" aria-label="Conversazioni">
        {matches !== null ? (
          <div className="chat-group">
            <h2 className="group-label">
              {searching ? "Cerco..." : `Risultati (${matches.length})`}
            </h2>
            {!searching && matches.length === 0 && <p className="empty-note">Nessun risultato.</p>}
            {matches.map((match) => row(match, match.snippet))}
          </div>
        ) : (
          <>
            {groups.length === 0 && <p className="empty-note">Nessuna conversazione.</p>}
            {groups.map((group) => (
              <div key={group.label} className="chat-group">
                <h2 className="group-label">{group.label}</h2>
                {group.chats.map((chat) => row(chat))}
              </div>
            ))}
          </>
        )}
      </nav>

      <button className="sidebar-foot" onClick={onOpenSettings}>
        Impostazioni
        <span className="version">V{APP_VERSION}</span>
      </button>
    </aside>
  );
}
