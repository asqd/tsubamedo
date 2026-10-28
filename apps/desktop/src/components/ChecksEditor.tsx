// What the response must be, one check per line, and the values other requests take from it.
// A line that the core does not understand is marked right under it, before sending.
import { useEffect, useState } from "react";
import { api, Check, Pair } from "../api";
import { t } from "../i18n";
import { CodeEditor } from "./CodeEditor";
import { PairsTable } from "./PairsTable";

const EXAMPLES = ["status == 200", "time < 1000", "$.id exists", "$.items count > 0", 'header Content-Type contains json'];

export function ChecksEditor({
  checks,
  captures,
  onChecks,
  onCaptures,
  onSubmit,
  saved,
}: {
  checks: Check[];
  captures: Pair[];
  onChecks: (c: Check[]) => void;
  onCaptures: (c: Pair[]) => void;
  onSubmit: () => void;
  /** Captured values reach other requests only from a request saved in the collection. */
  saved: boolean;
}) {
  const all = [...checks, { text: "", enabled: true }];
  const set = (i: number, patch: Partial<Check>) => {
    const next = all.map((c, j) => (j === i ? { ...c, ...patch } : c));
    onChecks(next.filter((c, j) => j < checks.length || c.text));
  };
  const unused = EXAMPLES.filter((e) => !checks.some((c) => c.text === e));

  return (
    <div className="checks-editor">
      <div className="section-head">{t("Проверки ответа", "Response checks")}</div>
      <table className="pairs">
        <tbody>
          {all.map((c, i) => {
            const fresh = i === checks.length;
            return (
              <tr key={i} className={c.enabled ? "" : "off"}>
                <td className="chk">
                  {!fresh && <input type="checkbox" checked={c.enabled} onChange={(e) => set(i, { enabled: e.target.checked })} title={t("Проверять", "Check")} />}
                </td>
                <td>
                  <CodeEditor mode="line" className="field" value={c.text} placeholder="status == 200" onSubmit={onSubmit} onChange={(text) => set(i, { text })} />
                  <CheckProblem text={c.text} />
                </td>
                <td className="del">
                  {!fresh && (
                    <button className="icon" title={t("Удалить", "Delete")} onClick={() => onChecks(checks.filter((_, j) => j !== i))}>
                      ×
                    </button>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {unused.length > 0 && (
        <div className="chips">
          {unused.map((e) => (
            <button key={e} className="chip mono" onClick={() => onChecks([...checks, { text: e, enabled: true }])}>
              + {e}
            </button>
          ))}
        </div>
      )}
      <div className="hint">
        {t("Что проверить:", "What to check:")} <code>status</code>, <code>time</code> ({t("мс", "ms")}), <code>size</code>, <code>header {t("Имя", "Name")}</code>,{" "}
        <code>body</code> {t("или JSONPath", "or a JSONPath")} <code>$.{t("путь", "path")}</code>; {t("после него", "after it,")} <code>count</code>{" "}
        {t("— длина. Условия:", "gives the length. Conditions:")} <code>==</code> <code>!=</code> <code>&lt;</code> <code>&gt;</code> <code>contains</code> <code>matches</code>{" "}
        <code>exists</code> <code>empty</code>
        {t(", перед словами можно", "; a word condition can take")} <code>not</code>
        {t(". Значение — JSON или просто текст,", " before it. The value is JSON or plain text;")} <code>{t("{{переменные}}", "{{variables}}")}</code>{" "}
        {t("работают.", "work.")}
        <br />
        {t("Путь с", "A path with")} <code>*</code>, <code>..</code> {t("или фильтром:", "or a filter:")} <code>$.items[*].price &gt; 0</code>{" "}
        {t("— каждое совпадение;", "— every match;")} <code>count</code>, <code>exists</code>, <code>empty</code> {t("и", "and")} <code>contains</code>{" "}
        {t("— про весь список.", "— the whole list.")}
        <br />
        JSON Schema: <code>$ schema schemas/order.json</code>{" "}
        {t("— файл ищется от папки запроса вверх до корня коллекции;", "— the file is looked up from the request's folder up to the collection root;")}{" "}
        <code>{'$.items[*] schema {"required": ["id"]}'}</code>{" "}
        {t("— схема прямо в строке, каждое совпадение пути проверяется отдельно.", "— a schema right in the line; each match of the path is checked separately.")}
      </div>

      <div className="section-head">{t("Значения из ответа", "Values from the response")}</div>
      <PairsTable rows={captures} onChange={onCaptures} namePlaceholder={t("Переменная", "Variable")} valuePlaceholder={t("$.token или header Location", "$.token or header Location")} onSubmit={onSubmit} />
      <div className="hint">
        {saved
          ? t(
              "Другие запросы коллекции берут их как {{переменную}}. Если значения ещё нет, этот запрос отправится сам перед ними. Значения хранятся в папке данных приложения, не в коллекции.",
              "Other requests of the collection use them as {{variable}}. If a value is not there yet, this request is sent first on its own. Values are kept in the app's data folder, not in the collection.",
            )
          : t(
              "После отправки значение станет переменной {{имя}}. Отправлять запрос сами перед другими умеют только запросы из коллекции.",
              "After sending, the value becomes the variable {{name}}. Only requests from a collection can send themselves before others.",
            )}
      </div>
    </div>
  );
}

/** The core's reason, a moment after typing stops. */
function CheckProblem({ text }: { text: string }) {
  const [problem, setProblem] = useState<string | null>(null);
  useEffect(() => {
    if (!text.trim()) return setProblem(null);
    let alive = true;
    const timer = setTimeout(() => {
      api
        .validateCheck(text)
        .then((p) => alive && setProblem(p))
        .catch(() => {});
    }, 300);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [text]);
  return problem ? <div className="check-problem small">{problem}</div> : null;
}
