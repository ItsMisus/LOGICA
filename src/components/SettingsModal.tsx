import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { PROFILES, type AiStatus, type Config, type Preferences } from "../types";

interface Props {
  preferences: Preferences;
  config: Config;
  status: AiStatus | null;
  onSave: (preferences: Preferences, config: Config) => void;
  onClose: () => void;
}

export default function SettingsModal({ preferences, config, status, onSave, onClose }: Props) {
  const [prefs, setPrefs] = useState<Preferences>(preferences);
  const [cfg, setCfg] = useState<Config>(config);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function addFolders() {
    const picked = await open({ directory: true, multiple: true, title: "Cartelle da monitorare" });
    if (!picked) return;
    const list = Array.isArray(picked) ? picked : [picked];
    setCfg((c) => ({
      ...c,
      monitored_folders: Array.from(new Set([...c.monitored_folders, ...list])),
    }));
  }

  function chooseProfile(profile: string) {
    // Il profilo riempie le preferenze testuali, che restano modificabili.
    setCfg((c) => ({ ...c, profile, user_preferences: PROFILES[profile] ?? c.user_preferences }));
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" role="dialog" aria-label="Impostazioni" onClick={(e) => e.stopPropagation()}>
        <header className="row space-between">
          <h2>Impostazioni</h2>
          <button className="btn ghost tiny" onClick={onClose} aria-label="Chiudi">
            ✕
          </button>
        </header>

        <section>
          <h3>Cartelle da monitorare</h3>
          <p className="hint">Suggerite nella chat quando apri una nuova conversazione.</p>
          <ul className="folder-list">
            {cfg.monitored_folders.map((folder) => (
              <li key={folder}>
                <span className="mono truncate" title={folder}>
                  {folder}
                </span>
                <button
                  className="btn ghost tiny danger"
                  onClick={() =>
                    setCfg((c) => ({
                      ...c,
                      monitored_folders: c.monitored_folders.filter((f) => f !== folder),
                    }))
                  }
                >
                  Rimuovi
                </button>
              </li>
            ))}
            {cfg.monitored_folders.length === 0 && <li className="hint">Nessuna cartella.</li>}
          </ul>
          <button className="btn ghost small" onClick={() => void addFolders()}>
            + Aggiungi cartella
          </button>
        </section>

        <section>
          <h3>Profilo</h3>
          <div className="row wrap">
            {["developer", "media", "office", "custom"].map((p) => (
              <button
                key={p}
                className={`btn chip ${cfg.profile === p ? "selected" : ""}`}
                onClick={() => chooseProfile(p)}
              >
                {p === "developer" ? "Sviluppatore" : p === "media" ? "Media" : p === "office" ? "Ufficio" : "Personalizzato"}
              </button>
            ))}
          </div>
          <label className="field">
            <span>Preferenze passate a LOGICA</span>
            <textarea
              className="input"
              rows={3}
              value={cfg.user_preferences}
              placeholder="Es. tieni le fatture separate per anno"
              onChange={(e) => setCfg((c) => ({ ...c, user_preferences: e.target.value, profile: "custom" }))}
            />
          </label>
        </section>

        <section>
          <h3>Motore AI</h3>
          <label className="check">
            <input
              type="checkbox"
              checked={prefs.use_premium_ai}
              onChange={(e) => setPrefs((p) => ({ ...p, use_premium_ai: e.target.checked }))}
            />
            <span>
              Usa Claude API {status && !status.claude && <em className="hint">(ANTHROPIC_API_KEY non impostata)</em>}
            </span>
          </label>

          <label className="field">
            <span>URL Ollama</span>
            <input
              className="input"
              value={prefs.ollama_url}
              onChange={(e) => setPrefs((p) => ({ ...p, ollama_url: e.target.value }))}
            />
          </label>

          <label className="field">
            <span>Modello Ollama</span>
            {status && status.models.length > 0 ? (
              <select
                className="input"
                value={prefs.ollama_model}
                onChange={(e) => setPrefs((p) => ({ ...p, ollama_model: e.target.value }))}
              >
                {/* Il modello salvato puo' non essere piu' installato: resta selezionabile. */}
                {!status.models.includes(prefs.ollama_model) && (
                  <option value={prefs.ollama_model}>{prefs.ollama_model} (non installato)</option>
                )}
                {status.models.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            ) : (
              <input
                className="input"
                value={prefs.ollama_model}
                onChange={(e) => setPrefs((p) => ({ ...p, ollama_model: e.target.value }))}
              />
            )}
          </label>
        </section>

        <section>
          <h3>Tema</h3>
          <div className="row">
            {(["dark", "light"] as const).map((theme) => (
              <button
                key={theme}
                className={`btn chip ${prefs.theme === theme ? "selected" : ""}`}
                onClick={() => setPrefs((p) => ({ ...p, theme }))}
              >
                {theme === "dark" ? "🌙 Scuro" : "☀️ Chiaro"}
              </button>
            ))}
          </div>
        </section>

        <div className="row space-between">
          <button className="btn ghost" onClick={onClose}>
            Annulla
          </button>
          <button className="btn primary" onClick={() => onSave(prefs, cfg)}>
            Salva
          </button>
        </div>
      </div>
    </div>
  );
}
