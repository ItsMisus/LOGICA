import React from "react";
import ReactDOM from "react-dom/client";

// I caratteri viaggiano dentro l'app: nessuna richiesta di rete all'avvio.
// Solo il sottoinsieme latino e i pesi davvero usati dall'interfaccia.
import "@fontsource/playfair-display/latin-400.css";
import "@fontsource/playfair-display/latin-500.css";
import "@fontsource/inter/latin-400.css";
import "@fontsource/inter/latin-500.css";
import "@fontsource/inter/latin-600.css";
import "@fontsource/jetbrains-mono/latin-400.css";

import App from "./App";
import "./App.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
