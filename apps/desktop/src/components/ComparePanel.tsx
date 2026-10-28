// Two responses side by side over the editors: the open one and the same request in another
// environment, or an earlier answer from history. JSON by meaning, other text by lines.
import { useEffect, useState } from "react";
import { api, Comparison, CompareSide, JsonChange } from "../api";
import { size } from "../format";
import { t } from "../i18n";

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
    <div className="run-panel compare-panel" role="dialog" aria-label={t("Сравнение ответов", "Response comparison")}>
      <div className="run-head">
        <div className="ell">
          <b>{t("Сравнение", "Comparison")}</b>
          <span className="small">
            {" "}
            <span className="cmp-left">{state.left.label}</span> ⟷ <span className="cmp-right">{state.rightLabel}</span>
          </span>
        </div>
        <button className="icon" title={t("Закрыть", "Close")} onClick={onClose}>
          ×
        </button>
      </div>
      <div className="compare-options small">
        <input
          className="mono"
          value={ignore}
          placeholder={t("Не сравнивать: $.meta.requestId, $.items[*].updatedAt, updatedAt", "Ignore: $.meta.requestId, $.items[*].updatedAt, updatedAt")}
          spellCheck={false}
          onChange={(e) => setIgnore(e.target.value)}
        />
        <label className="muted">
          <input type="checkbox" checked={allHeaders} onChange={(e) => setAllHeaders(e.target.checked)} /> {t("все заголовки", "all headers")}
        </label>
      </div>
      {state.error && <div className="error-box run-error">{state.error}</div>}
      {problem && <div className="error-box run-error">{problem}</div>}
      {!state.right && !state.error && (
        <div className="pane-empty">
          <span className="spin big" />
          <div className="muted">{t(`Отправляю запрос: ${state.rightLabel}…`, `Sending the request: ${state.rightLabel}…`)}</div>
          <button onClick={onCancel}>{t("Отменить", "Cancel")}</button>
        </div>
      )}
      {state.right && !result && !problem && (
        <div className="pane-empty">
          <span className="spin big" />
          <div className="muted">{t("Сравниваю…", "Comparing…")}</div>
        </div>
      )}
      {result && state.right && <Result result={result} onSkip={skip} />}
    </div>
  );
}

function Result({ result, onSkip }: { result: Comparison; onSkip: (path: string) => void }) {
  const [a, b] = result.status;
  const body = result.body;
  const bodySame = body.kind === "same" || (body.kind === "json" && body.total === 0) || (body.kind === "big" && !body.firstDifference);
  const bodyText =
    body.kind === "same"
      ? t("одинаковое", "identical")
      : body.kind === "json"
        ? body.total === 0
          ? t("одинаковое по смыслу", "same in meaning")
          : t(`изменений ${body.total}`, `${body.total} ${body.total === 1 ? "change" : "changes"}`)
        : body.kind === "text"
          ? t(`+${body.added} −${body.removed} строк`, `+${body.added} −${body.removed} lines`)
          : body.firstDifference
            ? t("различается", "differs")
            : t("одинаковое", "identical");
  return (
    <div className="run-list scroll compare-body">
      <div className="compare-summary small">
        <span className={a === b ? "muted" : "bad-text"}>
          {t("Статус", "Status")} {a === b ? t(`${a} в обоих`, `${a} in both`) : `${a} → ${b}`}
        </span>
        <span className={result.headers.length ? "warn-text" : "muted"}>
          {t("Заголовки", "Headers")}:{" "}
          {result.headers.length
            ? t(`различий ${result.headers.length}`, `${result.headers.length} ${result.headers.length === 1 ? "difference" : "differences"}`)
            : t("одинаковые", "identical")}
          {result.hiddenHeaders > 0 && (
            <span className="faint">{t(` (ещё ${result.hiddenHeaders} меняются всегда)`, ` (${result.hiddenHeaders} more always change)`)}</span>
          )}
        </span>
        <span className={bodySame ? "good-text" : "warn-text"}>
          {t("Тело", "Body")}: {bodyText}
        </span>
      </div>

      {result.headers.length > 0 && (
        <table className="cmp-table small">
          <tbody>
            {result.headers.map((h) => (
              <tr key={h.name}>
                <td className="mono">{h.name}</td>
                <td className="mono cmp-left">{h.left ?? <span className="faint">{t("нет", "none")}</span>}</td>
                <td className="mono cmp-right">{h.right ?? <span className="faint">{t("нет", "none")}</span>}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {body.kind === "same" && <p className="muted small">{t(`Тела совпадают байт в байт, ${size(body.size)}.`, `The bodies match byte for byte, ${size(body.size)}.`)}</p>}
      {body.kind === "json" && (
        <>
          {body.total === 0 && <p className="muted small">{t("Отличаются только порядок ключей, пробелы или то, что не сравнивается.", "Only key order, whitespace or ignored parts differ.")}</p>}
          {body.changes.length > 0 && (
            <table className="cmp-table small">
              <tbody>
                {body.changes.map((c, i) => (
                  <tr key={i} className={`cmp-${c.kind}`}>
                    <td className="cmp-mark">{MARK[c.kind]}</td>
                    <td className="mono cmp-path" title={c.path}>
                      {c.path}
                      {c.kind === "order" && <span className="faint">{t(" — порядок", " — order")}</span>}
                      <button className="icon cmp-skip" title={t("Не сравнивать это место", "Ignore this path")} onClick={() => onSkip(c.path)}>
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
          {body.total > body.changes.length && <p className="faint small">
              {t(`И ещё ${body.total - body.changes.length} изменений.`, `And ${body.total - body.changes.length} more ${body.total - body.changes.length === 1 ? "change" : "changes"}.`)}
            </p>}
          {body.ignored > 0 && <p className="faint small">{t(`Не сравнивалось: ${body.ignored}.`, `Ignored: ${body.ignored}.`)}</p>}
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
          {body.truncated && <div className="faint">{t("Показано не всё.", "Not everything is shown.")}</div>}
        </div>
      )}
      {body.kind === "big" && (
        <p className="small">
          {t(
            `Тела слишком большие, чтобы сравнивать их по строкам (${size(body.sizes[0])} и ${size(body.sizes[1])})`,
            `The bodies are too big to compare line by line (${size(body.sizes[0])} and ${size(body.sizes[1])})`,
          )}
          {body.firstDifference
            ? t(
                `: расходятся со строки ${body.firstDifference.line} (байт ${body.firstDifference.byte}).`,
                `: they differ from line ${body.firstDifference.line} (byte ${body.firstDifference.byte}).`,
              )
            : t(": совпадают.", ": they match.")}
        </p>
      )}
    </div>
  );
}
