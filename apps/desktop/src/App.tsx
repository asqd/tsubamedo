// The window: collection on the left, tabs of open requests, each with its request and answer.
// Edits are saved to the request's file as you type; the core rewrites only what changed.
import { useCallback, useEffect, useRef, useState } from "react";
import { api, CollectionInfo, emptyRequest, ImportPreview, Reorganized, Request } from "./api";
import { CookiesDialog } from "./components/CookiesDialog";
import { ImportDialog, ImportTarget } from "./components/ImportDialog";
import { Logo } from "./components/Logo";
import { RequestEditor } from "./components/RequestEditor";
import { failedIds, RunPanel, RunSettings, RunState } from "./components/RunPanel";
import { ResponsePane } from "./components/ResponsePane";
import { Kind, TreeOps } from "./components/RequestTree";
import { Sidebar } from "./components/Sidebar";
import { TabBar } from "./components/TabBar";
import { Tab } from "./tabs";
import { mod, pickFolder, pickImportFile, pickParentFolder } from "./platform";
import { VarInfo, VarsContext } from "./vars";

interface Notice {
  text: string;
  undo?: () => void;
}

const store = {
  get(key: string): string | null {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  },
  set(key: string, value: string | null) {
    try {
      if (value == null) localStorage.removeItem(key);
      else localStorage.setItem(key, value);
    } catch {
      // Storage may be unavailable; these are conveniences only.
    }
  },
};

const newKey = () => crypto.randomUUID().slice(0, 8);

function scratchTab(key = newKey()): Tab {
  let request = emptyRequest();
  try {
    request = { ...request, ...JSON.parse(store.get(`scratch:${key}`) ?? "{}") };
  } catch {
    // A broken saved scratch request starts empty.
  }
  return { key, id: null, request, response: null, error: null, sendId: null, pristine: false };
}

