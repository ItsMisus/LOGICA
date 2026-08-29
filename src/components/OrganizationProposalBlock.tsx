import { useMemo, useState } from "react";
import type { OrganizationProposal, PlanPreview, Rule, SchemaNode } from "../types";

interface Props {
  proposal: OrganizationProposal;
  preview: PlanPreview | null;
  /** false quando la proposta e' storica (gia' applicata o annullata). */
  active: boolean;
  busy: boolean;
  onProposalChange: (p: OrganizationProposal) => void;
  onApprove: () => void;
  onCancel: () => void;
}

/* ---------- helper sullo schema (albero di cartelle, immutabile) ---------- */

function cloneSchema(node: SchemaNode): SchemaNode {
  return JSON.parse(JSON.stringify(node)) as SchemaNode;
}

/** Rimuove il nodo indicato da `path` e ritorna il sottoalbero rimosso. */
function detach(schema: SchemaNode, path: string[]): SchemaNode | null {
  let parent = schema;
  for (const part of path.slice(0, -1)) {
    if (!parent[part]) return null;
    parent = parent[part];
  }
  const leaf = path[path.length - 1];
  const removed = parent[leaf];
  if (!removed) return null;
  delete parent[leaf];
  return removed;
}

/** Inserisce `subtree` sotto `path` (path vuoto = radice) con nome `name`. */
function attach(schema: SchemaNode, path: string[], name: string, subtree: SchemaNode) {
  let parent = schema;
  for (const part of path) {
    if (!parent[part]) parent[part] = {};
    parent = parent[part];
  }
  // Se esiste gia' una cartella con lo stesso nome, i due rami vengono uniti.
  parent[name] = { ...(parent[name] ?? {}), ...subtree };
}

/** Riscrive le destinazioni delle regole quando una cartella cambia path. */
function rewriteRules(rules: Rule[], oldPrefix: string, newPrefix: string | null): Rule[] {
  return rules.map((r) => {
    if (r.destination !== oldPrefix && !r.destination.startsWith(oldPrefix + "/")) return r;
    if (newPrefix === null) return { ...r, destination: "Vari" };
    return { ...r, destination: newPrefix + r.destination.slice(oldPrefix.length) };
  });
}

/* ---------------------------- albero cartelle ---------------------------- */

interface TreeProps {
  node: SchemaNode;
  path: string[];
  editable: boolean;
  collapsed: Set<string>;
  onToggle: (key: string) => void;
  onRename: (path: string[], name: string) => void;
  onDelete: (path: string[]) => void;
  onMove: (from: string[], to: string[]) => void;
}

function Tree({ node, path, editable, collapsed, onToggle, onRename, onDelete, onMove }: TreeProps) {
  const [editing, setEditing] = useState<string | null>(null);
  const entries = Object.keys(node).sort((a, b) => a.localeCompare(b, "it"));

  return (
    <ul className="tree">
      {entries.map((name) => {
        const childPath = [...path, name];
        const key = childPath.join("/");
        const hasChildren = Object.keys(node[name]).length > 0;
        const isCollapsed = collapsed.has(key);

        return (
          <li key={key}>
            <div
              className="tree-row"
              draggable={editable}
              onDragStart={(e) => e.dataTransfer.setData("text/plain", key)}
              onDragOver={(e) => editable && e.preventDefault()}
              onDrop={(e) => {
                if (!editable) return;
                e.preventDefault();
                e.stopPropagation();
                const from = e.dataTransfer.getData("text/plain");
                if (from && from !== key) onMove(from.split("/"), childPath);
              }}
            >
              <button
                className="twisty"
                onClick={() => onToggle(key)}
                aria-label={isCollapsed ? "Espandi cartella" : "Comprimi cartella"}
                disabled={!hasChildren}
              >
                {hasChildren ? (isCollapsed ? "▸" : "▾") : "·"}
              </button>

              {editing === key ? (
                <input
                  className="input inline"
                  autoFocus
                  defaultValue={name}
                  aria-label="Nuovo nome cartella"
                  onBlur={(e) => {
                    setEditing(null);
                    const v = e.target.value.trim();
                    if (v && v !== name) onRename(childPath, v);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                    if (e.key === "Escape") setEditing(null);
                  }}
                />
              ) : (
                <span
                  className="tree-name"
                  onDoubleClick={() => editable && setEditing(key)}
                  title={editable ? "Doppio click per rinominare" : undefined}
                >
                  <span aria-hidden="true">📁</span> {name}
                </span>
              )}

              {editable && (
                <span className="tree-actions">
                  <button className="btn ghost tiny" onClick={() => setEditing(key)}>
                    Rinomina
                  </button>
                  <button className="btn ghost tiny danger" onClick={() => onDelete(childPath)}>
                    Elimina
                  </button>
                </span>
              )}
            </div>

            {hasChildren && !isCollapsed && (
              <Tree
                node={node[name]}
                path={childPath}
                editable={editable}
                collapsed={collapsed}
                onToggle={onToggle}
                onRename={onRename}
                onDelete={onDelete}
                onMove={onMove}
              />
            )}
          </li>
        );
      })}
    </ul>
  );
}

/* ------------------------------- componente ------------------------------ */

const ROWS_PER_PAGE = 10;

