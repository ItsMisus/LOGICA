import { useEffect, useState } from "react";

import { isDesktop } from "../api";
import RemoteAccessPanel from "./RemoteAccessPanel";
import type { AiStatus, Config, Preferences, Toast } from "../types";

interface Props {
  preferences: Preferences;
  config: Config;
  status: AiStatus | null;
  onSave: (preferences: Preferences, config: Config) => void;
  onClose: () => void;
  /** Esito delle azioni sul server, mostrato dalla pagina come avviso. */
  onNotice: (kind: Toast["kind"], message: string) => void;
}

export default function SettingsModal({
  preferences,
  config,
  status,
  onSave,
  onClose,
  onNotice,
}: Props) {
  const [prefs, setPrefs] = useState<Preferences>(preferences);
  const [cfg, setCfg] = useState<Config>(config);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const claudeReady = status?.claude ?? false;
  const ollamaReady = status?.ollama ?? false;

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" role="dialog" aria-label="Impostazioni" onClick={(e) => e.stopPropagation()}>
        <header className="modal-head">
          <h2>Impostazioni</h2>
          <button className="icon-btn" onClick={onClose} aria-label="Chiudi">
            ✕
          </button>
        </header>

        <section>
          <h3>Modello</h3>
          <div className="engine-choice">
            <button
              className={`engine-card ${prefs.use_premium_ai ? "" : "selected"}`}
              onClick={() => setPrefs((p) => ({ ...p, use_premium_ai: false }))}
            >
              <strong>Ollama (locale)</strong>
              <span className={`state ${ollamaReady ? "on" : "off"}`}>
                {ollamaReady ? "in esecuzione" : "non raggiungibile"}
              </span>
              <span className="note">Gira sul tuo PC, non esce niente in rete.</span>
            </button>

            <button
              className={`engine-card ${prefs.use_premium_ai ? "selected" : ""}`}
              onClick={() => setPrefs((p) => ({ ...p, use_premium_ai: true }))}
            >
              <strong>Claude API</strong>
              <span className={`state ${claudeReady ? "on" : "off"}`}>
                {claudeReady ? "chiave presente" : "ANTHROPIC_API_KEY assente"}
              </span>
              <span className="note">Risposte migliori, i messaggi vanno ad Anthropic.</span>
            </button>
          </div>

          {!prefs.use_premium_ai && (
            <>
              <label className="field">
                <span>Modello Ollama</span>
                {status && status.models.length > 0 ? (
                  <select
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
                    value={prefs.ollama_model}
                    onChange={(e) => setPrefs((p) => ({ ...p, ollama_model: e.target.value }))}
                  />
                )}
              </label>

              <label className="field">
                <span>Indirizzo di Ollama</span>
                <input
                  value={prefs.ollama_url}
                  disabled={!isDesktop}
                  onChange={(e) => setPrefs((p) => ({ ...p, ollama_url: e.target.value }))}
                />
                {!isDesktop && (
                  <span className="note">
                    Si cambia solo dal PC: da qui deciderebbe dove finiscono le conversazioni.
                  </span>
                )}
              </label>
            </>
          )}
        </section>

        <section>
          <h3>Internet</h3>
          <label className="switch">
            <input
              type="checkbox"
              checked={prefs.web_search}
              disabled={!isDesktop}
              onChange={(e) => setPrefs((p) => ({ ...p, web_search: e.target.checked }))}
            />
            <span>Cerca su internet quando serve</span>
          </label>
          <p className="note">
            Logica decide da sola se una domanda ha bisogno di dati aggiornati, cerca su DuckDuckGo e
            legge le prime pagine. La domanda riformulata esce in rete; i tuoi file no. I link che
            scrivi tu vengono aperti comunque, anche con l'interruttore spento.
            {!isDesktop && " Si cambia solo dal PC."}
          </p>
        </section>

        <section>
          <h3>Come deve risponderti</h3>
          <p className="note">
            Istruzioni che Logica riceve in ogni conversazione: tono, lingua, cosa sai gia', cosa eviti.
          </p>
          <textarea
            rows={4}
            value={cfg.user_preferences}
            placeholder="Es. sono uno studente di informatica, rispondimi in modo tecnico e senza giri di parole"
            onChange={(e) => setCfg((c) => ({ ...c, user_preferences: e.target.value, profile: "custom" }))}
          />
        </section>

        {isDesktop && (
          <RemoteAccessPanel
            onError={(m) => onNotice("error", m)}
            onDone={(m) => onNotice("success", m)}
          />
        )}

        <section>
          <h3>Aspetto</h3>
          <div className="theme-choice">
            {(["light", "dark"] as const).map((theme) => (
              <button
                key={theme}
                className={`chip ${prefs.theme === theme ? "selected" : ""}`}
                onClick={() => setPrefs((p) => ({ ...p, theme }))}
              >
                {theme === "dark" ? "Scuro" : "Chiaro"}
              </button>
            ))}
          </div>
        </section>

        <footer className="modal-foot">
          <button className="btn ghost" onClick={onClose}>
            Annulla
          </button>
          <button className="btn primary" onClick={() => onSave(prefs, cfg)}>
            Salva
          </button>
        </footer>
      </div>
    </div>
  );
}
