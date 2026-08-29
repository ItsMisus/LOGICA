import type { ApplyResult } from "../types";

interface Props {
  result: ApplyResult;
  conflicts: number;
  rootPath: string;
  undoing: boolean;
  onOpenFolder: () => void;
  onUndo: () => void;
  onRestart: () => void;
}

export default function ResultPhase({
  result,
  conflicts,
  rootPath,
  undoing,
  onOpenFolder,
  onUndo,
  onRestart,
}: Props) {
  return (
    <div className="card">
      <h1>{result.status === "success" ? "Fatto!" : "Completato con errori"}</h1>

      <ul className="summary">
        <li className="ok">
          <span aria-hidden="true">✅</span> {result.files_moved.toLocaleString("it-IT")} file spostati in{" "}
          {result.folders_created} cartelle
        </li>
        <li className="warn">
          <span aria-hidden="true">⚠️</span> {conflicts} conflitti di nome risolti automaticamente
        </li>
        <li className={result.errors.length > 0 ? "error" : ""}>
          <span aria-hidden="true">{result.errors.length > 0 ? "❌" : "✅"}</span> {result.errors.length}{" "}
          errori
        </li>
      </ul>

      {result.errors.length > 0 && (
        <div className="alert error">
          <ul>
            {result.errors.slice(0, 5).map((e) => (
              <li key={e}>{e}</li>
            ))}
          </ul>
        </div>
      )}

      <p className="mono truncate" title={rootPath}>
        {rootPath}
      </p>

      <div className="row">
        <button className="btn ghost" onClick={onOpenFolder}>
          Apri cartella
        </button>
        <button className="btn ghost" disabled={!result.undo_available || undoing} onClick={onUndo}>
          {undoing ? "Ripristino…" : "Annulla tutto (Ctrl+Z)"}
        </button>
        <button className="btn primary" onClick={onRestart}>
          Nuovo ciclo
        </button>
      </div>
    </div>
  );
}
