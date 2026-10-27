// The left panel: the collection as a tree, or the history of sent requests.
import { useEffect, useState } from "react";
import { CollectionInfo } from "../api";
import { home } from "../platform";
import { HistoryList } from "./HistoryList";
import { Kind, RequestTree, TreeOps } from "./RequestTree";

type Mode = "requests" | "history";

export function Sidebar({
  collection,
  selected,
  onSelect,
  onOpen,
  onReload,
  ops,
  renameTarget,
  historyVersion,
  onOpenHistory,
}: {
  collection: CollectionInfo | null;
  selected: string | null;
  onSelect: (id: string | null) => void;
  onOpen: () => void;
  onReload: () => void;
  ops: TreeOps;
  /** Something just created, to rename in place. */
  renameTarget: { kind: Kind; path: string; n: number } | null;
  historyVersion: number;
  onOpenHistory: (entryId: string) => void;
}) {
  const [mode, setMode] = useState<Mode>("requests");
  // Whatever was just created is renamed in the tree, so the tree must be showing.
  useEffect(() => {
    if (renameTarget) setMode("requests");
  }, [renameTarget]);
  const [filter, setFilter] = useState("");

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
            <button className="icon" title="Новый запрос (⌘N)" onClick={() => ops.newRequest(selected?.includes("/") ? selected.slice(0, selected.lastIndexOf("/")) : "")}>
              +
            </button>
            <button className="icon" title="Запустить всю коллекцию: запросы по очереди с проверками" disabled={collection.requests.length === 0} onClick={() => ops.run("")}>
              ▶
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
      <div className="side-mode">
        <div className="seg">
          <button className={mode === "requests" ? "on" : ""} onClick={() => setMode("requests")}>
            Запросы
          </button>
          <button className={mode === "history" ? "on" : ""} onClick={() => setMode("history")}>
            История
          </button>
        </div>
      </div>
      <input className="filter" placeholder={mode === "requests" ? "Найти запрос" : "Найти в истории"} value={filter} spellCheck={false} onChange={(e) => setFilter(e.target.value)} />
      <nav className="tree">
        {mode === "history" ? (
          <HistoryList version={historyVersion} currentId={selected} filter={filter} onOpen={onOpenHistory} />
        ) : (
          <>
            <button className={`item scratch ${selected === null ? "on" : ""}`} onClick={() => onSelect(null)} title="Новый черновик (⌘T)">
              <span className="m faint">✎</span>
              <span className="ell">Черновик</span>
            </button>
            {collection ? (
              <RequestTree collection={collection} selected={selected} filter={filter} onSelect={onSelect} ops={ops} renameTarget={renameTarget} />
            ) : (
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
          </>
        )}
      </nav>
    </aside>
  );
}
