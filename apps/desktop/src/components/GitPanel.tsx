// The collection's git in the sidebar: what changed, the commit, pull and push. Everything
// goes through the system git with the user's own keys and settings.
import { useEffect, useMemo, useState } from "react";
import { api, ChangeKind, CollectionInfo, GitChange, GitState } from "../api";
import { t } from "../i18n";

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

export const KIND_LETTER: Record<ChangeKind, string> = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  conflict: "!",
};

/** What a change is, in words; read at render time, in the window's language. */
export function kindLabel(kind: ChangeKind): string {
  switch (kind) {
    case "modified":
      return t("изменён", "modified");
    case "added":
      return t("новый", "new");
    case "deleted":
      return t("удалён", "deleted");
    case "renamed":
      return t("переименован", "renamed");
    case "conflict":
      return t("конфликт", "conflict");
  }
}

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

  if (!git) return <div className="side-empty small muted">{t("Смотрю, что в git…", "Checking git…")}</div>;
  if (git.git === "missing") return <div className="side-empty small warn-text">{git.error}</div>;
  if (git.git === "none") {
    return (
      <div className="side-empty small muted">
        <p>
          {t(
            "Коллекция не в git. Репозиторий в её папке — и запросы можно коммитить, получать и отправлять отсюда; секреты из *.local.toml в него не попадут.",
            "The collection is not in git. Make its folder a repository to commit, pull and push requests from here; secrets from *.local.toml stay out of it.",
          )}
        </p>
        <div className="side-empty-actions">
          <button disabled={working !== null} onClick={() => run("init", act.init)}>
            {t("Создать репозиторий", "Create repository")}
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
  const where = git.upstream ? `→ ${git.upstream}` : git.unborn ? t("ещё без коммитов", "no commits yet") : t("не отправлена", "not pushed");

  return (
    <div className="git">
      <div className="git-head">
        <div className="ell" title={git.branch ? t(`Ветка ${git.branch} ${where}`, `Branch ${git.branch} ${where}`) : t("HEAD не на ветке", "HEAD is not on a branch")}>
          <span className="git-branch">⎇ {git.branch ?? t("без ветки", "no branch")}</span> <span className="faint small">{where}</span>
        </div>
        <div className="git-sync">
          <button
            title={git.behind ? t(`Получить коммиты с сервера: ${git.behind}`, `Pull commits from the server: ${git.behind}`) : t("Получить коммиты с сервера", "Pull commits from the server")}
            disabled={working !== null || !git.upstream || !!git.busy}
            onClick={() => run("pull", act.pull)}
          >
            {working === "pull" ? "…" : "↓"} {git.behind || ""}
          </button>
          <button
            title={
              git.ahead
                ? t(`Отправить коммиты на сервер: ${git.ahead}`, `Push commits to the server: ${git.ahead}`)
                : git.upstream
                  ? t("Отправлять нечего", "Nothing to push")
                  : t("Отправить ветку на сервер", "Push the branch to the server")
            }
            disabled={working !== null || git.unborn || !git.branch || (!!git.upstream && git.ahead === 0)}
            onClick={() => run("push", act.push)}
          >
            {working === "push" ? "…" : "↑"} {git.ahead || ""}
          </button>
          <button className="icon" title={t("Обновить: посмотреть, что на сервере", "Refresh: check the server")} disabled={working !== null} onClick={() => run("fetch", () => act.refresh(true))}>
            {working === "fetch" ? "…" : "↻"}
          </button>
        </div>
      </div>
      {git.busy && <div className="git-note warn-text small">{t(`В репозитории не закончено ${git.busy} — закончите его в git.`, `The repository has an unfinished ${git.busy} — finish it in git.`)}</div>}

      {changes.length === 0 ? (
        <div className="side-empty small muted">{t("Всё закоммичено.", "Everything is committed.")}</div>
      ) : (
        <>
          <label className="git-all small faint">
            <input
              type="checkbox"
              checked={allOn && chosen.length > 0}
              disabled={changes.length === secrets.length}
              onChange={() => setUnchecked(allOn ? new Set(changes.filter((c) => !c.secret).map((c) => c.path)) : new Set())}
            />
            {t("Изменения", "Changes")}: {changes.length}
          </label>
          <div className="git-list">
            {changes.map((c) => {
              const id = requestId(c.path);
              const name = id ? names.get(id) : undefined;
              return (
                <div key={c.path} className={`item git-row ${c.secret ? "secret" : ""}`} title={`${c.path}: ${kindLabel(c.kind)}${c.from ? t(` (было ${c.from})`, ` (was ${c.from})`) : ""}`}>
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
                  <span className={`gm gm-${c.kind}`}>{KIND_LETTER[c.kind]}</span>
                </div>
              );
            })}
          </div>
          {secrets.length > 0 && (
            <div className="git-note small">
              <span className="warn-text">{t("В *.local.toml секреты — они не коммитятся.", "*.local.toml holds secrets — they are not committed.")}</span>{" "}
              <button className="link" disabled={working !== null} onClick={() => run("ignore", act.ignoreSecrets)}>
                {t("Добавить в .gitignore", "Add to .gitignore")}
              </button>
            </div>
          )}
          <textarea
            className="git-message"
            rows={3}
            placeholder={t("Что изменилось", "What changed")}
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
            {working === "commit" ? t("Коммичу…", "Committing…") : `${t("Закоммитить", "Commit")}${chosen.length ? ` (${chosen.length})` : ""}`}
          </button>
        </>
      )}
      {git.outside > 0 && (
        <div className="git-note faint small">
          {t(
            `Ещё изменений вне коллекции: ${git.outside} — их коммитят в git.`,
            `More changes outside the collection: ${git.outside} — commit those in git.`,
          )}
        </div>
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
      <div className="modal diff-dialog" role="dialog" aria-label={t("Изменения файла", "File changes")} tabIndex={-1} onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <h2 className="ell">{change.path}</h2>
        <div className="faint small">
          {kindLabel(change.kind)}
          {change.from ? t(`, было ${change.from}`, `, was ${change.from}`) : ""} {t("— с последнего коммита", "— since the last commit")}
        </div>
        <pre className="diff sel">
          {error ? (
            <span className="bad-text">{error}</span>
          ) : text === null ? (
            "…"
          ) : lines.length === 0 || (lines.length === 1 && !lines[0]) ? (
            <span className="faint">{t("Отличий в тексте нет.", "No differences in the text.")}</span>
          ) : (
            lines.map((l, i) => (
              <div key={i} className={l.startsWith("+") ? "d-add" : l.startsWith("-") ? "d-del" : l.startsWith("@@") ? "d-hunk" : ""}>
                {l || " "}
              </div>
            ))
          )}
        </pre>
        <div className="modal-actions">
          {onOpen && <button onClick={onOpen}>{t("Открыть запрос", "Open request")}</button>}
          <span className="sp" />
          <button className="primary" autoFocus onClick={onClose}>
            {t("Закрыть", "Close")}
          </button>
        </div>
      </div>
    </div>
  );
}
