import { useMemo, useState } from "react";

import type { ChatMetadata } from "../types";

interface Props {
  chats: ChatMetadata[];
  currentChatId: string;
  open: boolean;
  onNewChat: () => void;
  onSelectChat: (id: string) => void;
  onDeleteChat: (id: string) => void;
  onOpenSettings: () => void;
}

type Group = { label: string; chats: ChatMetadata[] };

/** Versione mostrata in fondo alla barra: tenuta allineata a package.json. */
const APP_VERSION = "0.1";

/** Scorciatoia della nuova chat: su Mac si scrive col simbolo del comando. */
const NEW_CHAT_SHORTCUT = navigator.userAgent.includes("Mac") ? "⌘N" : "Ctrl N";

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
  onOpenSettings,
}: Props) {
  const [query, setQuery] = useState("");
  // Prima conferma dell'eliminazione: il pulsante cambia testo, non apre dialoghi.
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);

  const groups = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const filtered = needle ? chats.filter((c) => c.title.toLowerCase().includes(needle)) : chats;
    return groupByAge(filtered);
  }, [chats, query]);

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
          placeholder="Cerca"
          aria-label="Cerca nelle conversazioni"
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      <nav className="chat-list" aria-label="Conversazioni">
        {groups.length === 0 && (
          <p className="empty-note">{chats.length === 0 ? "Nessuna conversazione." : "Nessun risultato."}</p>
        )}

        {groups.map((group) => (
          <div key={group.label} className="chat-group">
            <h2 className="group-label">{group.label}</h2>
            {group.chats.map((chat) => (
              <div key={chat.id} className={`chat-item ${chat.id === currentChatId ? "active" : ""}`}>
                <span className="chat-item-dot" aria-hidden="true" />
                <button className="chat-item-main" title={chat.title} onClick={() => onSelectChat(chat.id)}>
                  {chat.title}
                </button>
                <span className="chat-item-time">{stamp(chat.updated)}</span>
                <button
                  className={`chat-item-del ${pendingDelete === chat.id ? "confirm" : ""}`}
                  aria-label={`Elimina ${chat.title}`}
                  onClick={() => {
                    if (pendingDelete === chat.id) {
                      onDeleteChat(chat.id);
                      setPendingDelete(null);
                    } else {
                      setPendingDelete(chat.id);
                    }
                  }}
                  onBlur={() => setPendingDelete((id) => (id === chat.id ? null : id))}
                >
                  {pendingDelete === chat.id ? "Sicuro?" : "✕"}
                </button>
              </div>
            ))}
          </div>
        ))}
      </nav>

      <button className="sidebar-foot" onClick={onOpenSettings}>
        Impostazioni
        <span className="version">V{APP_VERSION}</span>
      </button>
    </aside>
  );
}
