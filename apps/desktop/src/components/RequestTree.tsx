// The collection as a tree of folders and requests, the same as on disk: renaming a request
// renames its file, dragging it moves the file, deleting sends it to the Trash.
import { useEffect, useMemo, useRef, useState } from "react";
import { CollectionInfo, RequestItem } from "../api";
import { ContextMenu, MenuItem } from "./ContextMenu";

export type Kind = "request" | "folder";

export interface TreeOps {
  /** Returns the new request's id, to rename it straight away. */
  newRequest: (folder: string) => Promise<string | null>;
  newFolder: (parent: string) => Promise<string | null>;
  rename: (kind: Kind, path: string, name: string) => Promise<void>;
  move: (kind: Kind, path: string, folder: string) => Promise<void>;
  duplicate: (id: string) => Promise<void>;
  remove: (kind: Kind, path: string) => Promise<void>;
  /** Runs the folder's requests, or the whole collection for "". */
  run: (folder: string) => void;
}

interface Row {
  kind: Kind;
  path: string;
  name: string;
  depth: number;
  item?: RequestItem;
}

const DRAG_TYPE = "application/x-tsubamedo";
const parentOf = (path: string) => (path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "");
const lastPart = (path: string) => path.split("/").pop() ?? path;

function loadCollapsed(root: string): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(`collapsed:${root}`) ?? "[]"));
  } catch {
    return new Set();
  }
}

