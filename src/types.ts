/**
 * Shared types. Field names are snake_case because they mirror the Rust
 * structs 1:1 (serde serializes with the field names as declared).
 */

export type Phase =
  | "SELECT_DIR"
  | "SCANNING"
  | "ANALYZING"
  | "PREVIEW"
  | "APPLYING"
  | "RESULT";

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

/** Nested folder tree: { "Documenti": { "PDF": {} }, "Immagini": {} } */
export interface SchemaNode {
  [folder: string]: SchemaNode;
}

export interface Rule {
  pattern: string;
  destination: string;
}

export interface OrganizationProposal {
  schema: SchemaNode;
  rules: Rule[];
  rationale: string;
  /** Which engine produced it: "ollama" | "claude" | "fallback" */
  source: string;
}

export interface Move {
  source: string;
  destination: string;
  relative_destination: string;
}

export interface Conflict {
  destination: string;
  resolved_as: string;
  reason: string;
}

export interface MappingStatistics {
  total_moves: number;
  new_folders: number;
  unmatched: number;
}

export interface MappingPlan {
  root: string;
  moves: Move[];
  conflicts: Conflict[];
  warnings: string[];
  statistics: MappingStatistics;
}

export interface ApplyResult {
  status: string;
  files_moved: number;
  folders_created: number;
  errors: string[];
  undo_available: boolean;
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

export interface Settings {
  use_premium_ai: boolean;
  ollama_model: string;
  user_preferences: string;
}

export const DEFAULT_SETTINGS: Settings = {
  use_premium_ai: false,
  ollama_model: "mistral",
  user_preferences: "",
};
