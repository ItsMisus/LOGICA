interface Props {
  fileCount: number;
  engine: string;
  elapsedMs: number;
}

export default function AnalysisPhase({ fileCount, engine, elapsedMs }: Props) {
  return (
    <div className="card center">
      <div className="spinner" role="img" aria-label="Analisi in corso" />
      <h1>{engine} sta analizzando…</h1>
      <p className="subtitle">
        {fileCount.toLocaleString("it-IT")} file · {(elapsedMs / 1000).toFixed(0)}s / 30s max
      </p>
      <p className="subtitle">Se l'AI non risponde in tempo viene usato uno schema per estensione.</p>
    </div>
  );
}