export function RequestTree({
  collection,
  selected,
  filter,
  onSelect,
  ops,
  renameTarget,
}: {
  collection: CollectionInfo;
  selected: string | null;
  filter: string;
  onSelect: (id: string) => void;
  ops: TreeOps;
  renameTarget: { kind: Kind; path: string; n: number } | null;
}) {
  const [collapsed, setCollapsed] = useState(() => loadCollapsed(collection.root));
  const [renaming, setRenaming] = useState<{ kind: Kind; path: string } | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);

  useEffect(() => setCollapsed(loadCollapsed(collection.root)), [collection.root]);
  useEffect(() => {
    if (!renameTarget) return;
    // Its folders open, so the row to rename is on screen.
    setCollapsed((prev) => new Set([...prev].filter((p) => !renameTarget.path.startsWith(`${p}/`))));
    setRenaming({ kind: renameTarget.kind, path: renameTarget.path });
  }, [renameTarget]);
  useEffect(() => {
    try {
      localStorage.setItem(`collapsed:${collection.root}`, JSON.stringify([...collapsed]));
    } catch {
      // Only a convenience.
    }
  }, [collapsed, collection.root]);

  const rows = useMemo(() => {
    const f = filter.trim().toLowerCase();
    const matches = (r: RequestItem) => !f || r.name.toLowerCase().includes(f) || r.id.toLowerCase().includes(f);
    // With a filter, folders show only to lead to what matched, and all of them open.
    const wanted = new Set<string>();
    for (const r of collection.requests) {
      if (!matches(r)) continue;
      for (let p = r.folder; p; p = parentOf(p)) wanted.add(p);
    }
    const out: Row[] = [];
    const walk = (folder: string, depth: number) => {
      for (const sub of collection.folders.filter((x) => parentOf(x) === folder)) {
        if (f && !wanted.has(sub)) continue;
        out.push({ kind: "folder", path: sub, name: lastPart(sub), depth });
        if (f || !collapsed.has(sub)) walk(sub, depth + 1);
      }
      for (const r of collection.requests.filter((x) => x.folder === folder && matches(x))) {
        out.push({ kind: "request", path: r.id, name: r.name, depth, item: r });
      }
    };
    walk("", 0);
    return out;
  }, [collection, collapsed, filter]);

  const toggle = (path: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  const moveMenu = (kind: Kind, path: string): MenuItem => {
    const here = parentOf(path);
    const targets = [{ path: "", label: "Корень коллекции", indent: 0 }, ...collection.folders.map((f) => ({ path: f, label: lastPart(f), indent: f.split("/").length }))];
    return {
      label: "Переместить в…",
      submenu: targets
        .filter((t) => kind === "request" || !(t.path === path || t.path.startsWith(`${path}/`)))
        .map((t) => ({ label: t.label, indent: t.indent, disabled: t.path === here, onClick: () => ops.move(kind, path, t.path) })),
    };
  };

  const startNewRequest = (folder: string) => ops.newRequest(folder);
  const startNewFolder = (parent: string) => ops.newFolder(parent);

  const openMenu = (e: React.MouseEvent, row: Row | null) => {
    e.preventDefault();
    e.stopPropagation();
    let items: MenuItem[];
    if (!row) {
      items = [
        { label: "Новый запрос", onClick: () => startNewRequest("") },
        { label: "Новая папка", onClick: () => startNewFolder("") },
        { separator: true, label: "" },
        { label: "Запустить всю коллекцию", onClick: () => ops.run("") },
      ];
    } else if (row.kind === "request") {
      items = [
        { label: "Открыть", onClick: () => onSelect(row.path) },
        { label: "Переименовать", onClick: () => setRenaming({ kind: "request", path: row.path }) },
        { label: "Дублировать", onClick: () => ops.duplicate(row.path) },
        moveMenu("request", row.path),
        { separator: true, label: "" },
        { label: "Удалить в Корзину", danger: true, confirm: `Удалить «${row.name}»? Файл уйдёт в Корзину.`, onClick: () => ops.remove("request", row.path) },
      ];
    } else {
      const count = collection.requests.filter((r) => r.id.startsWith(`${row.path}/`)).length;
      items = [
        { label: "Запустить папку", disabled: count === 0, onClick: () => ops.run(row.path) },
        { separator: true, label: "" },
        { label: "Новый запрос здесь", onClick: () => startNewRequest(row.path) },
        { label: "Новая папка внутри", onClick: () => startNewFolder(row.path) },
        { label: "Переименовать", onClick: () => setRenaming({ kind: "folder", path: row.path }) },
        moveMenu("folder", row.path),
        { separator: true, label: "" },
        {
          label: "Удалить в Корзину",
          danger: true,
          confirm: count ? `Удалить папку «${row.name}» и ${count} запр. в ней? Она уйдёт в Корзину.` : `Удалить папку «${row.name}»?`,
          onClick: () => ops.remove("folder", row.path),
        },
      ];
    }
    setMenu({ x: e.clientX, y: e.clientY, items });
  };

  const drop = (e: React.DragEvent, folder: string) => {
    e.preventDefault();
    e.stopPropagation();
    setDropTarget(null);
    try {
      const { kind, path } = JSON.parse(e.dataTransfer.getData(DRAG_TYPE)) as { kind: Kind; path: string };
      if (parentOf(path) === folder || path === folder || folder.startsWith(`${path}/`)) return;
      ops.move(kind, path, folder);
    } catch {
      // Not ours (a file from Finder, say).
    }
  };

  const dropProps = (folder: string) => ({
    onDragOver: (e: React.DragEvent) => {
      if (!e.dataTransfer.types.includes(DRAG_TYPE)) return;
      e.preventDefault();
      e.stopPropagation();
      e.dataTransfer.dropEffect = "move";
      setDropTarget(folder);
    },
    onDragLeave: () => setDropTarget((t) => (t === folder ? null : t)),
    onDrop: (e: React.DragEvent) => drop(e, folder),
  });

  return (
    <div className={`tree-body ${dropTarget === "" ? "drop" : ""}`} onContextMenu={(e) => openMenu(e, null)} {...dropProps("")}>
      {rows.map((row) => {
        const isRenaming = renaming?.kind === row.kind && renaming.path === row.path;
        const common = {
          draggable: !isRenaming,
          onDragStart: (e: React.DragEvent) => {
            e.dataTransfer.setData(DRAG_TYPE, JSON.stringify({ kind: row.kind, path: row.path }));
            e.dataTransfer.effectAllowed = "move";
          },
          onDragEnd: () => setDropTarget(null),
          onContextMenu: (e: React.MouseEvent) => openMenu(e, row),
          style: { paddingLeft: 8 + row.depth * 14 },
          title: row.path,
        };
        const label = isRenaming ? (
          <RenameInput
            initial={row.kind === "request" ? row.name : lastPart(row.path)}
            onDone={(name) => {
              setRenaming(null);
              const old = row.kind === "request" ? row.name : lastPart(row.path);
              if (name && name !== old) ops.rename(row.kind, row.path, name);
            }}
          />
        ) : (
          <span className="ell">{row.name}</span>
        );
        if (row.kind === "folder") {
          const open = Boolean(filter.trim()) || !collapsed.has(row.path);
          return (
            <div
              key={`f:${row.path}`}
              className={`item folder-row ${dropTarget === row.path ? "drop" : ""}`}
              onClick={() => !isRenaming && toggle(row.path)}
              onDoubleClick={() => setRenaming({ kind: "folder", path: row.path })}
              {...common}
              {...dropProps(row.path)}
            >
              <span className="chev">{open ? "▾" : "▸"}</span>
              {label}
              <button className="more" title="Действия" onClick={(e) => openMenu(e, row)}>
                ⋯
              </button>
            </div>
          );
        }
        const method = row.item!.method;
        return (
          <div
            key={`r:${row.path}`}
            className={`item ${selected === row.path ? "on" : ""}`}
            onClick={() => !isRenaming && onSelect(row.path)}
            onDoubleClick={() => setRenaming({ kind: "request", path: row.path })}
            {...common}
            {...dropProps(parentOf(row.path))}
          >
            <span className={`m m-${method.toLowerCase()}`}>{method === "DELETE" ? "DEL" : method.slice(0, 4)}</span>
            {label}
            <button className="more" title="Действия" onClick={(e) => openMenu(e, row)}>
              ⋯
            </button>
          </div>
        );
      })}
      {rows.length === 0 && <div className="side-empty small faint">{filter ? "Ничего не нашлось" : "Пусто. Правый клик — новый запрос или папка."}</div>}
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
}

function RenameInput({ initial, onDone }: { initial: string; onDone: (name: string | null) => void }) {
  const [value, setValue] = useState(initial);
  // Enter or Escape finish it; the blur that follows must not finish it again.
  const done = useRef(false);
  const finish = (name: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(name);
  };
  return (
    <input
      className="rename"
      autoFocus
      value={value}
      spellCheck={false}
      onFocus={(e) => e.target.select()}
      onClick={(e) => e.stopPropagation()}
      onChange={(e) => setValue(e.target.value)}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") finish(value.trim() || null);
        if (e.key === "Escape") finish(null);
      }}
      onBlur={() => finish(value.trim() || null)}
    />
  );
}
