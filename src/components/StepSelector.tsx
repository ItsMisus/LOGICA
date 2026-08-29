import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import type { Settings } from "../types";

interface Props {
  settings: Settings;
  onSettingsChange: (s: Settings) => void;
  onStart: (path: string) => void;
  error: string | null;
}

export default function StepSelector({ settings, onSettingsChange, onStart, error }: Props) {
  const [path, setPath] = useState("");
  const [ollamaOnline, setOllamaOnline] = useState<boolean | null>(null);
  const [showSettings, setShowSettings] = useState(false);

  // Ping Ollama once at mount so the user knows if the local AI is reachable.
  useEffect(() => {
    invoke<boolean>("check_ollama_connection")
      .then(setOllamaOnline)
      .catch(() => setOllamaOnline(false));
  }, []);

  async function pickFolder() {
    const selected = await open({ directory: true, multiple: false, title: "Scegli la cartella da organizzare" });
    if (typeof selected === "string") setPath(selected);
  }

  return (
    <div className="card">
      <h1>File Organizer AI</h1>
      <p className="subtitle">
        Analizza una cartella con AI locale e propone una struttura ordinata. Niente viene spostato
        prima della tua conferma.
      </p>

      <div className="row">
        <input
          className="input"
          value={path}
          placeholder="C:\Users\...\Downloads"
          onChange={(e) => setPath(e.target.value)}
          aria-label="Percorso cartella"
        />
        <button className="btn ghost" onClick={pickFolder}>
          Sfoglia…
        </button>
      </div>

      <div className="row space-between">
        <span className={`badge ${ollamaOnline ? "ok" : ollamaOnline === null ? "" : "warn"}`}>
          {ollamaOnline === null
            ? "Controllo Ollama…"
            : ollamaOnline
              ? `Ollama online (${settings.ollama_model})`
              : "Ollama offline — verrà usato lo schema di fallback"}
        </span>
        <button className="btn ghost small" onClick={() => setShowSettings((v) => !v)}>
          {showSettings ? "Chiudi impostazioni" : "Impostazioni"}
        </button>
      </div>

      {showSettings && (
        <div className="settings">
          <label className="row">
            <input
              type="checkbox"
              checked={settings.use_premium_ai}
              onChange={(e) => onSettingsChange({ ...settings, use_premium_ai: e.target.checked })}
            />
            <span>
              Usa Claude API (premium) invece di Ollama — richiede la variabile d'ambiente
              <code> ANTHROPIC_API_KEY</code>
            </span>
          </label>
          <label className="field">
            <span>Modello Ollama</span>
            <input
              className="input"
              value={settings.ollama_model}
              onChange={(e) => onSettingsChange({ ...settings, ollama_model: e.target.value })}
            />
          </label>
          <label className="field">
            <span>Preferenze (es. "developer", "media-library")</span>
            <input
              className="input"
              value={settings.user_preferences}
              placeholder="opzionale"
              onChange={(e) => onSettingsChange({ ...settings, user_preferences: e.target.value })}
            />
          </label>
        </div>
      )}

      {error && <div className="alert error">{error}</div>}

      <button className="btn primary" disabled={!path.trim()} onClick={() => onStart(path.trim())}>
        Scansiona cartella
      </button>
    </div>
  );
}
