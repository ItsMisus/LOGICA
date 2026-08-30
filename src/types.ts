/**
 * Tipi condivisi col backend. I nomi dei campi sono snake_case perche'
 * rispecchiano 1:1 le struct Rust (serde serializza i campi come dichiarati).
 */

/* ---------------------------------- chat ---------------------------------- */

/** Immagine allegata, gia' codificata dal backend per il modello. */
export interface AttachmentImage {
  /** Tipo MIME: `image/png`, `image/jpeg`, `image/gif`, `image/webp`. */
  media_type: string;
  /** Contenuto in base64, senza prefisso `data:`. */
  data: string;
}

/** Che cosa il backend e' riuscito a leggere dentro l'allegato. */
export type AttachmentKind = "text" | "pdf" | "docx" | "image";

/** File allegato a un messaggio: il contenuto lo estrae il backend. */
export interface Attachment {
  name: string;
  path: string;
  size: number;
  kind: AttachmentKind;
  /** Contenuto testuale gia' tagliato; null per le immagini e per i file
   *  che non si sono potuti leggere. */
  text: string | null;
  /** Presente solo per le immagini: la guarda un modello che vede. */
  image?: AttachmentImage;
  truncated: boolean;
  /** Perche' il file non e' stato letto, pronto da mostrare. */
  error: string | null;
}

/** Una pagina letta su internet, mostrata come fonte sotto la risposta. */
export interface WebSource {
  title: string;
  url: string;
  snippet: string;
}

/** Uno strumento che l'assistente ha usato sul disco per rispondere. */
export interface ToolUse {
  /** `list_dir` | `read_file` | `find_files` | `search_in_files` */
  name: string;
  /** Il percorso guardato o il testo cercato, gia' pronto da mostrare. */
  detail: string;
}

export type ChatRole = "user" | "ai";
export type ChatMessageType = "text" | "notification";

export interface ChatMessage {
  id: string;
  role: ChatRole;
  content: string;
  timestamp: string;
  type: ChatMessageType;
  attachments?: Attachment[];
  /** Pagine consultate su internet per scrivere questa risposta. */
  sources?: WebSource[];
  /** Cartelle e file guardati sul disco per scrivere questa risposta. */
  tools?: ToolUse[];
}

export interface ChatMetadata {
  id: string;
  title: string;
  created: string;
  updated: string;
  message_count: number;
  /** Motore scelto per questa conversazione. Assente = quello delle
   *  impostazioni. */
  engine?: ChatEngine;
}

/** Motore di una singola conversazione: `claude`, oppure `ollama:<modello>`.
 *  Il formato è una stringa sola perché è quello che finisce nel file della
 *  chat, e due campi separati si potrebbero salvare incoerenti fra loro. */
export type ChatEngine = string;

/** Il motore delle impostazioni, quando la chat non ne sceglie uno suo. */
export const DEFAULT_ENGINE = "";

/** Scioglie `engine` nei due campi che il backend si aspetta. Un valore che
 *  non si riconosce vale come "usa le impostazioni": una chat scritta da una
 *  versione futura non deve rompersi qui. */
export function engineOverride(
  engine: ChatEngine | undefined,
  fallback: { use_premium_ai: boolean; ollama_model: string },
): { use_premium_ai: boolean; ollama_model: string } {
  if (engine === "claude") return { use_premium_ai: true, ollama_model: fallback.ollama_model };
  if (engine?.startsWith("ollama:")) {
    return { use_premium_ai: false, ollama_model: engine.slice("ollama:".length) };
  }
  return fallback;
}

/** Come si chiama un motore, per l'etichetta nella barra del composer. */
export function engineLabelOf(
  engine: ChatEngine | undefined,
  fallback: { use_premium_ai: boolean; ollama_model: string },
): string {
  const resolved = engineOverride(engine, fallback);
  return resolved.use_premium_ai ? "claude-api" : resolved.ollama_model;
}

export interface StoredChat extends ChatMetadata {
  messages: ChatMessage[];
}

/** Conversazione trovata dalla ricerca, col pezzo di testo che l'ha fatta
 *  comparire. */
export interface ChatMatch extends ChatMetadata {
  /** Vuoto quando la corrispondenza sta solo nel titolo. */
  snippet: string;
  /** In quanti messaggi compare il testo cercato. */
  hits: number;
}

/** Formati in cui una conversazione si porta fuori dall'app. */
export type ExportFormat = "markdown" | "json";

