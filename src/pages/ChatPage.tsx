import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

import ChatInterface from "../components/ChatInterface";
import NotificationToast from "../components/NotificationToast";
import Sidebar from "../components/Sidebar";
import SettingsModal from "../components/SettingsModal";

import {
  DEFAULT_CONFIG,
  DEFAULT_PREFERENCES,
  type AiStatus,
  type ApplyProgress,
  type ApplyResult,
  type ChatMessage,
  type ChatMetadata,
  type ChatReply,
  type ChatTurn,
  type Config,
  type OrganizationProposal,
  type PlanPreview,
  type Preferences,
  type ScanProgress,
  type StoredChat,
  type Toast,
} from "../types";

const GREETING =
  "Ciao, sono Logica. Dimmi quale cartella vuoi sistemare (per esempio “organizza Downloads”) e ci penso io: prima ti mostro cosa farei, poi sposto solo se sei d'accordo.";

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

function message(
  role: ChatMessage["role"],
  content: string,
  type: ChatMessage["type"] = "text",
  proposalData?: OrganizationProposal,
): ChatMessage {
  return { id: newId("m"), role, content, timestamp: new Date().toISOString(), type, proposalData };
}

export default function ChatPage() {
  const [preferences, setPreferences] = useState<Preferences>(DEFAULT_PREFERENCES);
  const [config, setConfig] = useState<Config>(DEFAULT_CONFIG);
  const [status, setStatus] = useState<AiStatus | null>(null);

  const [chats, setChats] = useState<ChatMetadata[]>([]);
  const [chatId, setChatId] = useState<string>(newChatId);
  const [created] = useState<string>(() => new Date().toISOString());
  const [title, setTitle] = useState("Nuova chat");
  const [messages, setMessages] = useState<ChatMessage[]>([message("ai", GREETING)]);

  const [proposal, setProposal] = useState<OrganizationProposal | null>(null);
  const [activeProposalId, setActiveProposalId] = useState<string | null>(null);
  const [preview, setPreview] = useState<PlanPreview | null>(null);

  const [folders, setFolders] = useState<string[]>([]);
  const [fileCount, setFileCount] = useState(0);
  const [filesMoved, setFilesMoved] = useState(0);
  const [foldersCreated, setFoldersCreated] = useState(0);
  const [appliedOn, setAppliedOn] = useState<string | null>(null);
  const [undoAvailable, setUndoAvailable] = useState(false);

  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progressLabel, setProgressLabel] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);

  // Le chat vecchie perdono la scansione (vive nel processo Rust): al primo
  // messaggio la rifacciamo, cosi' la conversazione riprende da dove stava.
  const needsRescan = useRef(false);

  const toast = useCallback((kind: Toast["kind"], text: string) => {
    setToasts((prev) => [...prev, { id: newId("t"), kind, message: text }]);
  }, []);

  const dismissToast = useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const push = useCallback((msg: ChatMessage) => setMessages((prev) => [...prev, msg]), []);

  /* ------------------------------ avvio e stato ----------------------------- */

  useEffect(() => {
    void (async () => {
      try {
        const [prefs, cfg, list] = await Promise.all([
          invoke<Preferences>("get_preferences"),
          invoke<Config>("get_config"),
          invoke<ChatMetadata[]>("list_chats"),
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

  // Stato dei motori AI: all'avvio e a ogni cambio di URL/modello.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const s = await invoke<AiStatus>("check_ai_status", { ollamaUrl: preferences.ollama_url });
      if (!cancelled) setStatus(s);
    })().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [preferences.ollama_url, preferences.use_premium_ai]);

  // Progresso emesso dal backend durante scansione e spostamento.
  useEffect(() => {
    const unlisteners = [
      listen<ScanProgress>("scan-progress", (e) =>
        setProgressLabel(`${e.payload.scanned.toLocaleString("it-IT")} file scansionati`),
      ),
      listen<ApplyProgress>("apply-progress", (e) =>
        setProgressLabel(`sposto ${e.payload.done.toLocaleString("it-IT")} / ${e.payload.total.toLocaleString("it-IT")}`),
      ),
    ];
    return () => {
      unlisteners.forEach((u) => void u.then((f) => f()));
    };
  }, []);

  /* -------------------------------- persistenza ------------------------------- */

  const persist = useCallback(
    async (msgs: ChatMessage[], patch: Partial<StoredChat> = {}) => {
      const chat: StoredChat = {
        id: chatId,
        title,
        created,
        updated: new Date().toISOString(),
        folders,
        message_count: msgs.length,
        files_moved: filesMoved,
        folders_created: foldersCreated,
        applied_on: appliedOn,
        messages: msgs,
        final_proposal: proposal ?? undefined,
        ...patch,
      };
      try {
        await invoke("save_chat", { chat });
        setChats(await invoke<ChatMetadata[]>("list_chats"));
      } catch (e) {
        toast("error", `Chat non salvata: ${String(e)}`);
      }
    },
    [chatId, title, created, folders, filesMoved, foldersCreated, appliedOn, proposal, toast],
  );

  /* --------------------------------- azioni --------------------------------- */

  const applyReply = useCallback(
    (reply: ChatReply) => {
      const added: ChatMessage[] = [];
      if (reply.response.trim()) added.push(message("ai", reply.response));
      if (reply.proposal) {
        const block = message("ai", "Schema proposto", "proposal_block", reply.proposal);
        added.push(block);
        setProposal(reply.proposal);
        setActiveProposalId(block.id);
        setPreview(reply.preview);
      }

      setFolders(reply.folders);
      setFileCount(reply.file_count);

      if (reply.intent === "applied" && reply.apply_result) {
        setProposal(null);
        setActiveProposalId(null);
        setPreview(null);
        setFilesMoved((n) => n + reply.apply_result!.files_moved);
        setFoldersCreated((n) => n + reply.apply_result!.folders_created);
        setAppliedOn(new Date().toISOString());
        setUndoAvailable(true);
        toast("success", `${reply.apply_result.files_moved} file organizzati.`);
      }
      if (reply.intent === "cancelled") {
        setProposal(null);
        setActiveProposalId(null);
        setPreview(null);
      }
      if (reply.intent === "undone") {
        setUndoAvailable(false);
        toast("info", "Operazione annullata.");
      }
      if (reply.error) toast("error", reply.error);

      setMessages((prev) => [...prev, ...added]);
      return added;
    },
    [toast],
  );

  const restoreScans = useCallback(async () => {
    if (!needsRescan.current || folders.length === 0) return;
    needsRescan.current = false;
    setProgressLabel("riapro le cartelle della chat");
    try {
      const reply = await invoke<ChatReply>("scan_folders", { chatId, folders });
      setFolders(reply.folders);
      setFileCount(reply.file_count);
    } catch (e) {
      toast("error", `Non sono riuscita a rileggere le cartelle: ${String(e)}`);
    } finally {
      setProgressLabel("");
    }
  }, [chatId, folders, toast]);

  const send = useCallback(
    async (text: string) => {
      const userMessage = message("user", text);
      const history: ChatTurn[] = messages
        .filter((m) => m.type !== "proposal_block")
        .map((m) => ({ role: m.role, content: m.content }));
      setMessages((prev) => [...prev, userMessage]);
      setLoading(true);

      // La prima frase dell'utente da' il titolo alla conversazione.
      const nextTitle = title === "Nuova chat" ? text.slice(0, 48) : title;
      if (nextTitle !== title) setTitle(nextTitle);

      try {
        await restoreScans();
        const reply = await invoke<ChatReply>("chat_message", {
          message: text,
          chatId,
          history,
          config: {
            use_premium_ai: preferences.use_premium_ai,
            ollama_url: preferences.ollama_url,
            ollama_model: preferences.ollama_model,
            user_preferences: config.user_preferences,
          },
          proposal,
        });
        const added = applyReply(reply);
        void persist([...messages, userMessage, ...added], { title: nextTitle });
      } catch (e) {
        push(message("ai", `Qualcosa e' andato storto: ${String(e)}`, "notification"));
        toast("error", String(e));
      } finally {
        setLoading(false);
        setProgressLabel("");
      }
    },
    [messages, title, chatId, preferences, config, proposal, applyReply, persist, push, restoreScans, toast],
  );

  const approve = useCallback(async () => {
    if (!proposal) return;
    setBusy(true);
    try {
      const result = await invoke<ApplyResult>("apply_proposal", { chatId, proposal });
      const note = message(
        "ai",
        `✅ Fatto! ${result.files_moved} file spostati in ${result.folders_created} cartelle.` +
          (result.errors.length > 0 ? `\nAlcuni problemi: ${result.errors.slice(0, 3).join("; ")}` : "") +
          "\nSe non ti convince, scrivi “annulla” e rimetto tutto com'era.",
        "notification",
      );
      setProposal(null);
      setActiveProposalId(null);
      setPreview(null);
      setFilesMoved((n) => n + result.files_moved);
      setFoldersCreated((n) => n + result.folders_created);
      const now = new Date().toISOString();
      setAppliedOn(now);
      setUndoAvailable(true);
      push(note);
      toast("success", `${result.files_moved} file organizzati.`);
      void persist([...messages, note], { files_moved: filesMoved + result.files_moved, applied_on: now });
    } catch (e) {
      push(message("ai", `Non ho potuto applicare lo schema: ${String(e)}`, "notification"));
      toast("error", String(e));
    } finally {
      setBusy(false);
      setProgressLabel("");
    }
  }, [proposal, chatId, messages, filesMoved, push, persist, toast]);

  const undo = useCallback(async () => {
    setBusy(true);
    try {
      const msg = await invoke<string>("undo_last_action", { chatId });
      const note = message("ai", `↩ ${msg}`, "notification");
      setUndoAvailable(false);
      push(note);
      toast("info", msg);
      void persist([...messages, note]);
    } catch (e) {
      toast("error", String(e));
    } finally {
      setBusy(false);
    }
  }, [chatId, messages, push, persist, toast]);

  // Ctrl+Z annulla l'ultima operazione, come in qualsiasi altra app.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.ctrlKey && e.key.toLowerCase() === "z" && undoAvailable && !busy) {
        e.preventDefault();
        void undo();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undoAvailable, busy, undo]);

  /** Ricalcola l'anteprima quando l'utente modifica lo schema a mano. */
  const changeProposal = useCallback(
    async (updated: OrganizationProposal) => {
      setProposal(updated);
      setMessages((prev) =>
        prev.map((m) => (m.id === activeProposalId ? { ...m, proposalData: updated } : m)),
      );
      try {
        setPreview(await invoke<PlanPreview>("preview_proposal", { chatId, proposal: updated }));
      } catch (e) {
        toast("error", String(e));
      }
    },
    [activeProposalId, chatId, toast],
  );

  const cancelProposal = useCallback(() => {
    setProposal(null);
    setActiveProposalId(null);
    setPreview(null);
    push(message("ai", "Ok, lascio perdere questo schema. Dimmi come preferisci procedere.", "notification"));
  }, [push]);

  const addFolder = useCallback(async () => {
    const picked = await open({ directory: true, multiple: true, title: "Cartelle da organizzare" });
    if (!picked) return;
    const list = Array.isArray(picked) ? picked : [picked];
    setLoading(true);
    try {
      const reply = await invoke<ChatReply>("scan_folders", { chatId, folders: list });
      applyReply(reply);
    } catch (e) {
      toast("error", String(e));
    } finally {
      setLoading(false);
      setProgressLabel("");
    }
  }, [chatId, applyReply, toast]);

  /* ------------------------------ gestione chat ----------------------------- */

  function resetTo(id: string) {
    setChatId(id);
    setProposal(null);
    setActiveProposalId(null);
    setPreview(null);
    setFolders([]);
    setFileCount(0);
    setFilesMoved(0);
    setFoldersCreated(0);
    setAppliedOn(null);
    setUndoAvailable(false);
    setProgressLabel("");
  }

  function newChat() {
    resetTo(newChatId());
    setTitle("Nuova chat");
    setMessages([message("ai", GREETING)]);
    needsRescan.current = false;
  }

  async function selectChat(id: string) {
    if (id === chatId) return;
    try {
      const chat = await invoke<StoredChat>("load_chat", { chatId: id });
      resetTo(id);
      setTitle(chat.title);
      setMessages(chat.messages);
      setFolders(chat.folders);
      setFilesMoved(chat.files_moved);
      setFoldersCreated(chat.folders_created);
      setAppliedOn(chat.applied_on);
      setUndoAvailable(await invoke<boolean>("has_undo", { chatId: id }));
      // La scansione non e' salvata su disco: si rifa' al primo messaggio.
      needsRescan.current = chat.folders.length > 0;
    } catch (e) {
      toast("error", `Chat non caricata: ${String(e)}`);
    }
  }

  async function removeChat(id: string) {
    try {
      await invoke("delete_chat", { chatId: id });
      setChats(await invoke<ChatMetadata[]>("list_chats"));
      if (id === chatId) newChat();
      toast("info", "Chat eliminata.");
    } catch (e) {
      toast("error", String(e));
    }
  }

  async function saveSettings(prefs: Preferences, cfg: Config) {
    setPreferences(prefs);
    setConfig(cfg);
    setSettingsOpen(false);
    try {
      await Promise.all([
        invoke("set_preferences", { preferences: prefs }),
        invoke("set_config", { config: cfg }),
      ]);
      toast("success", "Impostazioni salvate.");
    } catch (e) {
      toast("error", String(e));
    }
  }

  const online = preferences.use_premium_ai ? (status?.claude ?? false) : (status?.ollama ?? false);
  const engineLabel = preferences.use_premium_ai ? "Claude" : `Ollama · ${preferences.ollama_model}`;

  return (
    <div className="layout">
      <ChatInterface
        messages={messages}
        activeProposalId={activeProposalId}
        preview={preview}
        isLoading={loading}
        busy={busy}
        progressLabel={progressLabel}
        online={online}
        engineLabel={engineLabel}
        undoAvailable={undoAvailable}
        folders={folders}
        fileCount={fileCount}
        suggestions={config.monitored_folders}
        onSendMessage={(text) => void send(text)}
        onProposalChange={(p) => void changeProposal(p)}
        onApprove={() => void approve()}
        onCancelProposal={cancelProposal}
        onUndo={() => void undo()}
        onCancelApply={() => void invoke("cancel_apply").catch(() => undefined)}
        onAddFolder={() => void addFolder()}
        onOpenSettings={() => setSettingsOpen(true)}
      />

      <Sidebar
        chats={chats}
        currentChatId={chatId}
        onNewChat={newChat}
        onSelectChat={(id) => void selectChat(id)}
        onDeleteChat={(id) => void removeChat(id)}
        onOpenSettings={() => setSettingsOpen(true)}
      />

      {settingsOpen && (
        <SettingsModal
          preferences={preferences}
          config={config}
          status={status}
          onSave={(p, c) => void saveSettings(p, c)}
          onClose={() => setSettingsOpen(false)}
        />
      )}

      <NotificationToast toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
}
