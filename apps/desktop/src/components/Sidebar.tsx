// The collection: requests grouped by folder, a filter, and the scratch request.
import { useMemo, useState } from "react";
import { CollectionInfo } from "../api";
import { home } from "../platform";

export function Sidebar({
  collection,
  selected,
  onSelect,
  onOpen,
  onCreate,
  onReload,
}: {
  collection: CollectionInfo | null;
  selected: string | null;
  onSelect: (id: string | null) => void;
  onOpen: () => void;
  onCreate: () => void;
  onReload: () => void;
}) {
  const [filter, setFilter] = useState("");
  const groups = useMemo(() => {
    const f = filter.trim().toLowerCase();
    const map = new Map<string, CollectionInfo["requests"]>();
    for (const r of collection?.requests ?? []) {
      if (f && !r.name.toLowerCase().includes(f) && !r.id.toLowerCase().includes(f)) continue;
      map.set(r.folder, [...(map.get(r.folder) ?? []), r]);
    }
    return [...map.entries()];
  }, [collection, filter]);

  return (
    <aside className="sidebar">
      <div className="side-head">
        <div className="ell">
          <div className="coll-name ell">{collection?.name ?? "Без коллекции"}</div>
          {collection && (
            <div className="faint small ell" title={collection.root}>
              {home(collection.root)}
            </div>
          )}
        </div>
        <span className="sp" />
        {collection && (
          <>
            <button className="icon" title="Новый запрос (⌘N)" onClick={onCreate}>
              +
            </button>
            <button className="icon" title="Перечитать файлы" onClick={onReload}>
              ↻
            </button>
          </>
        )}
        <button className="icon" title="Открыть коллекцию (⌘O)" onClick={onOpen}>
          ⌂
        </button>
      </div>
      {collection && collection.requests.length > 8 && (
        <input className="filter" placeholder="Найти запрос" value={filter} spellCheck={false} onChange={(e) => setFilter(e.target.value)} />
      )}
      <nav className="tree">
        <button className={`item ${selected === null ? "on" : ""}`} onClick={() => onSelect(null)}>
          <span className="m faint">✎</span>
          <span className="ell">Черновик</span>
        </button>
        {groups.map(([folder, items]) => (
          <div key={folder}>
            {folder && <div className="folder ell">{folder}</div>}
            {items.map((r) => (
              <button key={r.id} className={`item ${selected === r.id ? "on" : ""} ${folder ? "nested" : ""}`} onClick={() => onSelect(r.id)} title={r.id}>
                <span className={`m m-${r.method.toLowerCase()}`}>{r.method === "DELETE" ? "DEL" : r.method.slice(0, 4)}</span>
                <span className="ell">{r.name}</span>
              </button>
            ))}
          </div>
        ))}
        {!collection && (
          <div className="side-empty small muted">
            <p>Коллекция — это папка с файлом tsubamedo.toml: запросы лежат в ней обычными файлами и живут в git.</p>
            <button onClick={onOpen}>Открыть папку…</button>
            <p className="faint">
              Новая коллекция: <code>tsubamedo init</code>
            </p>
          </div>
        )}
        {collection?.broken.map((b) => (
          <div key={b} className="broken small" title={b}>
            ⚠ {b}
          </div>
        ))}
      </nav>
    </aside>
  );
}
