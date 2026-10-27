// Two responses side by side over the editors: the open one and the same request in another
// environment, or an earlier answer from history. JSON by meaning, other text by lines.
import { useEffect, useState } from "react";
import { api, Comparison, CompareSide, JsonChange } from "../api";
import { size } from "../format";

export interface CompareState {
  /** The request, for the remembered list of what not to compare. */
  requestId: string | null;
  left: { side: CompareSide; label: string };
  rightLabel: string;
  /** Null while the other environment is being asked. */
  right: CompareSide | null;
  sendId: string | null;
  error: string | null;
}

const MARK: Record<JsonChange["kind"], string> = { added: "+", removed: "−", changed: "~", order: "↕" };

function storedIgnore(requestId: string | null): string {
  try {
    return localStorage.getItem(`compare-ignore:${requestId ?? ""}`) ?? "";
  } catch {
    return "";
  }
}

export function ComparePanel({ state, onCancel, onClose }: { state: CompareState; onCancel: () => void; onClose: () => void }) {
  const [ignore, setIgnore] = useState(() => storedIgnore(state.requestId));
  const [allHeaders, setAllHeaders] = useState(false);
  const [result, setResult] = useState<Comparison | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const patterns = ignore
    .split(/[,\n]/)
    .map((p) => p.trim())
    .filter(Boolean);

  useEffect(() => {
    try {
      localStorage.setItem(`compare-ignore:${state.requestId ?? ""}`, ignore);
    } catch {
      // Only a convenience.
    }
  }, [ignore, state.requestId]);

  useEffect(() => {
    if (!state.right) return;
    let live = true;
    const right = state.right;
    const timer = setTimeout(() => {
      api.compare(state.left.side, right, patterns, allHeaders).then(
        (r) => {
          if (!live) return;
          setResult(r);
          setProblem(null);
        },
        (e) => live && setProblem((e as Error).message),
      );
    }, 250);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [state.right, patterns.join("\n"), allHeaders]);

  /** `$.items[id=42].updatedAt` → `$.items[*].updatedAt`: the same field in every element. */
  const skip = (path: string) => {
    const general = path.replace(/\[(id=[^\]]*|\d+)\]/g, "[*]");
    if (!patterns.includes(general)) setIgnore([...patterns, general].join(", "));
  };

  return (
    <div className="run-panel compare-panel" role="dialog" aria-label="Сравнение ответов">
      <div className="run-head">
        <div className="ell">
          <b>Сравнение</b>
          <span className="small">
            {" "}
            <span className="cmp-left">{state.left.label}</span> ⟷ <span className="cmp-right">{state.rightLabel}</span>
          </span>
        </div>
        <button className="icon" title="Закрыть" onClick={onClose}>
          ×
        </button>
      </div>
      <div className="compare-options small">
        <input
          className="mono"
          value={ignore}
          placeholder="Не сравнивать: $.meta.requestId, $.items[*].updatedAt, updatedAt"
          spellCheck={false}
          onChange={(e) => setIgnore(e.target.value)}
        />
        <label className="muted">
          <input type="checkbox" checked={allHeaders} onChange={(e) => setAllHeaders(e.target.checked)} /> все заголовки
        </label>
      </div>
      {state.error && <div className="error-box run-error">{state.error}</div>}
      {problem && <div className="error-box run-error">{problem}</div>}
      {!state.right && !state.error && (
        <div className="pane-empty">
          <span className="spin big" />
          <div className="muted">Отправляю запрос: {state.rightLabel}…</div>
          <button onClick={onCancel}>Отменить</button>
        </div>
      )}
      {state.right && !result && !problem && (
        <div className="pane-empty">
          <span className="spin big" />
          <div className="muted">Сравниваю…</div>
        </div>
      )}
      {result && state.right && <Result result={result} onSkip={skip} />}
    </div>
  );
}

function Result({ result, onSkip }: { result: Comparison; onSkip: (path: string) => void }) {
  const [a, b] = result.status;
  const body = result.body;
  const bodyText =
    body.kind === "same"
      ? "одинаковое"
      : body.kind === "json"
        ? body.total === 0
          ? "одинаковое по смыслу"
          : `изменений ${body.total}`
        : body.kind === "text"
          ? `+${body.added} −${body.removed} строк`
          : body.firstDifference
            ? "различается"
            : "одинаковое";
  return (
    <div className="run-list scroll compare-body">
      <div className="compare-summary small">
        <span className={a === b ? "muted" : "bad-text"}>Статус {a === b ? `${a} в обоих` : `${a} → ${b}`}</span>
        <span className={result.headers.length ? "warn-text" : "muted"}>
          Заголовки: {result.headers.length ? `различий ${result.headers.length}` : "одинаковые"}
          {result.hiddenHeaders > 0 && <span className="faint"> (ещё {result.hiddenHeaders} меняются всегда)</span>}
        </span>
        <span className={bodyText.startsWith("одинаков") ? "good-text" : "warn-text"}>Тело: {bodyText}</span>
      </div>

      {result.headers.length > 0 && (
        <table className="cmp-table small">
          <tbody>
            {result.headers.map((h) => (
              <tr key={h.name}>
                <td className="mono">{h.name}</td>
                <td className="mono cmp-left">{h.left ?? <span className="faint">нет</span>}</td>
                <td className="mono cmp-right">{h.right ?? <span className="faint">нет</span>}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {body.kind === "same" && <p className="muted small">Тела совпадают байт в байт, {size(body.size)}.</p>}
      {body.kind === "json" && (
        <>
          {body.total === 0 && <p className="muted small">Отличаются только порядок ключей, пробелы или то, что не сравнивается.</p>}
          {body.changes.length > 0 && (
            <table className="cmp-table small">
              <tbody>
                {body.changes.map((c, i) => (
                  <tr key={i} className={`cmp-${c.kind}`}>
                    <td className="cmp-mark">{MARK[c.kind]}</td>
                    <td className="mono cmp-path" title={c.path}>
                      {c.path}
                      {c.kind === "order" && <span className="faint"> — порядок</span>}
                      <button className="icon cmp-skip" title="Не сравнивать это место" onClick={() => onSkip(c.path)}>
                        ⊘
                      </button>
                    </td>
                    <td className="mono cmp-left">{c.left ?? ""}</td>
                    <td className="mono cmp-right">{c.right ?? ""}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {body.total > body.changes.length && <p className="faint small">И ещё {body.total - body.changes.length} изменений.</p>}
          {body.ignored > 0 && <p className="faint small">Не сравнивалось: {body.ignored}.</p>}
        </>
      )}
      {body.kind === "text" && (
        <div className="cmp-lines mono small">
          {body.lines.map((l, i) =>
            l.tag === "gap" ? (
              <div key={i} className="cmp-gap">
                ⋯
              </div>
            ) : (
              <div key={i} className={`cmp-line ${l.tag}`}>
                <span className="faint">{l.left ?? ""}</span>
                <span className="faint">{l.right ?? ""}</span>
                <span>{l.tag === "added" ? "+" : l.tag === "removed" ? "−" : " "}</span>
                <span className="cmp-text">{l.text}</span>
              </div>
            ),
          )}
          {body.truncated && <div className="faint">Показано не всё.</div>}
        </div>
      )}
      {body.kind === "big" && (
        <p className="small">
          Тела слишком большие, чтобы сравнивать их по строкам ({size(body.sizes[0])} и {size(body.sizes[1])})
          {body.firstDifference ? `: расходятся со строки ${body.firstDifference.line} (байт ${body.firstDifference.byte}).` : ": совпадают."}
        </p>
      )}
    </div>
  );
}
