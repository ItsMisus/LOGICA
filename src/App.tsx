import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import StepSelector from "./components/StepSelector";
import ScanningPhase from "./components/ScanningPhase";
import AnalysisPhase from "./components/AnalysisPhase";
import PreviewPhase from "./components/PreviewPhase";
import ApplyPhase from "./components/ApplyPhase";
import ResultPhase from "./components/ResultPhase";

import {
  DEFAULT_SETTINGS,
  type ApplyProgress,
  type ApplyResult,
  type FileInfo,
  type MappingPlan,
  type OrganizationProposal,
  type Phase,
  type ScanProgress,
  type Settings,
} from "./types";

const SCAN_TIMEOUT_MS = 5 * 60 * 1000;
const ANALYZE_TIMEOUT_MS = 40 * 1000; // il backend si ferma a 30s, questo e' solo una rete di sicurezza
const SETTINGS_KEY = "file-organizer-ai:settings";

/** Rifiuta se la promise non si risolve entro `ms`. */
function withTimeout<T>(p: Promise<T>, ms: number, label: string): Promise<T> {
  return Promise.race([
    p,
    new Promise<T>((_, reject) =>
      setTimeout(() => reject(new Error(`${label} ha superato il tempo massimo.`)), ms),
    ),
  ]);
}

function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    return raw ? { ...DEFAULT_SETTINGS, ...JSON.parse(raw) } : DEFAULT_SETTINGS;
  } catch {
    return DEFAULT_SETTINGS;
  }
}