export interface ChatTurn {
  role: ChatRole;
  content: string;
}

/** Esito di un turno di conversazione (comando `talk`). */
export interface TalkReply {
  text: string;
  /** L'utente ha premuto "Interrompi": il testo e' parziale ma si tiene. */
  stopped: boolean;
  error: string | null;
  sources: WebSource[];
  tools: ToolUse[];
}

/** Pezzo di risposta in arrivo durante lo streaming (evento `assistant-delta`). */
export interface AssistantDelta {
  chat_id: string;
  text: string;
}

/** Cosa sta facendo l'app prima di scrivere (evento `assistant-status`):
 *  la ricerca in corso, la pagina che sta leggendo. */
export interface AssistantStatus {
  chat_id: string;
  text: string;
}

/** Una cosa che l'assistente ha imparato sull'utente e si porta dietro fra
 *  una chat e l'altra. */
export interface Fact {
  id: string;
  text: string;
  created: string;
  /** La conversazione da cui viene. */
  chat_id: string;
}

/** Stato dell'indice dei documenti. */
export interface IndexStatus {
  files: number;
  pieces: number;
  /** Fine dell'ultima indicizzazione, RFC 3339. Vuoto se non è mai stata fatta. */
  built: string;
  model: string;
}

/** A che punto è l'indicizzazione (evento `index-progress`). */
export interface IndexProgress {
  done: number;
  total: number;
  /** Il file in corso; vuoto quando ha finito. */
  file: string;
}

/* ------------------------------ configurazione ----------------------------- */

export interface Preferences {
  theme: "dark" | "light";
  use_premium_ai: boolean;
  ollama_url: string;
  ollama_model: string;
  /** L'assistente puo' cercare su internet quando la risposta lo richiede. */
  web_search: boolean;
  /** L'assistente puo' elencare, leggere e cercare dentro le cartelle
   *  autorizzate. Spento finche' non lo si accende. */
  file_tools: boolean;
  /** L'assistente ricorda fra una chat e l'altra quello che ha imparato. */
  memory: boolean;
  /** L'assistente consulta l'indice dei documenti prima di rispondere. */
  documents: boolean;
}

export interface Config {
  monitored_folders: string[];
  /** "developer" | "media" | "office" | "custom" */
  profile: string;
  user_preferences: string;
}

/** Config del motore AI inviata a ogni messaggio. */
export interface AiConfig {
  use_premium_ai: boolean;
  ollama_url: string;
  ollama_model: string;
  user_preferences: string;
  web_search: boolean;
  file_tools: boolean;
  memory: boolean;
  documents: boolean;
}

export interface AiStatus {
  ollama: boolean;
  claude: boolean;
  models: string[];
}

export interface Toast {
  id: string;
  kind: "success" | "error" | "info";
  message: string;
}

export const DEFAULT_PREFERENCES: Preferences = {
  theme: "dark",
  use_premium_ai: false,
  ollama_url: "http://localhost:11434",
  ollama_model: "llama3.1:8b",
  web_search: true,
  file_tools: false,
  memory: true,
  documents: false,
};

export const DEFAULT_CONFIG: Config = {
  monitored_folders: [],
  profile: "custom",
  user_preferences: "",
};

/** Modello con cui si calcolano i vettori dei documenti. Deve restare
 *  allineato a `EMBED_MODEL` in `src-tauri/src/rag.rs`. */
export const EMBED_MODEL = "nomic-embed-text";

/** Quanti fatti tiene la memoria. Deve restare allineata a `MAX_FACTS` in
 *  `src-tauri/src/memory.rs`. */
export const MAX_FACTS = 60;

/** Lunghezza minima della password per l'accesso da browser. Deve restare
 *  allineata a `MIN_PASSWORD_LEN` in `src-tauri/src/server/auth.rs`. */
export const MIN_PASSWORD_LEN = 12;

/** Stato del server web, come lo riporta il backend. */
export interface ServerStatus {
  running: boolean;
  /** Indirizzo da aprire sull'altro PC, quando e' acceso. */
  url: string | null;
  /** Impronta SHA-256 del certificato, da confrontare con quella del browser. */
  fingerprint: string | null;
  port: number;
  exposeToLan: boolean;
  autostart: boolean;
  hasPassword: boolean;
  /** Cartelle entro cui il browser puo' lavorare. Vuota = niente. */
  allowedFolders: string[];
}
