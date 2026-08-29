import type { ApplyProgress } from "../types";

interface Props {
  progress: ApplyProgress;
  elapsedMs: number;
  onCancel: () => void;
}

export default function ApplyPhase({ progress, elapsedMs, onCancel }: Props) {
  const pct = progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : 0;
  // Stima lineare: tempo medio per file * file rimanenti.
  const remainingMs =
    progress.done > 0 ? ((elapsedMs / progress.done) * (progress.total - progress.done)) : 0;

  return (
    <div className="card">
      <h1>Applicazione in corso…</h1>
      <div className="bar" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
        <div style={{ width: `${pct}%` }} />
      </div>
      <div className="row space-between">
        <span className="subtitle">
          {progress.done.toLocaleString("it-IT")} / {progress.total.toLocaleString("it-IT")} file
        </span>
        <span className="subtitle">{pct}%</span>
      </div>
      <div className="mono truncate" title={progress.current}>
        {progress.current || "…"}
      </div>
      <p className="subtitle">~{Math.ceil(remainingMs / 1000)}s rimanenti</p>
      <button className="btn ghost" onClick={onCancel}>
        Annulla e ripristina
      </button>
    </div>
  );
}
