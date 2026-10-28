// A collection or folder run as it goes: every planned request waiting, in flight or done
// with its status, time and failed checks. Requests that others take values from come first,
// even from outside the folder; several may go at once, and a failed one may be sent again.
import { RunResult, RunStep } from "../api";
import { ms, size, statusTone } from "../format";
import { t } from "../i18n";

export interface RunState {
  /** Empty for the whole collection. */
  folder: string;
  /** Only these requests: sending again what failed. */
  only: string[] | null;
  sendId: string;
  /** The plan, once the core has made it. */
  steps: RunStep[] | null;
  /** Step index → attempt, for those in flight. */
  running: Record<string, number>;
  /** In the order they finished; requests sent first for missing values have no step. */
  results: RunResult[];
  finished: boolean;
  totalMs: number | null;
  error: string | null;
}

export interface RunSettings {
  parallel: number;
  retries: number;
}

/** What failed, as ids to run again. */
export function failedIds(run: RunState): string[] {
  if (!run.steps) return [];
  return run.results.filter((r) => r.step != null && !r.passed).map((r) => run.steps![r.step!].id);
}

export function RunPanel({
  run,
  env,
  settings,
  onSettings,
  onOpen,
  onAgain,
  onRetryFailed,
  onCancel,
  onClose,
}: {
  run: RunState;
  env: string | null;
  settings: RunSettings;
  onSettings: (s: RunSettings) => void;
  onOpen: (id: string) => void;
  onAgain: () => void;
  onRetryFailed: () => void;
  onCancel: () => void;
  onClose: () => void;
}) {
  const running = !run.finished && run.error === null;
  const byStep = new Map<number, RunResult>();
  // Requests sent first for a missing value are shown with the step they were sent for.
  const firsts = new Map<number, RunResult[]>();
  let pending: RunResult[] = [];
  for (const r of run.results) {
    if (r.step == null) {
      pending.push(r);
    } else {
      byStep.set(r.step, r);
      if (pending.length) firsts.set(r.step, pending);
      pending = [];
    }
  }
  const finished = run.results.filter((r) => r.step != null);
  const passed = finished.filter((r) => r.passed).length;
  const total = run.steps?.length ?? 0;
  const failed = failedIds(run).length;
  const title = run.only
    ? t(`Повтор упавших: ${run.only.length}`, `Rerun of failed: ${run.only.length}`)
    : run.folder
      ? t(`Папка ${run.folder}`, `Folder ${run.folder}`)
      : t("Вся коллекция", "Whole collection");

  return (
    <div className="run-panel" role="dialog" aria-label={t("Запуск", "Run")}>
      <div className="run-head">
        <div className="ell">
          <b>{title}</b>
          <span className="faint small"> · {env ? t(`окружение ${env}`, `environment ${env}`) : t("без окружения", "no environment")}</span>
        </div>
        <label
          className="run-setting small muted"
          title={t(
            "Сколько запросов отправлять одновременно. Те, что берут значения из других, всё равно ждут их.",
            "How many requests to send at once. Those that take values from others still wait for them.",
          )}
        >
          {t("одновременно", "at once")}
          <select value={settings.parallel} disabled={running} onChange={(e) => onSettings({ ...settings, parallel: Number(e.target.value) })}>
            {[1, 2, 4, 8].map((n) => (
              <option key={n} value={n}>
                {n === 1 ? t("по одному", "one by one") : n}
              </option>
            ))}
          </select>
        </label>
        <label className="run-setting small muted" title={t("Сколько раз отправить упавший запрос ещё раз, с паузой", "How many times to resend a failed request, with a pause")}>
          {t("повторы", "retries")}
          <select value={settings.retries} disabled={running} onChange={(e) => onSettings({ ...settings, retries: Number(e.target.value) })}>
            {[0, 1, 2, 3].map((n) => (
              <option key={n} value={n}>
                {n === 0 ? t("нет", "none") : n}
              </option>
            ))}
          </select>
        </label>
        {running ? (
          <button onClick={onCancel}>{t("Остановить", "Stop")}</button>
        ) : (
          <>
            {failed > 0 && (
              <button onClick={onRetryFailed} title={t("Отправить ещё раз только упавшие запросы", "Send only the failed requests again")}>
                {t("Повторить упавшие", "Rerun failed")} ({failed})
              </button>
            )}
            <button onClick={onAgain} title={t("Запустить ещё раз", "Run again")}>
              {t("Ещё раз", "Again")}
            </button>
          </>
        )}
        <button className="icon" title={t("Закрыть", "Close")} onClick={onClose}>
          ×
        </button>
      </div>
      {run.error && <div className="error-box run-error">{run.error}</div>}
      {run.steps && (
        <div className={`run-summary ${running ? "" : passed === total ? "good-text" : "bad-text"}`}>
          {running ? (
            <>
              <span className="spin" /> {t(`Готово ${finished.length} из ${total}`, `Done ${finished.length} of ${total}`)}
              {failed > 0 && <span className="bad-text">{t(` · с ошибкой ${failed}`, ` · failed ${failed}`)}</span>}
            </>
          ) : (
            <>
              {passed === total ? t("✓ Все прошли", "✓ All passed") : t(`✗ С ошибкой: ${total - passed}`, `✗ Failed: ${total - passed}`)}
              {t(` · успешно ${passed} из ${total}`, ` · passed ${passed} of ${total}`)}
            </>
          )}
          {run.totalMs != null && <span className="faint"> · {ms(run.totalMs)}</span>}
          <div className="run-bar">
            <span className="good" style={{ width: `${total ? (passed / total) * 100 : 0}%` }} />
            <span className="bad" style={{ width: `${total ? ((finished.length - passed) / total) * 100 : 0}%` }} />
          </div>
        </div>
      )}
      {!run.steps && running && (
        <div className="pane-empty">
          <span className="spin big" />
        </div>
      )}
      {run.steps && (
        <div className="run-list scroll">
          {run.steps.map((step, i) => {
            const r = byStep.get(i);
            const attempt = run.running[i];
            return (
              <div key={i}>
                {(firsts.get(i) ?? []).map((f, j) => (
                  <Row key={j} result={f} id={f.id} method={f.method} name={null} first onOpen={onOpen} />
                ))}
                {r ? (
                  <Row result={r} id={step.id} method={step.method} name={step.name} onOpen={onOpen} />
                ) : (
                  <button className={`run-row ${attempt ? "" : "waiting"}`} onClick={() => onOpen(step.id)} title={t("Открыть запрос", "Open request")}>
                    <span>{attempt ? <span className="spin" /> : <span className="faint">·</span>}</span>
                    <Method method={step.method} />
                    <span className="ell">
                      {step.name ?? step.id}
                      {step.name && <span className="faint small"> {step.id}</span>}
                    </span>
                    <span />
                    <span className="faint small">{attempt
                        ? attempt > 1
                          ? t(`попытка ${attempt}`, `attempt ${attempt}`)
                          : t("отправляется…", "sending…")
                        : running
                          ? t("ждёт", "waiting")
                          : t("не отправлен", "not sent")}</span>
                  </button>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function Method({ method }: { method: string }) {
  return <span className={`m m-${method.toLowerCase()}`}>{method === "DELETE" ? "DEL" : method.slice(0, 4)}</span>;
}

function Row({ result: r, id, method, name, first, onOpen }: { result: RunResult; id: string; method: string; name: string | null; first?: boolean; onOpen: (id: string) => void }) {
  return (
    <button className={`run-row ${first ? "first" : ""}`} onClick={() => onOpen(id)} title={first ? t("Отправлен сначала: его значение понадобилось", "Sent first: its value was needed") : t("Открыть запрос", "Open request")}>
      <span className={r.passed ? "good-text" : "bad-text"}>{r.passed ? "✓" : "✗"}</span>
      <Method method={method} />
      <span className="ell">
        {first && <span className="faint small">{t("сначала ", "first ")}</span>}
        {name ?? id}
        {name && <span className="faint small"> {id}</span>}
      </span>
      {r.status != null ? <span className={`hist-status ${statusTone(r.status)}`}>{r.status}</span> : <span />}
      <span className="faint small num">
        {r.attempts > 1 && t(`попытка ${r.attempts} · `, `attempt ${r.attempts} · `)}
        {r.checks > 0 && t(`проверки ${r.checksPassed}/${r.checks} · `, `checks ${r.checksPassed}/${r.checks} · `)}
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
  );
}
