/**
 * Tipi condivisi col backend. I nomi dei campi sono snake_case perche'
 * rispecchiano 1:1 le struct Rust (serde serializza i campi come dichiarati).
 */

export interface FileInfo {
  name: string;
  path: string;
  extension: string;
  size: number;
  modified: string;
  relative_path: string;
  depth: number;
  content_preview: string | null;
}

/** Albero di cartelle annidate: { "Documenti": { "PDF": {} }, "Immagini": {} } */
export interface SchemaNode {
  [folder: string]: SchemaNode;
}

export interface Rule {
  pattern: string;
  destination: string;
}

/** Criterio di destinazione: per tipo (usa le regole) o sui metadati. */
export type Grouping = "extension" | "year" | "month" | "source_folder";

export interface OrganizationProposal {
  schema: SchemaNode;
  rules: Rule[];
  rationale: string;
  /** Chi l'ha prodotta: "ollama" | "claude" | "fallback" | "logica" */
  source: string;
  grouping: Grouping;
}

export interface Move {
  source: string;
  destination: string;
  relative_destination: string;
}

/** Riassunto dei piani di spostamento, mostrato nel blocco proposta. */
export interface PlanPreview {
  roots: string[];
  total_moves: number;
  new_folders: number;
  conflicts: number;
  unmatched: number;
  warnings: string[];
  sample: Move[];
}

export interface ApplyResult {
  status: string;
  files_moved: number;
  folders_created: number;
  errors: string[];
  undo_available: boolean;
}

/* ---------------------------------- chat ---------------------------------- */

export type ChatRole = "user" | "ai";
export type ChatMessageType = "text" | "proposal_block" | "notification";

export interface ChatMessage {
  id: string;
  role: ChatRole;
  content: string;
  timestamp: string;
  type: ChatMessageType;
  proposalData?: OrganizationProposal;
}

export interface ChatMetadata {
  id: string;
  title: string;
  created: string;
  updated: string;
  folders: string[];
  message_count: number;
  files_moved: number;
  folders_created: number;
  applied_on: string | null;
}

export interface StoredChat extends ChatMetadata {
  messages: ChatMessage[];
  final_proposal?: OrganizationProposal;
}

export interface ChatTurn {
  role: ChatRole;
  content: string;
}

/** Cosa ha fatto LOGICA in risposta all'ultimo messaggio. */
export type ChatIntent =
  | "chat"
  | "scanned"
  | "proposal"
  | "applied"
  | "undone"
  | "cancelled"
  | "error";

export interface ChatReply {
  response: string;
  intent: ChatIntent;
  proposal: OrganizationProposal | null;
  preview: PlanPreview | null;
  apply_result: ApplyResult | null;
  folders: string[];
  file_count: number;
  error: string | null;
}

/* ------------------------------ configurazione ----------------------------- */

export interface Preferences {
  theme: "dark" | "light";
  use_premium_ai: boolean;
  ollama_url: string;
  ollama_model: string;
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
}

export interface AiStatus {
  ollama: boolean;
  claude: boolean;
  models: string[];
}

export interface ScanProgress {
  scanned: number;
  current: string;
}

export interface ApplyProgress {
  done: number;
  total: number;
  current: string;
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
  ollama_model: "mistral",
};

export const DEFAULT_CONFIG: Config = {
  monitored_folders: [],
  profile: "custom",
  user_preferences: "",
};

/** Profili pronti: testo di preferenze passato all'AI. */
export const PROFILES: Record<string, string> = {
  developer:
    "Sono uno sviluppatore: tieni insieme codice, script e configurazioni; separa i progetti dagli archivi.",
  media:
    "Uso il PC soprattutto per foto e video: privilegia cartelle per tipo di media e per data.",
  office:
    "Lavoro con documenti d'ufficio: separa fatture, contratti, presentazioni e fogli di calcolo.",
  custom: "",
};
