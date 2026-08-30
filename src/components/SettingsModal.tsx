import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { call, isDesktop, subscribe } from "../api";
import RemoteAccessPanel from "./RemoteAccessPanel";
import {
  EMBED_MODEL,
  MAX_FACTS,
  type AiStatus,
  type Config,
  type Fact,
  type IndexProgress,
  type IndexStatus,
  type Preferences,
  type Toast,
  type UpdateInfo,
  type UpdateProgress,
} from "../types";

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

  // `null` = non ancora caricata, cosi' "non ricorda niente" non lampeggia
  // prima che la risposta arrivi.
  const [facts, setFacts] = useState<Fact[] | null>(null);

  useEffect(() => {
    call<Fact[]>("get_memory")
      .then(setFacts)
      .catch(() => setFacts([]));
  }, []);

  const [index, setIndex] = useState<IndexStatus | null>(null);
  const [indexing, setIndexing] = useState(false);
  const [progress, setProgress] = useState<IndexProgress | null>(null);

  useEffect(() => {
    call<IndexStatus>("index_status")
      .then(setIndex)
      .catch(() => setIndex(null));
  }, []);

  // L'indicizzazione dura minuti: senza l'avanzamento sembrerebbe piantata.
  useEffect(() => subscribe<IndexProgress>("index-progress", setProgress), []);

  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [download, setDownload] = useState<UpdateProgress | null>(null);

  // Il controllo parte da solo all'apertura delle impostazioni: e' l'unico
  // punto da cui si aggiorna, e chiedere due clic per sapere la versione
  // installata sarebbe uno di troppo.
  useEffect(() => {
    if (!isDesktop) return;
    call<UpdateInfo>("check_update")
      .then(setUpdate)
      .catch(() => setUpdate(null));
  }, []);

  useEffect(() => subscribe<UpdateProgress>("update-progress", setDownload), []);

  const claudeReady = status?.claude ?? false;
  const ollamaReady = status?.ollama ?? false;

  async function checkUpdate() {
    setChecking(true);
    try {
      const found = await call<UpdateInfo>("check_update");
      setUpdate(found);
      if (!found.available && !found.reason) onNotice("info", "Sei già all'ultima versione.");
    } catch (e) {
      onNotice("error", String(e));
    } finally {
      setChecking(false);
    }
  }

  async function installUpdate() {
    setDownload({ downloaded: 0, total: null });
    try {
      // Se va bene non torna: il processo si riavvia da solo.
      await call("install_update");
    } catch (e) {
      onNotice("error", String(e));
      setDownload(null);
    }
  }

  async function buildIndex() {
    setIndexing(true);
    setProgress(null);
    try {
      const built = await call<IndexStatus>("build_index", { ollamaUrl: prefs.ollama_url });
      setIndex(built);
      onNotice("success", `Indicizzati ${built.files} file in ${built.pieces} pezzi.`);
    } catch (e) {
      onNotice("error", String(e));
    } finally {
      setIndexing(false);
      setProgress(null);
    }
  }

  async function dropIndex() {
    try {
      await call("clear_index");
      setIndex(await call<IndexStatus>("index_status"));
      // Senza indice l'interruttore non ha piu' niente da consultare: si
      // spegne qui invece di lasciare una ricerca che non trova mai niente.
      setPrefs((p) => ({ ...p, documents: false }));
      onNotice("success", "Indice cancellato.");
    } catch (e) {
      onNotice("error", String(e));
    }
  }

  async function forget(id: string) {
    try {
      await call("forget_fact", { factId: id });
      setFacts((current) => (current ?? []).filter((f) => f.id !== id));
    } catch (e) {
      onNotice("error", String(e));
    }
  }

  async function forgetAll() {
    try {
      await call("clear_memory");
      setFacts([]);
      onNotice("success", "Memoria svuotata.");
    } catch (e) {
      onNotice("error", String(e));
    }
  }

  /// Il dialogo di sistema e' l'unico modo per autorizzare una cartella: un
  /// percorso scritto a mano nel browser aggirerebbe il senso del recinto.
  async function addFolder() {
    const picked = await open({ directory: true, multiple: true, title: "Cartelle che Logica puo' leggere" });
    if (!picked) return;
    const chosen = Array.isArray(picked) ? picked : [picked];
    setCfg((c) => ({
      ...c,
      monitored_folders: [...new Set([...c.monitored_folders, ...chosen])],
    }));
  }

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

        {isDesktop && (
          <section>
            <h3>Aggiornamenti</h3>
            <p className="note">
              {update ? `Versione installata: ${update.current}.` : "Controllo della versione…"}
            </p>

            {update?.reason && <p className="note">{update.reason}</p>}

            {update?.available && (
              <>
                <p className="note">
                  Disponibile la <strong>{update.version}</strong>.
                  {update.notes ? ` ${update.notes}` : ""}
                </p>
                <p className="note">
                  Installandola l'app si chiude e si riapre da sola: su Windows l'installer
                  sostituisce l'eseguibile in uso.
                </p>
              </>
            )}

            {download && (
              <p className="note">
                Scaricato {Math.round(download.downloaded / 1024)} KB
                {download.total ? ` di ${Math.round(download.total / 1024)} KB` : ""}
              </p>
            )}

            <div className="row-actions">
              <button className="tool-btn" disabled={checking} onClick={() => void checkUpdate()}>
                {checking ? "Controllo…" : "Controlla aggiornamenti"}
              </button>
              {update?.available && (
                <button className="tool-btn primary" onClick={() => void installUpdate()}>
                  Installa e riavvia
                </button>
              )}
            </div>
          </section>
        )}

        <section>
          <h3>Documenti</h3>
          <label className="switch">
            <input
              type="checkbox"
              checked={prefs.documents}
              disabled={!isDesktop || (index?.pieces ?? 0) === 0}
              onChange={(e) => setPrefs((p) => ({ ...p, documents: e.target.checked }))}
            />
            <span>Cerca nei documenti indicizzati prima di rispondere</span>
          </label>
          <p className="note">
            Diverso dagli strumenti sui file: quelli trovano un nome o una parola esatta, questo
            trova per significato — «il documento in cui parlavo delle condizioni di rimborso» anche
            se la parola «rimborso» dentro non c'è. I documenti delle cartelle autorizzate vengono
            tagliati a pezzi e trasformati in vettori una volta sola; a ogni domanda si confronta la
            domanda con i pezzi e i più vicini entrano nel prompt. Se non somiglia niente, non entra
            niente.
          </p>
          <p className="note">
            I vettori li calcola sempre Ollama, anche in modalità Claude: serve{" "}
            <code>ollama pull {EMBED_MODEL}</code>. Niente esce dal PC per indicizzare.
          </p>

          {index && index.pieces > 0 ? (
            <p className="note">
              {index.files} file, {index.pieces} pezzi. Ultima indicizzazione:{" "}
              {new Date(index.built).toLocaleString()}.
            </p>
          ) : (
            <p className="note">Nessun indice: finché non lo costruisci l'interruttore non si accende.</p>
          )}

          {progress && progress.total > 0 && (
            <p className="note">
              Indicizzo {progress.done}/{progress.total}
              {progress.file && ` — ${progress.file}`}
            </p>
          )}

          {isDesktop && (
            <div className="row-actions">
              <button className="tool-btn" disabled={indexing} onClick={() => void buildIndex()}>
                {indexing ? "Indicizzazione in corso…" : index && index.pieces > 0 ? "Aggiorna indice" : "Costruisci indice"}
              </button>
              {index && index.pieces > 0 && (
                <button className="tool-btn" disabled={indexing} onClick={() => void dropIndex()}>
                  Cancella indice
                </button>
              )}
            </div>
          )}
        </section>

        <section>
          <h3>Memoria</h3>
          <label className="switch">
            <input
              type="checkbox"
              checked={prefs.memory}
              onChange={(e) => setPrefs((p) => ({ ...p, memory: e.target.checked }))}
            />
            <span>Ricorda fra una conversazione e l'altra</span>
          </label>
          <p className="note">
            Alla fine di ogni scambio Logica si annota i fatti che varrà la pena sapere anche fra un
            mese — chi sei, cosa usi, come vuoi le risposte — e li rimette nel prompt delle chat
            successive. Non l'intera conversazione: {MAX_FACTS} righe al massimo, tutte qui sotto,
            tutte cancellabili. Spegnendo l'interruttore smette di annotare e di leggere; quello che
            c'è resta finché non lo cancelli.
          </p>

          {facts === null ? (
            <p className="note">Caricamento…</p>
          ) : facts.length === 0 ? (
            <p className="note">Non ha ancora imparato niente.</p>
          ) : (
            <>
              <ul className="fact-list">
                {facts.map((fact) => (
                  <li key={fact.id}>
                    <span>{fact.text}</span>
                    <button className="tool-btn" onClick={() => void forget(fact.id)}>
                      Dimentica
                    </button>
                  </li>
                ))}
              </ul>
              <button className="tool-btn" onClick={() => void forgetAll()}>
                Dimentica tutto
              </button>
            </>
          )}
        </section>

        <section>
          <h3>File</h3>
          <label className="switch">
            <input
              type="checkbox"
              checked={prefs.file_tools}
              disabled={!isDesktop}
              onChange={(e) => setPrefs((p) => ({ ...p, file_tools: e.target.checked }))}
            />
            <span>Lascia che Logica guardi nelle cartelle qui sotto</span>
          </label>
          <p className="note">
            Con l'interruttore acceso l'assistente puo' elencare le cartelle, leggere i file e
            cercarci dentro, ma solo qui: tutto il resto del disco resta fuori. Non puo' scrivere,
            spostare o cancellare niente. Quello che legge finisce nel prompt del modello: con Claude
            API esce quindi in rete, con Ollama resta sul PC.
            {!isDesktop && " Le cartelle si scelgono solo dal PC."}
          </p>

          {cfg.monitored_folders.length === 0 ? (
            <p className="note">Nessuna cartella autorizzata: finche' non ne aggiungi una, l'interruttore non fa niente.</p>
          ) : (
            <ul className="allowed-folders">
              {cfg.monitored_folders.map((folder) => (
                <li key={folder}>
                  <span title={folder}>{folder}</span>
                  <button
                    className="tool-btn"
                    disabled={!isDesktop}
                    onClick={() =>
                      setCfg((c) => ({
                        ...c,
                        monitored_folders: c.monitored_folders.filter((f) => f !== folder),
                      }))
                    }
                  >
                    Togli
                  </button>
                </li>
              ))}
            </ul>
          )}

          {isDesktop && (
            <button className="tool-btn" onClick={() => void addFolder()}>
              Aggiungi cartella
            </button>
          )}
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