export default function App() {
  const [collection, setCollection] = useState<CollectionInfo | null>(null);
  const [env, setEnv] = useState<string | null>(null);
  const [vars, setVars] = useState<VarInfo[]>([]);
  const [tabs, setTabs] = useState<Tab[]>(() => [scratchTab()]);
  const [active, setActive] = useState<string>(() => tabs[0].key);
  const [problem, setProblem] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [saveState, setSaveState] = useState<"idle" | "saving" | "saved">("idle");
  const [split, setSplit] = useState(() => Number(store.get("split")) || 0.46);
  const [historyVersion, setHistoryVersion] = useState(0);
  const [renameTarget, setRenameTarget] = useState<{ kind: Kind; path: string; n: number } | null>(null);
  const [run, setRun] = useState<RunState | null>(null);
  const [runSettings, setRunSettings] = useState<RunSettings>(() => {
    try {
      return { parallel: 1, retries: 0, ...JSON.parse(store.get("run") ?? "{}") };
    } catch {
      return { parallel: 1, retries: 0 };
    }
  });
  const [cookiesOpen, setCookiesOpen] = useState(false);
  const [importing, setImporting] = useState<{ path: string; preview: ImportPreview; busy: boolean } | null>(null);

  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const pendingSave = useRef<{ id: string; request: Request; timer: number } | null>(null);
  const tab = tabs.find((t) => t.key === active) ?? tabs[0];
  const root = collection?.root ?? "";

  const patchTab = useCallback((key: string, patch: Partial<Tab>) => {
    setTabs((all) => all.map((t) => (t.key === key ? { ...t, ...patch } : t)));
  }, []);

  const flash = useCallback((n: Notice) => {
    setNotice(n);
    window.setTimeout(() => setNotice((cur) => (cur === n ? null : cur)), n.undo ? 8000 : 3000);
  }, []);

  const flushSave = useCallback(async () => {
    const p = pendingSave.current;
    if (!p) return;
    pendingSave.current = null;
    clearTimeout(p.timer);
    setSaveState("saving");
    try {
      await api.saveRequest(p.id, p.request);
      setSaveState("saved");
      setCollection((c) =>
        c && {
          ...c,
          requests: c.requests.map((r) =>
            r.id === p.id ? { ...r, method: p.request.method.toUpperCase(), name: p.request.name || r.id.split("/").pop()! } : r,
          ),
        },
      );
    } catch (e) {
      setProblem(`Не удалось сохранить: ${(e as Error).message}`);
      setSaveState("idle");
    }
  }, []);

  // Open tabs are remembered per collection; not while they are being restored, or the
  // half-built list would overwrite the saved one.
  const restoring = useRef(true);
  useEffect(() => {
    if (restoring.current) return;
    store.set(`tabs:${root}`, JSON.stringify(tabs.map((t) => ({ key: t.key, id: t.id }))));
    store.set(`active:${root}`, active);
  }, [tabs, active, root]);

  // The core keeps a response's body and views only while a tab shows it, so a closed tab
  // with a 300 MB answer gives its memory back. Responses newer than any a tab has had are
  // still on their way to one and stay.
  const shown = tabs.flatMap((t) => (t.response ? [t.response.responseId] : []));
  const seenResponse = useRef(0);
  useEffect(() => {
    seenResponse.current = Math.max(seenResponse.current, ...shown);
    if (seenResponse.current > 0) api.retain(shown, seenResponse.current).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shown.join(",")]);

  const restoreTabs = useCallback(async (info: CollectionInfo | null) => {
    restoring.current = true;
    const key = info?.root ?? "";
    const last = store.get(`active:${key}`);
    let saved: { key: string; id: string | null }[] = [];
    try {
      saved = JSON.parse(store.get(`tabs:${key}`) ?? "[]");
    } catch {
      saved = [];
    }
    const restored: Tab[] = [];
    for (const s of saved) {
      if (s.id === null) restored.push(scratchTab(s.key));
      else if (info?.requests.some((r) => r.id === s.id)) {
        try {
          const request = await api.getRequest(s.id);
          restored.push({ key: s.key, id: s.id, request, response: null, error: null, sendId: null, pristine: true });
        } catch {
          // The file is gone or broken; its tab just does not come back.
        }
      }
    }
    if (restored.length === 0) restored.push(scratchTab());
    setTabs(restored);
    setActive(restored.some((t) => t.key === last) ? last! : restored[0].key);
    restoring.current = false;
  }, []);

  const openCollection = useCallback(
    async (path: string) => {
      await flushSave();
      try {
        const info = await api.openCollection(path);
        setCollection(info);
        setProblem(null);
        store.set("collection", info.root);
        const savedEnv = store.get(`env:${info.root}`);
        setEnv(savedEnv && info.environments.includes(savedEnv) ? savedEnv : (info.environments[0] ?? null));
        await restoreTabs(info);
      } catch (e) {
        setProblem((e as Error).message);
        restoring.current = false;
      }
    },
    [flushSave, restoreTabs],
  );

  useEffect(() => {
    const last = store.get("collection");
    if (last) openCollection(last);
    else restoreTabs(null);
  }, [openCollection, restoreTabs]);

  // Variables of the environment, for coloring and suggestions in the editors.
  const loadVars = useCallback(() => {
    if (!collection) return setVars([]);
    api
      .vars(env)
      .then(setVars)
      .catch((e) => {
        setVars([]);
        setProblem((e as Error).message);
      });
  }, [collection, env]);
  useEffect(loadVars, [loadVars]);

  const activate = useCallback(
    async (key: string) => {
      await flushSave();
      setSaveState("idle");
      setActive(key);
    },
    [flushSave],
  );

  const openRequest = useCallback(
    async (id: string | null) => {
      await flushSave();
      setSaveState("idle");
      if (id === null) return newScratch();
      const open = tabsRef.current.find((t) => t.id === id);
      if (open) return setActive(open.key);
      try {
        const request = await api.getRequest(id);
        const fresh: Tab = { key: newKey(), id, request, response: null, error: null, sendId: null, pristine: true };
        setTabs((all) => {
          const i = all.findIndex((t) => t.key === active);
          const current = all[i];
          // A tab only looked at gives way, so browsing the sidebar does not pile up tabs.
          if (current && current.pristine && current.id && !current.sendId) return all.map((t, j) => (j === i ? fresh : t));
          return [...all.slice(0, i + 1), fresh, ...all.slice(i + 1)];
        });
        setActive(fresh.key);
      } catch (e) {
        setProblem((e as Error).message);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [flushSave, active],
  );

  const newScratch = () => {
    const fresh = scratchTab();
    setTabs((all) => {
      const i = all.findIndex((t) => t.key === active);
      return [...all.slice(0, i + 1), fresh, ...all.slice(i + 1)];
    });
    setActive(fresh.key);
  };

  /** Closes tabs; the window always keeps at least one. */
  const closeTabs = (keys: string[]) => {
    if (keys.length === 0) return;
    const all = tabsRef.current;
    for (const t of all.filter((t) => keys.includes(t.key))) {
      if (t.sendId) api.cancel(t.sendId);
      if (t.id === null) store.set(`scratch:${t.key}`, null);
    }
    const first = all.findIndex((t) => keys.includes(t.key));
    const rest = all.filter((t) => !keys.includes(t.key));
    if (rest.length === 0) rest.push(scratchTab());
    setTabs(rest);
    if (keys.includes(active)) setActive(rest[Math.min(first, rest.length - 1)].key);
  };

  const closeTab = async (key: string) => {
    await flushSave();
    closeTabs([key]);
  };

  const change = (next: Request) => {
    patchTab(tab.key, { request: next, pristine: false });
    if (tab.id === null) {
      store.set(`scratch:${tab.key}`, JSON.stringify(next));
      return;
    }
    if (pendingSave.current && pendingSave.current.id !== tab.id) flushSave();
    if (pendingSave.current) clearTimeout(pendingSave.current.timer);
    pendingSave.current = { id: tab.id, request: next, timer: window.setTimeout(flushSave, 400) };
    setSaveState("saving");
  };

  const send = useCallback(async () => {
    const current = tabsRef.current.find((t) => t.key === active);
    if (!current || current.sendId || !current.request.url.trim()) return;
    await flushSave();
    const sendId = crypto.randomUUID();
    patchTab(current.key, { sendId, error: null, pristine: false });
    try {
      const meta = await api.send(sendId, current.id, current.request, env, false);
      patchTab(current.key, { response: meta, sendId: null });
    } catch (e) {
      const message = (e as Error).message;
      patchTab(current.key, { sendId: null, error: message === "Отменено" ? null : message });
    }
    setHistoryVersion((v) => v + 1);
    // Values captured from the response (or those sent first) are variables now.
    loadVars();
  }, [active, flushSave, patchTab, env, loadVars]);

  /** Runs the collection (`folder` empty), a folder, or `only` these requests, in the panel
   * over the editors; the panel follows the run as it goes. */
  const startRun = useCallback(
    async (folder: string, only: string[] | null = null) => {
      await flushSave();
      const sendId = crypto.randomUUID();
      setRun({ folder, only, sendId, steps: null, running: {}, results: [], finished: false, totalMs: null, error: null });
      const update = (patch: Partial<RunState>) => setRun((cur) => (cur?.sendId === sendId ? { ...cur, ...patch } : cur));
      let over = false;
      const poll = window.setInterval(async () => {
        const progress = await api.runProgress(sendId).catch(() => null);
        if (progress && !over) update({ steps: progress.steps, running: progress.running, results: progress.results });
      }, 250);
      try {
        const { steps, results, totalMs } = await api.runCollection(sendId, folder, env, runSettings.parallel, runSettings.retries, only);
        over = true;
        update({ steps, results, running: {}, finished: true, totalMs });
      } catch (e) {
        over = true;
        update({ running: {}, finished: true, error: (e as Error).message });
      } finally {
        window.clearInterval(poll);
      }
      setHistoryVersion((v) => v + 1);
      loadVars();
    },
    [flushSave, env, loadVars, runSettings],
  );

  const pasteCurl = (text: string): boolean => {
    if (!/^\s*curl\s/.test(text)) return false;
    const before = tab.request;
    api
      .importCurl(text)
      .then((imported) => {
        const next = { ...imported, name: before.name };
        change(next);
        flash({ text: "Запрос взят из команды curl", undo: () => change(before) });
      })
      .catch((e) => setProblem(`curl: ${(e as Error).message}`));
    return true;
  };

  const copyCurl = async () => {
    try {
      const command = await api.curl(tab.id, tab.request, env);
      await navigator.clipboard.writeText(command);
      flash({ text: "Команда curl скопирована" });
    } catch (e) {
      setProblem((e as Error).message);
    }
  };

  const open = async () => {
    const path = await pickFolder();
    if (path) openCollection(path);
  };

  const startImport = async () => {
    const path = await pickImportFile();
    if (!path) return;
    try {
      setImporting({ path, preview: await api.importPreview(path), busy: false });
    } catch (e) {
      setProblem((e as Error).message);
    }
  };

  const finishImport = async (target: ImportTarget) => {
    if (!importing) return;
    const parent = target.kind === "new" ? await pickParentFolder() : null;
    if (target.kind === "new" && !parent) return;
    await flushSave();
    setImporting({ ...importing, busy: true });
    try {
      const { written, collection: info } = await api.importCollection(importing.path, parent);
      setImporting(null);
      if (target.kind === "new") {
        await openCollection(written.root);
      } else {
        setCollection(info);
        if (!env && info.environments.length > 0) setEnv(info.environments[0]);
      }
      const n = written.requests;
      const what =
        target.kind === "new"
          ? `Создана коллекция «${info.name}»: запросов ${n}`
          : n > 0
            ? `Импортировано в папку «${written.folder}»: запросов ${n}`
            : `Добавлены окружения: ${written.environments.join(", ") || "—"}`;
      const kept = written.kept.length ? ` · уже были, оставлены: ${written.kept.join(", ")}` : "";
      flash({ text: what + kept });
    } catch (e) {
      setImporting((cur) => cur && { ...cur, busy: false });
      setProblem((e as Error).message);
    }
  };

  /** After a file operation: the new collection, open tabs following their files. */
  const applyReorganized = async (result: Reorganized, remap: (id: string) => string | null) => {
    setCollection(result.collection);
    const gone: string[] = [];
    for (const t of tabsRef.current) {
      if (!t.id) continue;
      const next = remap(t.id);
      if (next === null) gone.push(t.key);
      else if (next !== t.id || next === result.id) {
        const request = await api.getRequest(next).catch(() => t.request);
        patchTab(t.key, { id: next, request });
      }
    }
    closeTabs(gone);
  };

  const exact = (old: string, next: string | null) => (id: string) => (id === old ? next : id);
  const under = (old: string, next: string | null) => (id: string) => (id.startsWith(`${old}/`) ? (next === null ? null : next + id.slice(old.length)) : id);

  const openNew = async (id: string) => {
    const request = await api.getRequest(id);
    const fresh: Tab = { key: newKey(), id, request, response: null, error: null, sendId: null, pristine: false };
    setTabs((all) => [...all, fresh]);
    setActive(fresh.key);
  };

  /** Runs a file operation with saving flushed first and errors shown. */
  const guarded = async <T,>(op: () => Promise<T>): Promise<T | null> => {
    await flushSave();
    try {
      return await op();
    } catch (e) {
      setProblem((e as Error).message);
      return null;
    }
  };

  const ops: TreeOps = {
    run: (folder) => startRun(folder),
    newRequest: async (folder) =>
      guarded(async () => {
        const { id, collection: info } = await api.createRequest(folder, "Новый запрос");
        setCollection(info);
        await openNew(id);
        setRenameTarget({ kind: "request", path: id, n: Date.now() });
        return id;
      }),
    newFolder: async (parent) =>
      guarded(async () => {
        const result = await api.createFolder(parent, "Новая папка");
        setCollection(result.collection);
        setRenameTarget({ kind: "folder", path: result.id, n: Date.now() });
        return result.id;
      }),
    rename: async (kind, path, name) => {
      await guarded(async () => {
        const result = kind === "request" ? await api.renameRequest(path, name) : await api.renameFolder(path, name);
        await applyReorganized(result, kind === "request" ? exact(path, result.id) : under(path, result.id));
      });
    },
    move: async (kind, path, folder) => {
      await guarded(async () => {
        const result = kind === "request" ? await api.moveRequest(path, folder) : await api.moveFolder(path, folder);
        await applyReorganized(result, kind === "request" ? exact(path, result.id) : under(path, result.id));
      });
    },
    duplicate: async (id) => {
      await guarded(async () => {
        const result = await api.duplicateRequest(id);
        setCollection(result.collection);
        await openNew(result.id);
      });
    },
    remove: async (kind, path) => {
      await guarded(async () => {
        if (pendingSave.current?.id === path) pendingSave.current = null;
        const result = kind === "request" ? await api.deleteRequest(path) : await api.deleteFolder(path);
        await applyReorganized(result, kind === "request" ? exact(path, null) : under(path, null));
        flash({ text: kind === "request" ? "Запрос перемещён в Корзину" : "Папка перемещена в Корзину" });
      });
    },
  };

  const create = async () => {
    if (!collection) return newScratch();
    const folder = tab.id?.includes("/") ? tab.id.slice(0, tab.id.lastIndexOf("/")) : "";
    await ops.newRequest(folder);
  };

  /** An old exchange as a scratch tab: the request as it was, the answer if it was kept. */
  const openHistory = async (entryId: string) => {
    await flushSave();
    try {
      const entry = await api.openHistory(entryId);
      const when = new Date(entry.at).toLocaleString("ru-RU", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
      const title = entry.request.name ?? entry.requestId?.split("/").pop() ?? entry.request.url;
      const fresh = scratchTab();
      fresh.request = { ...entry.request, name: `${title} · ${when}` };
      fresh.response = entry.response;
      fresh.error = entry.response
        ? null
        : entry.error ?? `Ответ ${entry.status ?? ""} пришёл ${when}, но его тело больше 1 МБ и не сохранялось. Отправьте запрос ещё раз.`;
      store.set(`scratch:${fresh.key}`, JSON.stringify(fresh.request));
      setTabs((all) => [...all, fresh]);
      setActive(fresh.key);
    } catch (e) {
      setProblem((e as Error).message);
    }
  };

  const reload = async () => {
    await flushSave();
    try {
      const info = await api.reloadCollection();
      setCollection(info);
      loadVars();
      for (const t of tabsRef.current) {
        if (t.id && info.requests.some((r) => r.id === t.id)) patchTab(t.key, { request: await api.getRequest(t.id) });
      }
    } catch (e) {
      setProblem((e as Error).message);
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === "Tab") {
        e.preventDefault();
        const i = tabs.findIndex((t) => t.key === active);
        activate(tabs[(i + (e.shiftKey ? tabs.length - 1 : 1)) % tabs.length].key);
        return;
      }
      if (!mod(e)) return;
      const k = e.key.toLowerCase();
      if (e.key === "Enter") {
        e.preventDefault();
        send();
      } else if ((k === "i" || k === "ш") && e.shiftKey) {
        e.preventDefault();
        startImport();
      } else if (k === "o" || k === "щ") {
        e.preventDefault();
        open();
      } else if (k === "n" || k === "т") {
        e.preventDefault();
        create();
      } else if (k === "t" || k === "е") {
        e.preventDefault();
        newScratch();
      } else if (k === "w" || k === "ц") {
        e.preventDefault();
        closeTab(active);
      } else if (k === "s" || k === "ы") {
        // Saving is automatic; ⌘S just makes it immediate.
        e.preventDefault();
        flushSave();
      } else if (/^[1-9]$/.test(e.key)) {
        e.preventDefault();
        const target = e.key === "9" ? tabs[tabs.length - 1] : tabs[Number(e.key) - 1];
        if (target) activate(target.key);
      } else if (e.shiftKey && (e.key === "[" || e.key === "]" || e.key === "{" || e.key === "}")) {
        e.preventDefault();
        const i = tabs.findIndex((t) => t.key === active);
        const step = e.key === "[" || e.key === "{" ? tabs.length - 1 : 1;
        activate(tabs[(i + step) % tabs.length].key);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  useEffect(() => {
    const save = () => flushSave();
    window.addEventListener("beforeunload", save);
    return () => window.removeEventListener("beforeunload", save);
  }, [flushSave]);

  const dragSplit = (e: React.PointerEvent) => {
    const area = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect();
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => setSplit(Math.min(0.75, Math.max(0.25, (ev.clientX - area.left) / area.width)));
    const up = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      setSplit((s) => {
        store.set("split", String(s));
        return s;
      });
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
  };

  return (
    <VarsContext.Provider value={vars}>
      <div className="app">
        <header className="top" data-tauri-drag-region>
          <Logo />
          <span className="brand" data-tauri-drag-region>
            TsubameDo
          </span>
          <span className="sp" data-tauri-drag-region />
          {collection && collection.environments.length > 0 && (
            <label className="env" title="Окружение: переменные из папки env">
              <span className="faint small">Окружение</span>
              <select
                value={env ?? ""}
                onChange={(e) => {
                  const v = e.target.value || null;
                  setEnv(v);
                  store.set(`env:${collection.root}`, v);
                }}
              >
                <option value="">без окружения</option>
                {collection.environments.map((name) => (
                  <option key={name}>{name}</option>
                ))}
              </select>
            </label>
          )}
          <button className="ghost small" onClick={() => setCookiesOpen(true)} title="Cookie, которые поставили серверы, — у каждого окружения свои">
            Cookie
          </button>
          <span className="faint small save-state">{saveState === "saving" ? "сохраняю…" : saveState === "saved" ? "сохранено" : ""}</span>
        </header>
        {problem && (
          <div className="problem small">
            {problem}
            <button className="icon" onClick={() => setProblem(null)}>
              ×
            </button>
          </div>
        )}
        <div className="main">
          <Sidebar
            collection={collection}
            selected={tab.id}
            onSelect={openRequest}
            onOpen={open}
            onImport={startImport}
            onReload={reload}
            ops={ops}
            renameTarget={renameTarget}
            historyVersion={historyVersion}
            onOpenHistory={openHistory}
          />
          <div className="workspace">
            <TabBar tabs={tabs} active={tab.key} onSelect={activate} onClose={closeTab} onNew={newScratch} />
            <div className="work">
              {run && (
                <RunPanel
                  run={run}
                  env={env}
                  onOpen={(id) => {
                    setRun(null);
                    openRequest(id);
                  }}
                  settings={runSettings}
                  onSettings={(next) => {
                    setRunSettings(next);
                    store.set("run", JSON.stringify(next));
                  }}
                  onAgain={() => startRun(run.folder, run.only)}
                  onRetryFailed={() => startRun(run.folder, failedIds(run))}
                  onCancel={() => api.cancel(run.sendId)}
                  onClose={() => {
                    if (!run.finished) api.cancel(run.sendId);
                    setRun(null);
                  }}
                />
              )}
              <section className="left" style={{ width: `${split * 100}%` }}>
                <RequestEditor
                  key={tab.key}
                  request={tab.request}
                  onChange={change}
                  onSend={send}
                  sending={Boolean(tab.sendId)}
                  saved={tab.id}
                  onPasteCurl={pasteCurl}
                  onCopyCurl={copyCurl}
                  env={env}
                />
              </section>
              <div className="splitter" onPointerDown={dragSplit} />
              <section className="right">
                <ResponsePane
                  meta={tab.response}
                  error={tab.error}
                  sending={Boolean(tab.sendId)}
                  onCancel={() => tab.sendId && api.cancel(tab.sendId)}
                />
              </section>
            </div>
          </div>
        </div>
        {notice && (
          <div className="notice small">
            {notice.text}
            {notice.undo && (
              <button
                className="ghost"
                onClick={() => {
                  notice.undo!();
                  setNotice(null);
                }}
              >
                Вернуть как было
              </button>
            )}
          </div>
        )}
        {cookiesOpen && <CookiesDialog env={env} onClose={() => setCookiesOpen(false)} />}
        {importing && (
          <ImportDialog
            path={importing.path}
            preview={importing.preview}
            collection={collection}
            busy={importing.busy}
            onImport={finishImport}
            onCancel={() => setImporting(null)}
          />
        )}
      </div>
    </VarsContext.Provider>
  );
}
