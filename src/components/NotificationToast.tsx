import { useEffect } from "react";
import type { Toast } from "../types";

interface Props {
  toasts: Toast[];
  onDismiss: (id: string) => void;
}

const AUTO_DISMISS_MS = 6000;

function ToastRow({ toast, onDismiss }: { toast: Toast; onDismiss: (id: string) => void }) {
  useEffect(() => {
    const id = setTimeout(() => onDismiss(toast.id), AUTO_DISMISS_MS);
    return () => clearTimeout(id);
  }, [toast.id, onDismiss]);

  return (
    <div className={`toast ${toast.kind}`} role="status">
      <p>{toast.message}</p>
      <button className="btn ghost tiny" onClick={() => onDismiss(toast.id)} aria-label="Chiudi notifica">
        ✕
      </button>
    </div>
  );
}

export default function NotificationToast({ toasts, onDismiss }: Props) {
  if (toasts.length === 0) return null;
  return (
    <div className="toast-stack" aria-live="polite">
      {toasts.map((t) => (
        <ToastRow key={t.id} toast={t} onDismiss={onDismiss} />
      ))}
    </div>
  );
}
