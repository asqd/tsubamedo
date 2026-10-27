// What the response must be, one check per line, and the values other requests take from it.
// A line that the core does not understand is marked right under it, before sending.
import { useEffect, useState } from "react";
import { api, Check, Pair } from "../api";
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
      <div className="section-head">Проверки ответа</div>
      <table className="pairs">
        <tbody>
          {all.map((c, i) => {
            const fresh = i === checks.length;
            return (
              <tr key={i} className={c.enabled ? "" : "off"}>
                <td className="chk">
                  {!fresh && <input type="checkbox" checked={c.enabled} onChange={(e) => set(i, { enabled: e.target.checked })} title="Проверять" />}
                </td>
                <td>
                  <CodeEditor mode="line" className="field" value={c.text} placeholder="status == 200" onSubmit={onSubmit} onChange={(text) => set(i, { text })} />
                  <CheckProblem text={c.text} />
                </td>
                <td className="del">
                  {!fresh && (
                    <button className="icon" title="Удалить" onClick={() => onChecks(checks.filter((_, j) => j !== i))}>
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
        Что проверить: <code>status</code>, <code>time</code> (мс), <code>size</code>, <code>header Имя</code>, <code>body</code> или JSONPath <code>$.путь</code>;
        после него <code>count</code> — длина. Условия: <code>==</code> <code>!=</code> <code>&lt;</code> <code>&gt;</code> <code>contains</code> <code>matches</code>{" "}
        <code>exists</code> <code>empty</code>, перед словами можно <code>not</code>. Значение — JSON или просто текст, <code>{"{{переменные}}"}</code> работают.
        <br />
        JSON Schema: <code>$ schema schemas/order.json</code> — файл ищется от папки запроса вверх до корня коллекции;{" "}
        <code>{'$.items[*] schema {"required": ["id"]}'}</code> — схема прямо в строке, каждое совпадение пути проверяется отдельно.
      </div>

      <div className="section-head">Значения из ответа</div>
      <PairsTable rows={captures} onChange={onCaptures} namePlaceholder="Переменная" valuePlaceholder="$.token или header Location" onSubmit={onSubmit} />
      <div className="hint">
        {saved
          ? "Другие запросы коллекции берут их как {{переменную}}. Если значения ещё нет, этот запрос отправится сам перед ними. Значения хранятся в папке данных приложения, не в коллекции."
          : "После отправки значение станет переменной {{имя}}. Отправлять запрос сами перед другими умеют только запросы из коллекции."}
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
    const t = setTimeout(() => {
      api
        .validateCheck(text)
        .then((p) => alive && setProblem(p))
        .catch(() => {});
    }, 300);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [text]);
  return problem ? <div className="check-problem small">{problem}</div> : null;
}
