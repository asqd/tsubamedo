// The collection's git in the sidebar: what changed, the commit, pull and push. Everything
// goes through the system git with the user's own keys and settings.
import { useEffect, useMemo, useState } from "react";
import { api, ChangeKind, CollectionInfo, GitChange, GitState } from "../api";

export interface GitActions {
  /** Status again; with `fetch`, after a look at the server. */
  refresh: (fetch?: boolean) => Promise<void>;
  /** True when the commit went. */
  commit: (paths: string[], message: string) => Promise<boolean>;
  pull: () => Promise<void>;
  push: () => Promise<void>;
  init: () => Promise<void>;
  ignoreSecrets: () => Promise<void>;
}

export const KIND: Record<ChangeKind, { letter: string; label: string }> = {
  modified: { letter: "M", label: "изменён" },
  added: { letter: "A", label: "новый" },
  deleted: { letter: "D", label: "удалён" },
  renamed: { letter: "R", label: "переименован" },
  conflict: { letter: "!", label: "конфликт" },
};

/** The request a changed file is, if it is one. */
const requestId = (path: string) => (path.endsWith(".toml") ? path.slice(0, -5) : null);

/** Every path a change commits: a rename both of its paths. */
const pathsOf = (c: GitChange) => (c.from ? [c.path, c.from] : [c.path]);

