import { useState } from "react";

import { login } from "../api";

interface Props {
  onAccess: () => void;
}

/**
 * La schermata che si vede da browser prima di entrare. Nella finestra desktop
 * non compare mai: li' la prova di chi sei e' l'aver aperto l'app sul tuo PC.
 *
 * L'errore che arriva dal server e' volutamente povero ("Password errata")
 * oppure dice quanto manca alla fine di un blocco: nessuna informazione in
 * piu' a chi sta provando a indovinare.
 */
export default function LoginScreen({ onAccess }: Props) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (busy || password.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      await login(password);
      setPassword("");
      onAccess();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="login-screen">
      <form className="login-card" onSubmit={(e) => void submit(e)}>
        <h1 className="login-title">LOGICA</h1>
        <p className="login-subtitle">Accesso da remoto ai file del tuo PC.</p>

        <label className="login-label" htmlFor="login-password">
          Password
        </label>
        <input
          id="login-password"
          className="login-input"
          type="password"
          value={password}
          autoComplete="current-password"
          autoFocus
          onChange={(e) => setPassword(e.target.value)}
        />

        {error && (
          <p className="login-error" role="alert">
            {error}
          </p>
        )}

        <button className="login-button" type="submit" disabled={busy || password.length === 0}>
          {busy ? "Verifica..." : "Entra"}
        </button>

        <p className="login-note">
          Da qui si lavora solo dentro le cartelle autorizzate sul PC. Tutto il resto del disco
          resta fuori portata, anche da dentro.
        </p>
      </form>
    </div>
  );
}
