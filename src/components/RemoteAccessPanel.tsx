import { useCallback, useEffect, useState } from "react";

import { call } from "../api";
import { MIN_PASSWORD_LEN, type ServerStatus } from "../types";

interface Props {
  onError: (message: string) => void;
  onDone: (message: string) => void;
}

/**
 * Accesso da browser: si accende qui, dal PC, e solo da qui.
 *
 * Le scelte che allargano la superficie esposta (porta aperta sulla rete,
 * avvio automatico) stanno tutte in questo pannello, con scritto accanto cosa
 * comportano. Quello che il browser puo' toccare invece non si decide qui: e'
 * l'elenco delle cartelle aperte in LOGICA, mostrato in fondo.
 */
export default function RemoteAccessPanel({ onError, onDone }: Props) {
  const [status, setStatus] = useState<ServerStatus | null>(null);
  const [password, setPassword] = useState("");
  const [port, setPort] = useState(8443);
  const [exposeToLan, setExposeToLan] = useState(false);
  const [autostart, setAutostart] = useState(false);
  const [busy, setBusy] = useState(false);

  const apply = useCallback((s: ServerStatus) => {
    setStatus(s);
    setPort(s.port);
    setExposeToLan(s.exposeToLan);
    setAutostart(s.autostart);
  }, []);

  useEffect(() => {
    void call<ServerStatus>("server_status").then(apply).catch(() => undefined);
  }, [apply]);

  async function run(action: () => Promise<ServerStatus>, done: string) {
    if (busy) return;
    setBusy(true);
    try {
      apply(await action());
      onDone(done);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  if (!status) return null;

  const running = status.running;
  const noFolders = status.allowedFolders.length === 0;

  return (
    <section>
      <h3>Accesso da browser</h3>
      <p className="note">
        Apre LOGICA a un altro dispositivo della rete di casa. Da spento non c'e' nessuna porta
        aperta.
      </p>

      <div className="server-state">
        <span className={`engine-dot ${running ? "on" : "off"}`} aria-hidden="true" />
        {running ? "Acceso" : "Spento"}
      </div>

      {/* Password: senza, il server rifiuta di partire. */}
      <label className="field">
        <span>{status.hasPassword ? "Cambia password" : "Password di accesso"}</span>
        <input
          type="password"
          value={password}
          autoComplete="new-password"
          placeholder={`almeno ${MIN_PASSWORD_LEN} caratteri`}
          onChange={(e) => setPassword(e.target.value)}
        />
      </label>
      <button
        className="btn ghost"
        disabled={busy || password.length < MIN_PASSWORD_LEN}
        onClick={() =>
          void run(async () => {
            const s = await call<ServerStatus>("server_set_password", { password });
            setPassword("");
            return s;
          }, "Password impostata. Il server e' stato spento: riaccendilo.")
        }
      >
        {status.hasPassword ? "Sostituisci password" : "Imposta password"}
      </button>

      <label className="field">
        <span>Porta</span>
        <input
          type="number"
          min={1024}
          max={65535}
          value={port}
          onChange={(e) => setPort(Number(e.target.value))}
        />
      </label>

      <label className="check">
        <input
          type="checkbox"
          checked={exposeToLan}
          onChange={(e) => setExposeToLan(e.target.checked)}
        />
        <span>Raggiungibile dagli altri dispositivi della rete locale</span>
      </label>
      <p className="note">
        Da spento, il server risponde solo a questo PC. Da acceso, risponde a chiunque sia
        collegato alla tua rete e conosca la password. Non aprire questa porta sul router: per
        usarlo fuori casa serve una VPN.
      </p>

      <label className="check">
        <input
          type="checkbox"
          checked={autostart}
          onChange={(e) => setAutostart(e.target.checked)}
        />
        <span>Accendi il server all'avvio di LOGICA</span>
      </label>

      <button
        className="btn ghost"
        disabled={busy}
        onClick={() =>
          void run(
            () => call<ServerStatus>("server_configure", { options: { port, exposeToLan, autostart } }),
            "Impostazioni salvate. Riaccendi il server per applicarle.",
          )
        }
      >
        Salva impostazioni del server
      </button>

      <div className="server-actions">
        {running ? (
          <button
            className="btn ghost"
            disabled={busy}
            onClick={() => void run(() => call<ServerStatus>("server_stop"), "Server spento.")}
          >
            Spegni
          </button>
        ) : (
          <button
            className="btn primary"
            disabled={busy || !status.hasPassword}
            onClick={() => void run(() => call<ServerStatus>("server_start"), "Server acceso.")}
          >
            Accendi
          </button>
        )}
      </div>

      {!status.hasPassword && (
        <p className="note warn">Imposta una password: senza, il server non parte.</p>
      )}

      {running && status.url && (
        <div className="server-info">
          <p>
            Indirizzo da aprire sull'altro PC: <code>{status.url}</code>
          </p>
          <p className="note">
            Il browser avvisera' che il certificato non e' verificato: e' atteso, se lo firma
            LOGICA stessa. Prima di accettare, controlla che l'impronta mostrata dal browser sia
            identica a questa. Se non lo e', chiudi la pagina: non stai parlando con il tuo PC.
          </p>
          <code className="fingerprint">{status.fingerprint}</code>
        </div>
      )}

      <div className="server-folders">
        <h4>Cartelle raggiungibili da browser</h4>
        {noFolders ? (
          <p className="note warn">
            Nessuna. Da remoto non si puo' toccare niente finche' non apri una cartella in LOGICA
            da questo PC.
          </p>
        ) : (
          <>
            <ul className="folder-list">
              {status.allowedFolders.map((f) => (
                <li key={f}>
                  <code>{f}</code>
                </li>
              ))}
            </ul>
            <p className="note">
              Solo queste. Ogni operazione che nomina un percorso fuori da qui viene rifiutata dal
              backend, anche a sessione valida, e questo elenco si cambia solo dal PC.
            </p>
          </>
        )}
      </div>

      <button
        className="btn ghost"
        disabled={busy}
        onClick={() =>
          void run(
            () => call<ServerStatus>("server_regenerate_certificate"),
            "Certificato rigenerato: i browser mostreranno una impronta nuova.",
          )
        }
      >
        Rigenera certificato
      </button>
    </section>
  );
}
