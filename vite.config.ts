/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri serves the frontend from a fixed port and must fail loudly if it's taken.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  // I test girano su Node: il parser Markdown si rende in HTML statico, e
  // nessun test tocca il DOM. Aggiungere jsdom costerebbe una dipendenza e
  // qualche secondo per ogni esecuzione, senza provare niente di piu'.
  test: {
    environment: "node",
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
