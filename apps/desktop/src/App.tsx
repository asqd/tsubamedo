// The window: collection on the left, the request in the middle, the answer on the right.
// Edits are saved to the request's file as you type; the core rewrites only what changed.
import { useCallback, useEffect, useRef, useState } from "react";
import { api, CollectionInfo, emptyRequest, Request, ResponseMeta } from "./api";
import { Logo } from "./components/Logo";
import { RequestEditor } from "./components/RequestEditor";
import { ResponsePane } from "./components/ResponsePane";
import { Sidebar } from "./components/Sidebar";
import { mod, pickFolder } from "./platform";

const SCRATCH = "";
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

function loadScratch(): Request {
  try {
    return { ...emptyRequest(), ...JSON.parse(store.get("scratch") ?? "{}") };
  } catch {
    return emptyRequest();
  }
}

export default function App() {
  const [collection, setCollection] = useState<CollectionInfo | null>(null);
  const [env, setEnv] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [request, setRequest] = useState<Request>(loadScratch);
  const [responses, setResponses] = useState<Record<string, ResponseMeta>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [sending, setSending] = useState<Record<string, string>>({});
  const [problem, setProblem] = useState<string | null>(null);
  const [saveState, setSaveState] = useState<"idle" | "saving" | "saved">("idle");
  const [split, setSplit] = useState(() => Number(store.get("split")) || 0.46);

  const pendingSave = useRef<{ id: string; request: Request; timer: number } | null>(null);
  const slot = selected ?? SCRATCH;

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

  const openCollection = useCallback(async (path: string) => {
    try {
      const info = await api.openCollection(path);
      setCollection(info);
      setProblem(null);
      store.set("collection", info.root);
      const savedEnv = store.get(`env:${info.root}`);
      setEnv(savedEnv && info.environments.includes(savedEnv) ? savedEnv : (info.environments[0] ?? null));
    } catch (e) {
      setProblem((e as Error).message);
    }
  }, []);

  useEffect(() => {
    const last = store.get("collection");
    if (last) openCollection(last);
  }, [openCollection]);

  const select = useCallback(
    async (id: string | null) => {
      await flushSave();
      setSaveState("idle");
      if (id === null) {
        setSelected(null);
        setRequest(loadScratch());
        return;
      }
      try {
        const r = await api.getRequest(id);
        setSelected(id);
        setRequest(r);
      } catch (e) {
        setProblem((e as Error).message);
      }
    },
    [flushSave],
  );

  const change = (next: Request) => {
    setRequest(next);
    if (selected === null) {
      store.set("scratch", JSON.stringify(next));
      return;
    }
    if (pendingSave.current) clearTimeout(pendingSave.current.timer);
    pendingSave.current = { id: selected, request: next, timer: window.setTimeout(flushSave, 400) };
    setSaveState("saving");
  };

  const send = useCallback(async () => {
    if (sending[slot] || !request.url.trim()) return;
    await flushSave();
    const sendId = crypto.randomUUID();
    setSending((s) => ({ ...s, [slot]: sendId }));
    setErrors((e) => ({ ...e, [slot]: "" }));
    try {
      const meta = await api.send(sendId, selected, request, env, false);
      setResponses((r) => ({ ...r, [slot]: meta }));
    } catch (e) {
      const message = (e as Error).message;
      if (message !== "Отменено") setErrors((er) => ({ ...er, [slot]: message }));
    } finally {
      setSending((s) => {
        const next = { ...s };
        delete next[slot];
        return next;
      });
    }
  }, [sending, slot, request, flushSave, selected, env]);

  const cancel = () => {
    const id = sending[slot];
    if (id) api.cancel(id);
  };

  const open = async () => {
    const path = await pickFolder();
    if (path) openCollection(path);
  };

  const create = async () => {
    if (!collection) return;
    await flushSave();
    const folder = selected?.includes("/") ? selected.slice(0, selected.lastIndexOf("/")) : "";
    try {
      const { id, collection: info } = await api.createRequest(folder, "Новый запрос");
      setCollection(info);
      await select(id);
    } catch (e) {
      setProblem((e as Error).message);
    }
  };

  const reload = async () => {
    await flushSave();
    try {
      setCollection(await api.reloadCollection());
      if (selected) setRequest(await api.getRequest(selected));
    } catch (e) {
      setProblem((e as Error).message);
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!mod(e)) return;
      if (e.key === "Enter") {
        e.preventDefault();
        send();
      } else if (e.key === "o") {
        e.preventDefault();
        open();
      } else if (e.key === "n") {
        e.preventDefault();
        create();
      } else if (e.key === "s") {
        // Saving is automatic; ⌘S just makes it immediate.
        e.preventDefault();
        flushSave();
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
        <Sidebar collection={collection} selected={selected} onSelect={select} onOpen={open} onCreate={create} onReload={reload} />
        <div className="work">
          <section className="left" style={{ width: `${split * 100}%` }}>
            <RequestEditor request={request} onChange={change} onSend={send} sending={Boolean(sending[slot])} saved={selected} />
          </section>
          <div className="splitter" onPointerDown={dragSplit} />
          <section className="right">
            <ResponsePane meta={responses[slot] ?? null} error={errors[slot] || null} sending={Boolean(sending[slot])} onCancel={cancel} />
          </section>
        </div>
      </div>
    </div>
  );
}
