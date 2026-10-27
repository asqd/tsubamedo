// A collection or folder run: every request with its status, time and failed checks. Requests
// that others take values from come first, even from outside the folder.
import { RunResult } from "../api";
import { ms, size, statusTone } from "../format";

export interface RunState {
  /** Empty for the whole collection. */
  folder: string;
  sendId: string;
  results: RunResult[] | null;
  totalMs: number | null;
  error: string | null;
}

export function RunPanel({
  run,
  env,
  onOpen,
  onAgain,
  onCancel,
  onClose,
}: {
  run: RunState;
  env: string | null;
  onOpen: (id: string) => void;
  onAgain: () => void;
  onCancel: () => void;
  onClose: () => void;
}) {
  const running = run.results === null && run.error === null;
  const passed = run.results?.filter((r) => r.passed).length ?? 0;
  const total = run.results?.length ?? 0;
  return (
    <div className="run-panel" role="dialog" aria-label="Запуск">
      <div className="run-head">
        <div>
          <b>{run.folder ? `Папка ${run.folder}` : "Вся коллекция"}</b>
          <span className="faint small"> · {env ? `окружение ${env}` : "без окружения"}</span>
        </div>
        <span className="sp" />
        {running ? (
          <button onClick={onCancel}>Остановить</button>
        ) : (
          <button onClick={onAgain} title="Запустить ещё раз">
            Ещё раз
          </button>
        )}
        <button className="icon" title="Закрыть" onClick={onClose}>
          ×
        </button>
      </div>
      {running && (
        <div className="pane-empty">
          <span className="spin big" />
          <div className="muted">Запросы отправляются по очереди…</div>
        </div>
      )}
      {run.error && <div className="error-box run-error">{run.error}</div>}
      {run.results && (
        <>
          <div className={`run-summary ${passed === total ? "good-text" : "bad-text"}`}>
            {passed === total ? "✓ Все прошли" : `✗ С ошибкой: ${total - passed}`} · успешно {passed} из {total}
            {run.totalMs != null && <span className="faint"> · {ms(run.totalMs)}</span>}
          </div>
          <div className="run-list scroll">
            {run.results.map((r, i) => (
              <button key={i} className="run-row" onClick={() => onOpen(r.id)} title="Открыть запрос">
                <span className={r.passed ? "good-text" : "bad-text"}>{r.passed ? "✓" : "✗"}</span>
                <span className={`m m-${r.method.toLowerCase()}`}>{r.method === "DELETE" ? "DEL" : r.method.slice(0, 4)}</span>
                <span className="mono ell">{r.id}</span>
                {r.status != null ? <span className={`hist-status ${statusTone(r.status)}`}>{r.status}</span> : <span />}
                <span className="faint small num">
                  {r.checks > 0 && `проверки ${r.checksPassed}/${r.checks} · `}
                  {r.totalMs != null && ms(r.totalMs)}
                  {r.size != null && ` · ${size(r.size)}`}
                </span>
                {r.failures.length > 0 && (
                  <span className="run-why small bad-text">
                    {r.failures.map((f, j) => (
                      <div key={j}>{f}</div>
                    ))}
                  </span>
                )}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
