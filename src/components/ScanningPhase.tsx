import type { ScanProgress } from "../types";

interface Props {
  progress: ScanProgress;
  elapsedMs: number;
  onCancel: () => void;
}

export default function ScanningPhase({ progress, elapsedMs, onCancel }: Props) {
  const perSec = elapsedMs > 0 ? Math.round((progress.scanned / elapsedMs) * 1000) : 0;

  return (
    <div className="card">
      <h1>Scansione in corso…</h1>
      <div className="big-number">{progress.scanned.toLocaleString("it-IT")}</div>
      <p className="subtitle">file trovati</p>
      <div className="mono truncate" title={progress.current}>
        {progress.current || "…"}
      </div>
      <p className="subtitle">
        {(elapsedMs / 1000).toFixed(1)}s trascorsi · {perSec.toLocaleString("it-IT")} file/s
      </p>
      <div className="bar indeterminate">
        <div />
      </div>
      <button className="btn ghost" onClick={onCancel}>
        Annulla
      </button>
    </div>
  );
}
