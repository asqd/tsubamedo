// The window: collection on the left, tabs of open requests, each with its request and answer.
// Edits are saved to the request's file as you type; the core rewrites only what changed.
import { useCallback, useEffect, useRef, useState } from "react";
import { api, CollectionInfo, emptyRequest, Request } from "./api";
import { Logo } from "./components/Logo";
import { RequestEditor } from "./components/RequestEditor";
import { ResponsePane } from "./components/ResponsePane";
import { Sidebar } from "./components/Sidebar";
import { TabBar } from "./components/TabBar";
import { Tab } from "./tabs";
import { mod, pickFolder } from "./platform";
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

  const closeTab = async (key: string) => {
    await flushSave();
    const all = tabsRef.current;
    const i = all.findIndex((t) => t.key === key);
    const closing = all[i];
    if (!closing) return;
    if (closing.sendId) api.cancel(closing.sendId);
    if (closing.id === null) store.set(`scratch:${key}`, null);
    const rest = all.filter((t) => t.key !== key);
    if (rest.length === 0) rest.push(scratchTab());
    setTabs(rest);
    if (key === active) setActive(rest[Math.min(i, rest.length - 1)].key);
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
  }, [active, flushSave, patchTab, env]);

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

  const create = async () => {
    if (!collection) return newScratch();
    await flushSave();
    const folder = tab.id?.includes("/") ? tab.id.slice(0, tab.id.lastIndexOf("/")) : "";
    try {
      const { id, collection: info } = await api.createRequest(folder, "Новый запрос");
      setCollection(info);
      const request = await api.getRequest(id);
      const fresh: Tab = { key: newKey(), id, request, response: null, error: null, sendId: null, pristine: false };
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
          <Sidebar collection={collection} selected={tab.id} onSelect={openRequest} onOpen={open} onCreate={create} onReload={reload} />
          <div className="workspace">
            <TabBar tabs={tabs} active={tab.key} onSelect={activate} onClose={closeTab} onNew={newScratch} />
            <div className="work">
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
      </div>
    </VarsContext.Provider>
  );
}
