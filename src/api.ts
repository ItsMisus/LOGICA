/**
 * Un solo modo di chiamare il backend, due strade sotto.
 *
 * Nella finestra desktop si passa dalla IPC di Tauri. Nel browser, dall'altro
 * PC di casa, si passa da HTTPS verso il server che gira dentro LOGICA. I nomi
 * dei comandi e la forma degli argomenti sono identici, cosi' il resto
 * dell'interfaccia non sa quale delle due sta usando.
 *
 * Differenza che conta: da browser il backend applica limiti che sul desktop
 * non esistono (cartelle autorizzate, motore AI non modificabile). Quando un
 * comando viene rifiutato per quel motivo, l'errore arriva qui come testo
 * normale e va mostrato all'utente cosi' com'e'.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** `true` dentro la finestra dell'app, `false` in un browser. */
export const isDesktop: boolean =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Header con cui il server distingue le nostre richieste da quelle di un
 *  altro sito che provasse a usare il cookie. */
const CSRF_HEADER = "X-Logica-Csrf";

let csrfToken: string | null = null;

/** Chiamato quando il server dice che la sessione non vale piu'. */
type SessionLostHandler = () => void;
let onSessionLost: SessionLostHandler = () => {};

export function setSessionLostHandler(handler: SessionLostHandler): void {
  onSessionLost = handler;
}

/* --------------------------------- accesso -------------------------------- */

export interface SessionState {
  authenticated: boolean;
}

/** Sul desktop non c'e' login: la finestra e' gia' la prova di chi sei. */
export async function session(): Promise<SessionState> {
  if (isDesktop) return { authenticated: true };

  const res = await fetch("/api/session", { credentials: "same-origin" });
  if (!res.ok) return { authenticated: false };
  const body = (await res.json()) as { authenticated: boolean; csrf?: string };
  if (body.authenticated && body.csrf) csrfToken = body.csrf;
  return { authenticated: body.authenticated };
}

export async function login(password: string): Promise<void> {
  const res = await fetch("/api/login", {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ password }),
  });
  const body = await res.json().catch(() => ({}) as Record<string, unknown>);
  if (!res.ok) {
    throw new Error((body as { error?: string }).error ?? "Accesso non riuscito.");
  }
  csrfToken = (body as { csrf: string }).csrf;
}

export async function logout(): Promise<void> {
  if (isDesktop) return;
  await fetch("/api/logout", {
    method: "POST",
    credentials: "same-origin",
    headers: csrfToken ? { [CSRF_HEADER]: csrfToken } : {},
  }).catch(() => undefined);
  csrfToken = null;
  closeEvents();
}

/* --------------------------------- comandi -------------------------------- */

export async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (isDesktop) return invoke<T>(command, args);

  const res = await fetch(`/api/cmd/${command}`, {
    method: "POST",
    credentials: "same-origin",
    headers: {
      "Content-Type": "application/json",
      ...(csrfToken ? { [CSRF_HEADER]: csrfToken } : {}),
    },
    body: JSON.stringify(args ?? {}),
  });

  if (res.status === 401) {
    csrfToken = null;
    closeEvents();
    onSessionLost();
    throw new Error("Sessione scaduta: rifai l'accesso.");
  }

  const body = await res.json().catch(() => null);
  if (!res.ok) {
    const message = (body as { error?: string } | null)?.error;
    throw new Error(message ?? `Richiesta fallita (${res.status}).`);
  }
  return body as T;
}

/* --------------------------------- eventi --------------------------------- */

type Handler = (payload: unknown) => void;

let source: EventSource | null = null;
const handlers = new Map<string, Set<Handler>>();

/**
 * Una sola connessione di eventi per tutta la pagina, con i sottoscrittori
 * sopra: aprirne una per componente vorrebbe dire tenere aperte piu'
 * connessioni verso lo stesso flusso.
 */
function ensureEvents(): void {
  if (source) return;
  source = new EventSource("/api/events", { withCredentials: true });
  for (const name of handlers.keys()) attachNative(name);
}

function attachNative(name: string): void {
  source?.addEventListener(name, (event) => {
    const message = event as MessageEvent<string>;
    let payload: unknown = null;
    try {
      payload = JSON.parse(message.data);
    } catch {
      return;
    }
    handlers.get(name)?.forEach((h) => h(payload));
  });
}

function closeEvents(): void {
  source?.close();
  source = null;
}

/** Ritorna la funzione che disiscrive, come `listen` di Tauri. */
export function subscribe<T>(event: string, handler: (payload: T) => void): () => void {
  if (isDesktop) {
    const pending = listen<T>(event, (e) => handler(e.payload));
    return () => {
      void pending.then((off) => off());
    };
  }

  const wrapped: Handler = (payload) => handler(payload as T);
  const existing = handlers.get(event);
  if (existing) {
    existing.add(wrapped);
  } else {
    handlers.set(event, new Set([wrapped]));
    if (source) attachNative(event);
  }
  ensureEvents();

  return () => {
    const set = handlers.get(event);
    set?.delete(wrapped);
    if (set && set.size === 0) handlers.delete(event);
  };
}
