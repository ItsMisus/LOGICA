import { useCallback, useEffect, useState } from "react";

import LoginScreen from "./components/LoginScreen";
import ChatPage from "./pages/ChatPage";
import { isDesktop, session, setSessionLostHandler } from "./api";

/**
 * Nella finestra desktop si entra subito. Da browser si passa dal login, e ci
 * si torna appena il server dice che la sessione non vale piu' (scaduta,
 * chiusa, o server riacceso).
 */
export default function App() {
  const [authenticated, setAuthenticated] = useState(isDesktop);
  const [checked, setChecked] = useState(isDesktop);

  useEffect(() => {
    if (isDesktop) return;
    void session()
      .then((s) => setAuthenticated(s.authenticated))
      .catch(() => setAuthenticated(false))
      .finally(() => setChecked(true));
  }, []);

  useEffect(() => {
    setSessionLostHandler(() => setAuthenticated(false));
  }, []);

  const onAccess = useCallback(() => setAuthenticated(true), []);

  // Un attimo di attesa prima di sapere se la sessione c'e': senza, il login
  // lampeggerebbe anche a chi era gia' dentro.
  if (!checked) return <div className="login-screen" />;

  if (!authenticated) return <LoginScreen onAccess={onAccess} />;

  return <ChatPage />;
}
