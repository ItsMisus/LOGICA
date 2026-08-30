import { useCallback, useEffect, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";

import { call, isDesktop, subscribe } from "../api";

import ChatView from "../components/ChatView";
import NotificationToast from "../components/NotificationToast";
import Sidebar from "../components/Sidebar";
import SettingsModal from "../components/SettingsModal";

import {
  DEFAULT_CONFIG,
  DEFAULT_PREFERENCES,
  type AiStatus,
  type Attachment,
  type AttachmentImage,
  type AssistantDelta,
  type AssistantStatus,
  type ChatMatch,
  type ChatMessage,
  type ChatMetadata,
  type ChatTurn,
  type Config,
  type ExportFormat,
  type Preferences,
  type StoredChat,
  type TalkReply,
  type Toast,
  type ToolUse,
  type WebSource,
} from "../types";

let idCounter = 0;
function newId(prefix: string): string {
  idCounter += 1;
  return `${prefix}${Date.now().toString(36)}${idCounter.toString(36)}`;
}

/** Id chat leggibile e sicuro lato backend: solo cifre, lettere e underscore. */
function newChatId(): string {
  const now = new Date();
  const pad = (n: number) => n.toString().padStart(2, "0");
  const stamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}_${pad(now.getHours())}${pad(
    now.getMinutes(),
  )}${pad(now.getSeconds())}`;
  return `${stamp}_${Math.random().toString(36).slice(2, 6)}`;
}

function message(role: ChatMessage["role"], content: string, type: ChatMessage["type"] = "text"): ChatMessage {
  return { id: newId("m"), role, content, timestamp: new Date().toISOString(), type };
}

/**
 * Cosa vede il modello di un messaggio: il testo scritto piu' il contenuto dei
 * file allegati. Nella chat gli allegati restano una riga di etichette, cosi'
 * la conversazione non si riempie di dump.
 */
function promptContent(m: ChatMessage): string {
  if (!m.attachments || m.attachments.length === 0) return m.content;
  const blocks = m.attachments.map((a) => {
    // Le immagini non hanno testo: i byte viaggiano a parte, qui resta
    // l'etichetta, cosi' il modello sa a cosa si riferisce la domanda.
    if (a.kind === "image" && a.image) return `--- immagine allegata: ${a.name} ---`;
    if (a.text === null) return `--- allegato: ${a.name} (non letto: ${a.error ?? "motivo ignoto"}) ---`;
    const tail = a.truncated ? "\n[...contenuto troncato]" : "";
    return `--- allegato: ${a.name} ---\n${a.text}${tail}`;
  });
  return [m.content, ...blocks].filter((part) => part.length > 0).join("\n\n");
}

/** Le immagini di un messaggio, gia' pronte per il modello. */
function promptImages(m: ChatMessage): AttachmentImage[] {
  return (m.attachments ?? []).flatMap((a) => (a.image ? [a.image] : []));
}

/**
 * Nome del file proposto quando si esporta. Si tengono lettere, cifre, spazi e
 * trattini: il resto puo' essere un carattere che il sistema non accetta in un
 * nome di file, e un dialogo che si rifiuta di aprirsi non si spiega da solo.
 */
function fileNameFor(title: string | undefined): string {
  const clean = (title ?? "")
    .replace(/[^\p{L}\p{N} _-]/gu, "")
    .replace(/\s+/g, " ")
    .trim();
  return clean.length > 0 ? clean.slice(0, 60) : "conversazione";
}

/** Titolo di ripiego, finche' il modello non ne propone uno migliore. */
function titleFrom(text: string): string {
  const clean = text.replace(/\s+/g, " ").trim();
  return clean.length > 48 ? `${clean.slice(0, 48)}...` : clean;
}

const UNTITLED = "Nuova chat";

/**
 * Modelli locali che sanno guardare una immagine. Un modello solo testuale
 * riceve le immagini e le ignora in silenzio: meglio dirlo prima, invece di
 * lasciar credere che la risposta le abbia viste.
 */
const VISION_MODELS = ["llava", "vision", "moondream", "bakllava", "minicpm-v", "-vl", "gemma3"];

function seesImages(model: string): boolean {
  const name = model.toLowerCase();
  return VISION_MODELS.some((mark) => name.includes(mark));
}

export default function ChatPage() {
  const [preferences, setPreferences] = useState<Preferences>(DEFAULT_PREFERENCES);
  const [config, setConfig] = useState<Config>(DEFAULT_CONFIG);
  const [status, setStatus] = useState<AiStatus | null>(null);

  const [chats, setChats] = useState<ChatMetadata[]>([]);
  const [chatId, setChatId] = useState<string>(newChatId);
  const [created, setCreated] = useState<string>(() => new Date().toISOString());
  const [title, setTitle] = useState(UNTITLED);
  const [messages, setMessages] = useState<ChatMessage[]>([]);

  /** Testo in arrivo dal modello: vive fuori da `messages` finche' non finisce. */
  const [streamingText, setStreamingText] = useState<string | null>(null);
  const [waiting, setWaiting] = useState(false);
  /** Cosa sta facendo l'app prima di rispondere: la ricerca, la pagina aperta. */
  const [statusText, setStatusText] = useState<string | null>(null);

  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);

  // L'ascoltatore degli eventi vive quanto la pagina: la chat corrente e i
  // parametri del modello glieli passiamo per riferimento, non per closure.
  const currentChat = useRef(chatId);
  useEffect(() => {
    currentChat.current = chatId;
  }, [chatId]);

  const toast = useCallback((kind: Toast["kind"], text: string) => {
    setToasts((prev) => [...prev, { id: newId("t"), kind, message: text }]);
  }, []);

  const dismissToast = useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  /* ------------------------------ avvio e stato ----------------------------- */

  useEffect(() => {
    void (async () => {
      try {
        const [prefs, cfg, list] = await Promise.all([
          call<Preferences>("get_preferences"),
          call<Config>("get_config"),
          call<ChatMetadata[]>("list_chats"),
        ]);
        setPreferences(prefs);
        setConfig(cfg);
        setChats(list);
      } catch (e) {
        toast("error", `Impossibile leggere ~/.logica: ${String(e)}`);
      }
    })();
  }, [toast]);

  useEffect(() => {
    document.documentElement.dataset.theme = preferences.theme;
  }, [preferences.theme]);

  // Stato dei motori: all'avvio e a ogni cambio di motore, indirizzo o modello.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const s = await call<AiStatus>("check_ai_status", { ollamaUrl: preferences.ollama_url });
      if (!cancelled) setStatus(s);
    })().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [preferences.ollama_url, preferences.use_premium_ai, preferences.ollama_model]);

  // Effetto macchina da scrivere: ogni evento e' un pezzo di risposta.
  useEffect(() => {
    return subscribe<AssistantDelta>("assistant-delta", (delta) => {
      if (delta.chat_id !== currentChat.current) return;
      // Il primo pezzo di testo chiude la riga di stato: da qui in poi si legge.
      setStatusText(null);
      setStreamingText((prev) => (prev ?? "") + delta.text);
    });
  }, []);

  // Ricerche e pagine aperte, mentre lo schermo sarebbe fermo.
  useEffect(() => {
    return subscribe<AssistantStatus>("assistant-status", (status) => {
      if (status.chat_id !== currentChat.current) return;
      setStatusText(status.text);
    });
  }, []);

  /* -------------------------------- persistenza ------------------------------ */

  const persist = useCallback(
    async (msgs: ChatMessage[], patch: Partial<StoredChat> = {}) => {
      const chat: StoredChat = {
        id: chatId,
        title,
        created,
        updated: new Date().toISOString(),
        message_count: msgs.length,
        messages: msgs,
        ...patch,
      };
      try {
        await call("save_chat", { chat });
        setChats(await call<ChatMetadata[]>("list_chats"));
      } catch (e) {
        toast("error", `Conversazione non salvata: ${String(e)}`);
      }
    },
    [chatId, title, created, toast],
  );

  /* ------------------------------ conversazione ----------------------------- */

  const aiConfig = useCallback(
    () => ({
      use_premium_ai: preferences.use_premium_ai,
      ollama_url: preferences.ollama_url,
      ollama_model: preferences.ollama_model,
      user_preferences: config.user_preferences,
      web_search: preferences.web_search,
      file_tools: preferences.file_tools,
      memory: preferences.memory,
    }),
    [preferences, config.user_preferences],
  );

  /**
   * Un turno completo: manda `text`, riceve la risposta in streaming e salva.
   * `history` e' la conversazione *prima* di questo messaggio, cosi' la stessa
   * funzione serve sia per un messaggio nuovo sia per "Rigenera".
   */
  const run = useCallback(
    async (text: string, history: ChatMessage[], attachments: Attachment[] = []) => {
      const turns: ChatTurn[] = history
        .filter((m) => m.type === "text")
        .map((m) => ({ role: m.role, content: promptContent(m) }));

      const userMessage = message("user", text);
      if (attachments.length > 0) userMessage.attachments = attachments;
      const withUser = [...history, userMessage];
      setMessages(withUser);
      setStreamingText(null);
      setStatusText(null);
      setWaiting(true);

      const provisional =
        title === UNTITLED ? titleFrom(text || attachments.map((a) => a.name).join(", ")) : title;
      if (provisional !== title) setTitle(provisional);

      // Se nel frattempo l'utente cambia conversazione, questa risposta non
      // deve piu' finire a schermo: appartiene a un'altra chat.
      const startedIn = chatId;
      const stillHere = () => currentChat.current === startedIn;

      try {
        const reply = await call<TalkReply>("talk", {
          chatId,
          message: promptContent(userMessage),
          history: turns,
          images: promptImages(userMessage),
          config: aiConfig(),
        });
        if (!stillHere()) return;

        const answer = reply.text.trim();
        const aiMessage = message("ai", answer);
        const sources: WebSource[] = reply.sources ?? [];
        if (sources.length > 0) aiMessage.sources = sources;
        const used: ToolUse[] = reply.tools ?? [];
        if (used.length > 0) aiMessage.tools = used;
        const finalMessages = answer.length > 0 ? [...withUser, aiMessage] : withUser;
        // Il testo in streaming sparisce nello stesso momento in cui compare il
        // messaggio finito, altrimenti si vede due volte.
        setStreamingText(null);
        setStatusText(null);
        setMessages(finalMessages);
        if (reply.error) toast("error", reply.error);
        if (answer.length === 0 && !reply.stopped) {
          toast("error", "Il modello non ha risposto nulla. Riprova o cambia modello.");
        }

        void persist(finalMessages, { title: provisional });

        // Il titolo vero arriva dopo il primo scambio, quando c'e' abbastanza
        // contesto: se il modello non collabora resta quello provvisorio.
        if (history.length === 0 && answer.length > 0) {
          const suggested = await call<string | null>("suggest_title", {
            question: text,
            answer,
            config: aiConfig(),
          }).catch(() => null);
          if (suggested && stillHere()) {
            setTitle(suggested);
            void persist(finalMessages, { title: suggested });
          }
        }
      } catch (e) {
        if (!stillHere()) return;
        const failed = [...withUser, message("ai", String(e), "notification")];
        setStreamingText(null);
        setMessages(failed);
        void persist(failed, { title: provisional });
      } finally {
        if (stillHere()) {
          setWaiting(false);
          setStreamingText(null);
          setStatusText(null);
        }
      }
    },
    [chatId, title, aiConfig, persist, toast],
  );

  /** Manda un messaggio: i file scelti col "+" vengono letti prima di partire. */
  const send = useCallback(
    (text: string, attachmentPaths: string[] = []) => {
      void (async () => {
        let attachments: Attachment[] = [];
        if (attachmentPaths.length > 0) {
          try {
            attachments = await call<Attachment[]>("read_attachments", { paths: attachmentPaths });
          } catch (e) {
            toast("error", `Allegati non letti: ${String(e)}`);
            return;
          }
          for (const failed of attachments.filter((a) => a.error !== null)) {
            toast("error", `${failed.name}: ${failed.error}`);
          }
          const images = attachments.filter((a) => a.image).length;
          if (images > 0 && !preferences.use_premium_ai && !seesImages(preferences.ollama_model)) {
            toast(
              "info",
              `${preferences.ollama_model} non guarda le immagini: scegli un modello con la vista (llava, llama3.2-vision) o passa a Claude.`,
            );
          }
        }
        await run(text, messages, attachments);
      })();
    },
    [run, messages, toast, preferences.use_premium_ai, preferences.ollama_model],
  );

  const stop = useCallback(() => {
    void call("stop_talk").catch(() => undefined);
  }, []);

  /** Rigenera: si toglie l'ultima risposta e si rimanda l'ultima domanda. */
  const regenerate = useCallback(() => {
    const lastUser = [...messages].reverse().find((m) => m.role === "user");
    if (!lastUser) return;
    const index = messages.indexOf(lastUser);
    void run(lastUser.content, messages.slice(0, index), lastUser.attachments ?? []);
  }, [messages, run]);

  /**
   * Riscrive un messaggio gia' mandato e riparte da li'. Quello che veniva
   * dopo viene lasciato indietro: se serve tenerlo, si duplica prima la
   * conversazione dal menu della barra laterale.
   */
  const editMessage = useCallback(
    (messageId: string, text: string) => {
      const index = messages.findIndex((m) => m.id === messageId);
      if (index < 0) return;
      void run(text, messages.slice(0, index), messages[index].attachments ?? []);
    },
    [messages, run],
  );

  /* ------------------------------ gestione chat ----------------------------- */

  const newChat = useCallback(() => {
    stop();
    setChatId(newChatId());
    setCreated(new Date().toISOString());
    setTitle(UNTITLED);
    setMessages([]);
    setStreamingText(null);
    setStatusText(null);
    setWaiting(false);
  }, [stop]);

  const selectChat = useCallback(
    async (id: string) => {
      if (id === chatId) return;
      stop();
      try {
        const chat = await call<StoredChat>("load_chat", { chatId: id });
        setChatId(id);
        setCreated(chat.created);
        setTitle(chat.title);
        setMessages(chat.messages);
        setStreamingText(null);
        setStatusText(null);
        setWaiting(false);
      } catch (e) {
        toast("error", `Conversazione non caricata: ${String(e)}`);
      }
    },
    [chatId, stop, toast],
  );

  /** Cerca dentro le conversazioni: la lista la disegna la barra laterale. */
  const searchChats = useCallback(
    async (query: string) => {
      try {
        return await call<ChatMatch[]>("search_chats", { query });
      } catch (e) {
        toast("error", `Ricerca non riuscita: ${String(e)}`);
        return [];
      }
    },
    [toast],
  );

  const renameChat = useCallback(
    async (id: string, newTitle: string) => {
      try {
        const meta = await call<ChatMetadata>("rename_chat", { chatId: id, title: newTitle });
        setChats(await call<ChatMetadata[]>("list_chats"));
        // Se e' la conversazione aperta, il titolo in memoria deve seguire, o
        // il primo salvataggio successivo riscriverebbe quello vecchio.
        if (id === chatId) setTitle(meta.title);
      } catch (e) {
        toast("error", `Titolo non cambiato: ${String(e)}`);
      }
    },
    [chatId, toast],
  );

  const duplicateChat = useCallback(
    async (id: string) => {
      try {
        const meta = await call<ChatMetadata>("duplicate_chat", { chatId: id });
        setChats(await call<ChatMetadata[]>("list_chats"));
        // Ci si sposta nella copia: e' li' che si voleva cambiare strada.
        await selectChat(meta.id);
      } catch (e) {
        toast("error", `Copia non riuscita: ${String(e)}`);
      }
    },
    [selectChat, toast],
  );

  const exportChat = useCallback(
    async (id: string, format: ExportFormat) => {
      try {
        const content = await call<string>("export_chat", { chatId: id, format });
        const name = `${fileNameFor(chats.find((c) => c.id === id)?.title)}.${
          format === "json" ? "json" : "md"
        }`;

        if (isDesktop) {
          const path = await save({
            defaultPath: name,
            filters: [
              format === "json"
                ? { name: "JSON", extensions: ["json"] }
                : { name: "Markdown", extensions: ["md"] },
            ],
          });
          if (!path) return;
          await call("save_export", { path, content });
          toast("success", `Conversazione esportata in ${path}`);
          return;
        }

        // Da browser il file non si scrive sul PC: si scarica.
        const type = format === "json" ? "application/json" : "text/markdown";
        const url = URL.createObjectURL(new Blob([content], { type }));
        const link = document.createElement("a");
        link.href = url;
        link.download = name;
        link.click();
        URL.revokeObjectURL(url);
      } catch (e) {
        toast("error", `Esportazione non riuscita: ${String(e)}`);
      }
    },
    [chats, toast],
  );

  const removeChat = useCallback(
    async (id: string) => {
      try {
        await call("delete_chat", { chatId: id });
        setChats(await call<ChatMetadata[]>("list_chats"));
        if (id === chatId) newChat();
      } catch (e) {
        toast("error", String(e));
      }
    },
    [chatId, newChat, toast],
  );

  async function saveSettings(prefs: Preferences, cfg: Config) {
    setPreferences(prefs);
    setConfig(cfg);
    setSettingsOpen(false);
    try {
      await Promise.all([
        call("set_preferences", { preferences: prefs }),
        call("set_config", { config: cfg }),
      ]);
    } catch (e) {
      toast("error", String(e));
    }
  }

  // Ctrl+N apre una conversazione nuova, come nel resto del sistema.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.ctrlKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        newChat();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [newChat]);

  const online = preferences.use_premium_ai ? (status?.claude ?? false) : (status?.ollama ?? false);
  // Etichetta corta per il riquadro del composer: solo il nome del modello.
  const engineLabel = preferences.use_premium_ai ? "claude-api" : preferences.ollama_model;

  return (
    <div className={`layout ${sidebarOpen ? "" : "no-sidebar"}`}>
      <Sidebar
        chats={chats}
        currentChatId={chatId}
        open={sidebarOpen}
        onNewChat={newChat}
        onSelectChat={(id) => void selectChat(id)}
        onDeleteChat={(id) => void removeChat(id)}
        onRenameChat={(id, newTitle) => void renameChat(id, newTitle)}
        onDuplicateChat={(id) => void duplicateChat(id)}
        onExportChat={(id, format) => void exportChat(id, format)}
        onSearch={searchChats}
        onOpenSettings={() => setSettingsOpen(true)}
      />

      <ChatView
        messages={messages}
        streamingText={streamingText}
        waiting={waiting}
        statusText={statusText}
        online={online}
        engineLabel={engineLabel}
        sidebarOpen={sidebarOpen}
        onToggleSidebar={() => setSidebarOpen((v) => !v)}
        onSend={send}
        onStop={stop}
        onRegenerate={regenerate}
        onEdit={editMessage}
        onError={(text) => toast("error", text)}
      />

      {settingsOpen && (
        <SettingsModal
          preferences={preferences}
          config={config}
          status={status}
          onSave={(p, c) => void saveSettings(p, c)}
          onClose={() => setSettingsOpen(false)}
          onNotice={toast}
        />
      )}

      <NotificationToast toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}
