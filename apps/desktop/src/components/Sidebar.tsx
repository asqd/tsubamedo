// The left panel: the collection as a tree, the history of sent requests, or what changed in
// git.
import { useEffect, useMemo, useState } from "react";
import { ChangeKind, CollectionInfo, GitState } from "../api";
import { t } from "../i18n";
import { home } from "../platform";
import { GitActions, GitPanel } from "./GitPanel";
import { HistoryList } from "./HistoryList";
import { Kind, RequestTree, TreeOps } from "./RequestTree";

type Mode = "requests" | "history" | "changes";

export function Sidebar({
  collection,
  selected,
  onSelect,
  onOpen,
  onImport,
  onReload,
  ops,
  renameTarget,
  historyVersion,
  onOpenHistory,
  git,
  gitActions,
}: {
  collection: CollectionInfo | null;
  selected: string | null;
  onSelect: (id: string | null) => void;
  onOpen: () => void;
  onImport: () => void;
  onReload: () => void;
  ops: TreeOps;
  /** Something just created, to rename in place. */
  renameTarget: { kind: Kind; path: string; n: number } | null;
  historyVersion: number;
  onOpenHistory: (entryId: string) => void;
  git: GitState | null;
  gitActions: GitActions;
}) {
  const [mode, setMode] = useState<Mode>("requests");
  // Whatever was just created is renamed in the tree, so the tree must be showing.
  useEffect(() => {
    if (renameTarget) setMode("requests");
  }, [renameTarget]);
  const [filter, setFilter] = useState("");
  const changes = git?.git === "repo" ? git.changes : [];
  // What changed, by request id and by folder, for the marks in the tree.
  const marks = useMemo(() => {
    const out = new Map<string, ChangeKind>();
    for (const c of changes) {
      if (!c.path.endsWith(".toml")) continue;
      out.set(c.path.slice(0, -5), c.kind);
      for (let p = c.path; p.includes("/"); ) {
        p = p.slice(0, p.lastIndexOf("/"));
        if (!out.has(p)) out.set(p, "modified");
      }
    }
    return out;
  }, [changes]);

  return (
    <aside className="sidebar">
      <div className="side-head">
        <div className="ell">
          <div className="coll-name ell">{collection?.name ?? t("Без коллекции", "No collection")}</div>
          {collection && (
            <div className="faint small ell" title={collection.root}>
              {home(collection.root)}
            </div>
          )}
        </div>
        <span className="sp" />
        {collection && (
          <>
            <button className="icon" title={t("Новый запрос (⌘N)", "New request (⌘N)")} onClick={() => ops.newRequest(selected?.includes("/") ? selected.slice(0, selected.lastIndexOf("/")) : "")}>
              +
            </button>
            <button className="icon" title={t("Запустить всю коллекцию: запросы по очереди с проверками", "Run the whole collection: requests one by one, with checks")} disabled={collection.requests.length === 0} onClick={() => ops.run("")}>
              ▶
            </button>
            <button className="icon" title={t("Перечитать файлы", "Reload files")} onClick={onReload}>
              ↻
            </button>
          </>
        )}
        <button className="icon" title={t("Импорт из Insomnia, Postman, OpenAPI или HAR (⇧⌘I)", "Import from Insomnia, Postman, OpenAPI or HAR (⇧⌘I)")} onClick={onImport}>
          ⇣
        </button>
        <button className="icon" title={t("Открыть коллекцию (⌘O)", "Open collection (⌘O)")} onClick={onOpen}>
          ⌂
        </button>
      </div>
      <div className="side-mode">
        <div className="seg">
          <button className={mode === "requests" ? "on" : ""} onClick={() => setMode("requests")}>
            {t("Запросы", "Requests")}
          </button>
          <button className={mode === "history" ? "on" : ""} onClick={() => setMode("history")}>
            {t("История", "History")}
          </button>
          {collection && (
            <button className={mode === "changes" ? "on" : ""} onClick={() => setMode("changes")} title={t("Git: что изменилось, коммит, получить и отправить", "Git: what changed, commit, pull and push")}>
              Git{changes.length ? ` ${changes.length}` : ""}
            </button>
          )}
        </div>
      </div>
      {mode === "changes" && collection ? (
        <nav className="tree">
          <GitPanel collection={collection} git={git} act={gitActions} onOpenRequest={(id) => (setMode("requests"), onSelect(id))} />
        </nav>
      ) : (
        <>
      <input className="filter" placeholder={mode === "requests" ? t("Найти запрос", "Find request") : t("Найти в истории", "Search history")} value={filter} spellCheck={false} onChange={(e) => setFilter(e.target.value)} />
      <nav className="tree">
        {mode === "history" ? (
          <HistoryList version={historyVersion} currentId={selected} filter={filter} onOpen={onOpenHistory} />
        ) : (
          <>
            <button className={`item scratch ${selected === null ? "on" : ""}`} onClick={() => onSelect(null)} title={t("Новый черновик (⌘T)", "New draft (⌘T)")}>
              <span className="m faint">✎</span>
              <span className="ell">{t("Черновик", "Draft")}</span>
            </button>
            {collection ? (
              <RequestTree collection={collection} selected={selected} filter={filter} onSelect={onSelect} ops={ops} renameTarget={renameTarget} marks={marks} />
            ) : (
              <div className="side-empty small muted">
                <p>
                  {t(
                    "Коллекция — это папка с файлом tsubamedo.toml: запросы лежат в ней обычными файлами и живут в git.",
                    "A collection is a folder with a tsubamedo.toml file: requests are plain files in it and live in git.",
                  )}
                </p>
                <div className="side-empty-actions">
                  <button onClick={onOpen}>{t("Открыть папку…", "Open folder…")}</button>
                  <button onClick={onImport} title={t("Экспорт Insomnia, коллекция Postman, OpenAPI, HAR", "Insomnia export, Postman collection, OpenAPI, HAR")}>
                    {t("Импорт…", "Import…")}
                  </button>
                </div>
                <p className="faint">
                  {t("Новая коллекция", "New collection")}: <code>tsubamedo init</code>
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
        </>
      )}
    </aside>
  );
}