export function GitPanel({
  collection,
  git,
  act,
  onOpenRequest,
}: {
  collection: CollectionInfo;
  git: GitState | null;
  act: GitActions;
  onOpenRequest: (id: string) => void;
}) {
  const [unchecked, setUnchecked] = useState<Set<string>>(new Set());
  const [message, setMessage] = useState("");
  const [working, setWorking] = useState<string | null>(null);
  const [diff, setDiff] = useState<GitChange | null>(null);

  // A look at the server when the panel opens, for the counts.
  useEffect(() => {
    act.refresh(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [collection.root]);

  const names = useMemo(() => new Map(collection.requests.map((r) => [r.id, r.name])), [collection]);
  const changes = git?.git === "repo" ? git.changes : [];
  const chosen = changes.filter((c) => !c.secret && !unchecked.has(c.path));
  const secrets = changes.filter((c) => c.secret);

  const run = async (what: string, f: () => Promise<unknown>) => {
    setWorking(what);
    try {
      await f();
    } finally {
      setWorking(null);
    }
  };
  const commit = () =>
    run("commit", async () => {
      if (await act.commit(chosen.flatMap(pathsOf), message)) {
        setMessage("");
        setUnchecked(new Set());
      }
    });

  if (!git) return <div className="side-empty small muted">Смотрю, что в git…</div>;
  if (git.git === "missing") return <div className="side-empty small warn-text">{git.error}</div>;
  if (git.git === "none") {
    return (
      <div className="side-empty small muted">
        <p>Коллекция не в git. Репозиторий в её папке — и запросы можно коммитить, получать и отправлять отсюда; секреты из *.local.toml в него не попадут.</p>
        <div className="side-empty-actions">
          <button disabled={working !== null} onClick={() => run("init", act.init)}>
            Создать репозиторий
          </button>
        </div>
      </div>
    );
  }

  const toggle = (path: string) =>
    setUnchecked((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  const allOn = chosen.length === changes.length - secrets.length;
  const where = git.upstream ? `→ ${git.upstream}` : git.unborn ? "ещё без коммитов" : "не отправлена";

  return (
    <div className="git">
      <div className="git-head">
        <div className="ell" title={git.branch ? `Ветка ${git.branch} ${where}` : "HEAD не на ветке"}>
          <span className="git-branch">⎇ {git.branch ?? "без ветки"}</span> <span className="faint small">{where}</span>
        </div>
        <div className="git-sync">
          <button
            title={git.behind ? `Получить коммиты с сервера: ${git.behind}` : "Получить коммиты с сервера"}
            disabled={working !== null || !git.upstream || !!git.busy}
            onClick={() => run("pull", act.pull)}
          >
            {working === "pull" ? "…" : "↓"} {git.behind || ""}
          </button>
          <button
            title={git.ahead ? `Отправить коммиты на сервер: ${git.ahead}` : git.upstream ? "Отправлять нечего" : "Отправить ветку на сервер"}
            disabled={working !== null || git.unborn || !git.branch || (!!git.upstream && git.ahead === 0)}
            onClick={() => run("push", act.push)}
          >
            {working === "push" ? "…" : "↑"} {git.ahead || ""}
          </button>
          <button className="icon" title="Обновить: посмотреть, что на сервере" disabled={working !== null} onClick={() => run("fetch", () => act.refresh(true))}>
            {working === "fetch" ? "…" : "↻"}
          </button>
        </div>
      </div>
      {git.busy && <div className="git-note warn-text small">В репозитории не закончено {git.busy} — закончите его в git.</div>}

      {changes.length === 0 ? (
        <div className="side-empty small muted">Всё закоммичено.</div>
      ) : (
        <>
          <label className="git-all small faint">
            <input
              type="checkbox"
              checked={allOn && chosen.length > 0}
              disabled={changes.length === secrets.length}
              onChange={() => setUnchecked(allOn ? new Set(changes.filter((c) => !c.secret).map((c) => c.path)) : new Set())}
            />
            Изменения: {changes.length}
          </label>
          <div className="git-list">
            {changes.map((c) => {
              const id = requestId(c.path);
              const name = id ? names.get(id) : undefined;
              return (
                <div key={c.path} className={`item git-row ${c.secret ? "secret" : ""}`} title={`${c.path}: ${KIND[c.kind].label}${c.from ? ` (было ${c.from})` : ""}`}>
                  <input
                    type="checkbox"
                    checked={!c.secret && !unchecked.has(c.path)}
                    disabled={c.secret}
                    onChange={() => toggle(c.path)}
                  />
                  <button className="git-name ell" onClick={() => setDiff(c)}>
                    {name ?? c.path.split("/").pop()}
                    <span className="faint small"> {name ? c.path : c.path.includes("/") ? c.path.slice(0, c.path.lastIndexOf("/")) : ""}</span>
                  </button>
                  <span className={`gm gm-${c.kind}`}>{KIND[c.kind].letter}</span>
                </div>
              );
            })}
          </div>
          {secrets.length > 0 && (
            <div className="git-note small">
              <span className="warn-text">В *.local.toml секреты — они не коммитятся.</span>{" "}
              <button className="link" disabled={working !== null} onClick={() => run("ignore", act.ignoreSecrets)}>
                Добавить в .gitignore
              </button>
            </div>
          )}
          <textarea
            className="git-message"
            rows={3}
            placeholder="Что изменилось"
            value={message}
            spellCheck={false}
            disabled={!!git.busy}
            onChange={(e) => setMessage(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && message.trim() && chosen.length) {
                e.preventDefault();
                commit();
              }
            }}
          />
          <button className="primary git-commit" disabled={working !== null || !message.trim() || chosen.length === 0 || !!git.busy} onClick={commit}>
            {working === "commit" ? "Коммичу…" : `Закоммитить${chosen.length ? ` (${chosen.length})` : ""}`}
          </button>
        </>
      )}
      {git.outside > 0 && (
        <div className="git-note faint small">Ещё изменений вне коллекции: {git.outside} — их коммитят в git.</div>
      )}
      {diff && (
        <DiffDialog
          change={diff}
          onClose={() => setDiff(null)}
          onOpen={requestId(diff.path) && names.has(requestId(diff.path)!) ? () => (setDiff(null), onOpenRequest(requestId(diff.path)!)) : null}
        />
      )}
    </div>
  );
}

function DiffDialog({ change, onClose, onOpen }: { change: GitChange; onClose: () => void; onOpen: (() => void) | null }) {
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.gitDiff(change.path).then(setText, (e) => setError((e as Error).message));
  }, [change.path]);
  // The header of a unified diff says nothing the dialog's title does not.
  const lines = (text ?? "").split("\n").filter((l) => !/^(diff --git|index |--- |\+\+\+ |new file mode|deleted file mode|similarity|rename (from|to))/.test(l));
  return (
    <div className="modal-back" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal diff-dialog" role="dialog" aria-label="Изменения файла" tabIndex={-1} onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <h2 className="ell">{change.path}</h2>
        <div className="faint small">
          {KIND[change.kind].label}
          {change.from ? `, было ${change.from}` : ""} — с последнего коммита
        </div>
        <pre className="diff sel">
          {error ? (
            <span className="bad-text">{error}</span>
          ) : text === null ? (
            "…"
          ) : lines.length === 0 || (lines.length === 1 && !lines[0]) ? (
            <span className="faint">Отличий в тексте нет.</span>
          ) : (
            lines.map((l, i) => (
              <div key={i} className={l.startsWith("+") ? "d-add" : l.startsWith("-") ? "d-del" : l.startsWith("@@") ? "d-hunk" : ""}>
                {l || " "}
              </div>
            ))
          )}
        </pre>
        <div className="modal-actions">
          {onOpen && <button onClick={onOpen}>Открыть запрос</button>}
          <span className="sp" />
          <button className="primary" autoFocus onClick={onClose}>
            Закрыть
          </button>
        </div>
      </div>
    </div>
  );
}