export default function OrganizationProposalBlock({
  proposal,
  preview,
  active,
  busy,
  onProposalChange,
  onApprove,
  onCancel,
}: Props) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [editing, setEditing] = useState(false);
  const [page, setPage] = useState(0);

  const rows = useMemo(() => preview?.sample ?? [], [preview]);
  const pageCount = Math.max(1, Math.ceil(rows.length / ROWS_PER_PAGE));
  const visible = rows.slice(page * ROWS_PER_PAGE, page * ROWS_PER_PAGE + ROWS_PER_PAGE);

  // Lo schema si modifica solo quando il criterio e' per tipo: negli altri casi
  // le cartelle nascono dai metadati dei file, rinominarle non avrebbe effetto.
  const canEdit = active && proposal.grouping === "extension";

  function toggle(key: string) {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  function rename(path: string[], newName: string) {
    const schema = cloneSchema(proposal.schema);
    const subtree = detach(schema, path);
    if (!subtree) return;
    attach(schema, path.slice(0, -1), newName, subtree);
    const oldPrefix = path.join("/");
    const newPrefix = [...path.slice(0, -1), newName].join("/");
    onProposalChange({ ...proposal, schema, rules: rewriteRules(proposal.rules, oldPrefix, newPrefix) });
  }

  function remove(path: string[]) {
    const schema = cloneSchema(proposal.schema);
    if (!detach(schema, path)) return;
    // I file che puntavano qui finiscono in "Vari": non si perde mai nulla.
    onProposalChange({ ...proposal, schema, rules: rewriteRules(proposal.rules, path.join("/"), null) });
  }

  function move(from: string[], to: string[]) {
    const fromKey = from.join("/");
    const toKey = to.join("/");
    // Impedisce di trascinare una cartella dentro se stessa o un suo discendente.
    if (toKey === fromKey || toKey.startsWith(fromKey + "/")) return;
    const schema = cloneSchema(proposal.schema);
    const subtree = detach(schema, from);
    if (!subtree) return;
    const name = from[from.length - 1];
    attach(schema, to, name, subtree);
    onProposalChange({ ...proposal, schema, rules: rewriteRules(proposal.rules, fromKey, [...to, name].join("/")) });
  }

  return (
    <section className={`proposal ${active ? "" : "stale"}`}>
      <header className="proposal-head">
        <h2>Schema proposto</h2>
        <span className="badge">motore: {proposal.source}</span>
        {!active && <span className="badge muted">non piu' attiva</span>}
      </header>

      <p className="rationale">{proposal.rationale}</p>

      {preview && (
        <div className="stats">
          <div className="stat">
            <strong>{preview.total_moves.toLocaleString("it-IT")}</strong>
            <span>file da spostare</span>
          </div>
          <div className="stat">
            <strong>{preview.new_folders}</strong>
            <span>cartelle</span>
          </div>
          <div className="stat warn">
            <strong>{preview.conflicts}</strong>
            <span>conflitti risolti</span>
          </div>
          <div className="stat">
            <strong>{preview.unmatched.toLocaleString("it-IT")}</strong>
            <span>senza regola &rarr; Vari</span>
          </div>
        </div>
      )}

      <div className="split">
        <div>
          <h3>Cartelle</h3>
          {editing && canEdit && (
            <p className="hint">Doppio click per rinominare &middot; trascina una cartella dentro un&apos;altra per unirle.</p>
          )}
          <Tree
            node={proposal.schema}
            path={[]}
            editable={editing && canEdit}
            collapsed={collapsed}
            onToggle={toggle}
            onRename={rename}
            onDelete={remove}
            onMove={move}
          />
        </div>

        <div>
          <h3>Dove finiscono i file</h3>
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th>File</th>
                  <th>Destinazione</th>
                </tr>
              </thead>
              <tbody>
                {visible.map((m) => (
                  <tr key={m.source}>
                    <td className="mono truncate" title={m.source}>
                      {m.source.split(/[\\/]/).pop()}
                    </td>
                    <td className="mono truncate" title={m.relative_destination}>
                      {m.relative_destination}
                    </td>
                  </tr>
                ))}
                {visible.length === 0 && (
                  <tr>
                    <td colSpan={2}>Nessun file da spostare.</td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
          {rows.length > ROWS_PER_PAGE && (
            <div className="row space-between">
              <button className="btn ghost small" disabled={page === 0} onClick={() => setPage((p) => p - 1)}>
                &larr; Prec
              </button>
              <span className="subtitle">
                {page + 1} / {pageCount}
              </span>
              <button
                className="btn ghost small"
                disabled={page >= pageCount - 1}
                onClick={() => setPage((p) => p + 1)}
              >
                Succ &rarr;
              </button>
            </div>
          )}
        </div>
      </div>

      {preview && preview.warnings.length > 0 && (
        <div className="alert warn">
          <strong>Avvisi ({preview.warnings.length}):</strong>
          <ul>
            {preview.warnings.slice(0, 4).map((w) => (
              <li key={w}>{w}</li>
            ))}
          </ul>
        </div>
      )}

      {active && (
        <div className="row">
          <button className="btn primary" disabled={busy || !preview || preview.total_moves === 0} onClick={onApprove}>
            {busy ? "Sto spostando..." : "Approva"}
          </button>
          {canEdit && (
            <button className="btn ghost" disabled={busy} onClick={() => setEditing((v) => !v)}>
              {editing ? "Fine modifiche" : "Modifica"}
            </button>
          )}
          <button className="btn ghost danger" disabled={busy} onClick={onCancel}>
            Annulla
          </button>
        </div>
      )}
    </section>
  );
}