export default function App() {
  const [phase, setPhase] = useState<Phase>("SELECT_DIR");
  const [settings, setSettings] = useState<Settings>(loadSettings);
  const [rootPath, setRootPath] = useState("");
  const [files, setFiles] = useState<FileInfo[]>([]);
  const [proposal, setProposal] = useState<OrganizationProposal | null>(null);
  const [plan, setPlan] = useState<MappingPlan | null>(null);
  const [result, setResult] = useState<ApplyResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [undoing, setUndoing] = useState(false);

  const [scanProgress, setScanProgress] = useState<ScanProgress>({ scanned: 0, current: "" });
  const [applyProgress, setApplyProgress] = useState<ApplyProgress>({ done: 0, total: 0, current: "" });
  const [elapsedMs, setElapsedMs] = useState(0);
  const startedAt = useRef(0);

  useEffect(() => {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
  }, [settings]);

  // Eventi di progresso emessi dal backend Rust.
  useEffect(() => {
    const unlisteners = [
      listen<ScanProgress>("scan-progress", (e) => setScanProgress(e.payload)),
      listen<ApplyProgress>("apply-progress", (e) => setApplyProgress(e.payload)),
    ];
    return () => {
      unlisteners.forEach((u) => void u.then((f) => f()));
    };
  }, []);

  // Cronometro condiviso dalle fasi che mostrano tempi.
  useEffect(() => {
    if (phase !== "SCANNING" && phase !== "ANALYZING" && phase !== "APPLYING") return;
    startedAt.current = Date.now();
    setElapsedMs(0);
    const id = setInterval(() => setElapsedMs(Date.now() - startedAt.current), 200);
    return () => clearInterval(id);
  }, [phase]);

  const failTo = useCallback((phaseToGoBack: Phase, err: unknown) => {
    setError(err instanceof Error ? err.message : String(err));
    setPhase(phaseToGoBack);
  }, []);

  /* --------------------------- SCANNING + ANALYZING -------------------------- */

  async function start(path: string) {
    setError(null);
    setRootPath(path);
    setScanProgress({ scanned: 0, current: "" });
    setPhase("SCANNING");

    let scanned: FileInfo[];
    try {
      scanned = await withTimeout(invoke<FileInfo[]>("scan_directory", { path }), SCAN_TIMEOUT_MS, "La scansione");
    } catch (e) {
      failTo("SELECT_DIR", e);
      return;
    }

    if (scanned.length === 0) {
      failTo("SELECT_DIR", new Error("Nessun file trovato in questa cartella."));
      return;
    }
    setFiles(scanned);
    setPhase("ANALYZING");

    try {
      const p = await withTimeout(
        invoke<OrganizationProposal>("get_organization_proposal", {
          files: scanned,
          useOllama: !settings.use_premium_ai,
          model: settings.ollama_model,
          userPreferences: settings.user_preferences || null,
        }),
        ANALYZE_TIMEOUT_MS,
        "L'analisi AI",
      );
      setProposal(p);
      setPhase("PREVIEW");
    } catch (e) {
      failTo("SELECT_DIR", e);
    }
  }

  /* -------------------------------- PREVIEW -------------------------------- */

  // Ricalcola il piano ogni volta che la proposta cambia (rinomina, merge, delete).
  useEffect(() => {
    if (phase !== "PREVIEW" || !proposal) return;
    let cancelled = false;
    invoke<MappingPlan>("create_mapping_plan", { root: rootPath, files, proposal })
      .then((p) => {
        if (!cancelled) setPlan(p);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [phase, proposal, files, rootPath]);

  /* -------------------------------- APPLYING ------------------------------- */

  const apply = useCallback(async () => {
    if (!plan || plan.moves.length === 0) return;
    setError(null);
    setApplyProgress({ done: 0, total: plan.moves.length, current: "" });
    setPhase("APPLYING");
    try {
      const r = await invoke<ApplyResult>("apply_mapping_plan", { plan });
      setResult(r);
      setPhase("RESULT");
    } catch (e) {
      failTo("PREVIEW", e);
    }
  }, [plan, failTo]);

  const undo = useCallback(async () => {
    setUndoing(true);
    try {
      const msg = await invoke<string>("undo_last_operation");
      setResult((prev) => (prev ? { ...prev, undo_available: false } : prev));
      setError(msg);
    } catch (e) {
      setError(String(e));
    } finally {
      setUndoing(false);
    }
  }, []);

  function restart() {
    setPhase("SELECT_DIR");
    setFiles([]);
    setProposal(null);
    setPlan(null);
    setResult(null);
    setError(null);
  }

  /* ---------------------------- scorciatoie tastiera --------------------------- */

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const inField = (e.target as HTMLElement)?.tagName === "INPUT";
      if (e.key === "Enter" && phase === "PREVIEW" && !inField) void apply();
      if (e.key === "Escape" && phase === "PREVIEW") setPhase("SELECT_DIR");
      if (e.ctrlKey && e.key.toLowerCase() === "z" && phase === "RESULT" && result?.undo_available) {
        e.preventDefault();
        void undo();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [phase, apply, undo, result]);

  /* --------------------------------- render -------------------------------- */

  return (
    <main className="app">
      {phase === "SELECT_DIR" && (
        <StepSelector settings={settings} onSettingsChange={setSettings} onStart={start} error={error} />
      )}

      {phase === "SCANNING" && (
        <ScanningPhase progress={scanProgress} elapsedMs={elapsedMs} onCancel={restart} />
      )}

      {phase === "ANALYZING" && (
        <AnalysisPhase
          fileCount={files.length}
          engine={settings.use_premium_ai ? "Claude" : "Ollama"}
          elapsedMs={elapsedMs}
        />
      )}

      {phase === "PREVIEW" && proposal && (
        <PreviewPhase
          proposal={proposal}
          plan={plan}
          onProposalChange={setProposal}
          onApply={apply}
          onBack={() => setPhase("SELECT_DIR")}
        />
      )}

      {phase === "APPLYING" && (
        <ApplyPhase
          progress={applyProgress}
          elapsedMs={elapsedMs}
          onCancel={() => void invoke("cancel_apply").catch(() => undefined)}
        />
      )}

      {phase === "RESULT" && result && (
        <ResultPhase
          result={result}
          conflicts={plan?.conflicts.length ?? 0}
          rootPath={rootPath}
          undoing={undoing}
          onOpenFolder={() => void invoke("open_folder_in_explorer", { path: rootPath }).catch(setError)}
          onUndo={undo}
          onRestart={restart}
        />
      )}
    </main>
  );
}
