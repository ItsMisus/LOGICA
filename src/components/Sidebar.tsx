import { useMemo } from "react";
import type { ChatMetadata } from "../types";

interface Props {
  chats: ChatMetadata[];
  currentChatId: string;
  onNewChat: () => void;
  onSelectChat: (id: string) => void;
  onDeleteChat: (id: string) => void;
  onOpenSettings: () => void;
}

/** "oggi 14:32" / "ieri" / "12 mar" a seconda di quanto e' vecchia la chat. */
function relativeDate(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const now = new Date();
  const sameDay = date.toDateString() === now.toDateString();
  if (sameDay) return date.toLocaleTimeString("it-IT", { hour: "2-digit", minute: "2-digit" });

  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (date.toDateString() === yesterday.toDateString()) return "ieri";

  return date.toLocaleDateString("it-IT", { day: "numeric", month: "short" });
}

export default function Sidebar({
  chats,
  currentChatId,
  onNewChat,
  onSelectChat,
  onDeleteChat,
  onOpenSettings,
}: Props) {
  const monthlyFiles = useMemo(() => {
    const now = new Date();
    return chats.reduce((sum, chat) => {
      if (!chat.applied_on) return sum;
      const applied = new Date(chat.applied_on);
      const sameMonth =
        applied.getFullYear() === now.getFullYear() && applied.getMonth() === now.getMonth();
      return sameMonth ? sum + chat.files_moved : sum;
    }, 0);
  }, [chats]);

  const totalFiles = useMemo(() => chats.reduce((s, c) => s + c.files_moved, 0), [chats]);

  return (
    <aside className="sidebar">
      <button className="btn primary block" onClick={onNewChat}>
        + Nuova chat
      </button>

      <nav className="chat-list" aria-label="Conversazioni">
        {chats.length === 0 && <p className="hint">Nessuna conversazione salvata.</p>}
        {chats.map((chat) => (
          <div key={chat.id} className={`chat-item ${chat.id === currentChatId ? "active" : ""}`}>
            <button className="chat-item-main" onClick={() => onSelectChat(chat.id)} title={chat.title}>
              <span className="chat-title">{chat.title}</span>
              <span className="chat-meta">
                {relativeDate(chat.updated)}
                {chat.files_moved > 0 && ` · ${chat.files_moved} file`}
              </span>
            </button>
            <button
              className="btn ghost tiny danger"
              onClick={() => onDeleteChat(chat.id)}
              aria-label={`Elimina ${chat.title}`}
            >
              ✕
            </button>
          </div>
        ))}
      </nav>

      <div className="sidebar-foot">
        <div className="stat">
          <strong>{monthlyFiles.toLocaleString("it-IT")}</strong>
          <span>file organizzati questo mese</span>
        </div>
        <div className="stat">
          <strong>{totalFiles.toLocaleString("it-IT")}</strong>
          <span>in totale</span>
        </div>
        <button className="btn ghost block" onClick={onOpenSettings}>
          ⚙️ Impostazioni
        </button>
      </div>
    </aside>
  );
}
