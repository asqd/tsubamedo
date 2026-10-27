// The answer: status line, then body, headers, time by phase and what was actually sent.
import { useState } from "react";
import { ResponseMeta } from "../api";
import { ms, size, statusTone } from "../format";
import { BodyViewer } from "./BodyViewer";

type Tab = "body" | "checks" | "headers" | "time" | "sent";

export function ResponsePane({
  meta,
  error,
  sending,
  onCancel,
}: {
  meta: ResponseMeta | null;
  error: string | null;
  sending: boolean;
  onCancel: () => void;
}) {
  const [tab, setTab] = useState<Tab>("body");

  if (sending) {
    return (
      <div className="pane-empty">
        <span className="spin big" />
        <div className="muted">Отправка…</div>
        <button onClick={onCancel}>Отменить</button>
      </div>
    );
  }
  if (error) {
    return (
      <div className="pane-empty">
        <div className="error-box">{error}</div>
      </div>
    );
  }
  if (!meta) {
    return (
      <div className="pane-empty faint">
        <div>Ответ появится здесь</div>
        <div className="small">⌘↵ — отправить запрос</div>
      </div>
    );
  }

  const compressed = meta.wireSize !== meta.size && meta.wireSize > 0;
  const checks = meta.checks ?? [];
  const captures = meta.captures ?? [];
  const before = meta.before ?? [];
  const logs = meta.logs ?? [];
  const failed = checks.filter((c) => !c.ok).length + captures.filter((c) => c.error).length + before.filter((b) => !b.passed).length;
  const hasChecks = checks.length + captures.length + before.length + logs.length > 0;
  return (
    <div className="response">
      <div className="status-line">
        <span className={`status ${statusTone(meta.status)}`}>
          {meta.status} {meta.reason}
        </span>
        <span className="metric" title="Всё время запроса">
          {ms(meta.timings.total)}
        </span>
        <span className="metric" title={compressed ? `По сети ${size(meta.wireSize)}` : undefined}>
          {size(meta.size)}
          {compressed && <span className="faint"> · сжато {size(meta.wireSize)}</span>}
        </span>
        <span className="faint small">{meta.version}</span>
        {meta.redirects.length > 0 && (
          <span className="faint small" title={meta.redirects.map((r) => `${r.status} ${r.url}`).join("\n")}>
            {meta.redirects.length} перенаправл.
          </span>
        )}
        {checks.length > 0 && (
          <button className={`check-badge ${failed ? "bad" : "ok"}`} onClick={() => setTab("checks")} title="Проверки ответа">
            {failed ? "✗" : "✓"} {checks.filter((c) => c.ok).length}/{checks.length}
          </button>
        )}
      </div>
      <div className="tabs">
        {(
          [
            ["body", "Тело"],
            ...(hasChecks ? [["checks", failed ? `Проверки ✗${failed}` : checks.length + captures.length + before.length ? "Проверки" : "Журнал"]] : []),
            ["headers", `Заголовки ${meta.headers.length}`],
            ["time", "Время"],
            ["sent", "Отправлено"],
          ] as [Tab, string][]
        ).map(([id, label]) => (
          <button key={id} className={tab === id ? "on" : ""} onClick={() => setTab(id)}>
            {label}
          </button>
        ))}
      </div>
      <div className="tab-body">
        {tab === "body" && <BodyViewer key={meta.responseId} meta={meta} />}
        {tab === "checks" && <ChecksResult meta={meta} />}
        {tab === "headers" && <HeaderTable headers={meta.headers} />}
        {tab === "time" && <TimingChart meta={meta} />}
        {tab === "sent" && (
          <div className="scroll pad">
            <div className="mono sel sent-line">
              <b>{meta.sent.method}</b> {meta.sent.url}
            </div>
            <HeaderTable headers={meta.sent.headers} />
            <div className="faint small pad-top">Тело: {meta.sent.bodySize ? size(meta.sent.bodySize) : "нет"}</div>
          </div>
        )}
      </div>
    </div>
  );
}

/** Requests sent first, the checks with what came, and the values kept for other requests. */
function ChecksResult({ meta }: { meta: ResponseMeta }) {
  const before = meta.before ?? [];
  const checks = meta.checks ?? [];
  const captures = meta.captures ?? [];
  return (
    <div className="scroll pad check-results">
      {before.length > 0 && (
        <>
          <div className="section-head">Сначала отправлены — за значениями для этого запроса</div>
          {before.map((b, i) => (
            <div key={i} className="check-row">
              <span className={b.passed ? "good-text" : "bad-text"}>{b.passed ? "✓" : "✗"}</span>
              <span className="mono">{b.id}</span>
              {b.status != null && <span className={`hist-status ${statusTone(b.status)}`}>{b.status}</span>}
              {b.failures.map((f, j) => (
                <div key={j} className="check-why bad-text small">
                  {f}
                </div>
              ))}
            </div>
          ))}
        </>
      )}
      {checks.length > 0 && <div className="section-head">Проверки</div>}
      {checks.map((c, i) => (
        <div key={i} className="check-row">
          <span className={c.ok ? "good-text" : "bad-text"}>{c.ok ? "✓" : "✗"}</span>
          <span className="mono sel">{c.check}</span>
          {(c.error || c.actual != null) && (
            <div className={`check-why small ${c.ok ? "faint" : "bad-text"}`}>
              {c.error ?? (c.ok ? c.actual : `пришло ${c.actual}`)}
            </div>
          )}
        </div>
      ))}
      {captures.length > 0 && <div className="section-head">Значения из ответа</div>}
      {captures.map((c, i) => (
        <div key={i} className="check-row">
          <span className={c.error ? "bad-text" : "good-text"}>{c.error ? "✗" : "→"}</span>
          <span className="mono">{`{{${c.name}}}`}</span>
          <div className={`check-why small mono sel ${c.error ? "bad-text" : "faint"}`}>{c.error ?? short(c.value ?? "")}</div>
        </div>
      ))}
      {(meta.logs ?? []).length > 0 && (
        <>
          <div className="section-head">Журнал скриптов</div>
          <pre className="script-log mono small sel">{(meta.logs ?? []).join("\n")}</pre>
        </>
      )}
    </div>
  );
}

const short = (text: string) => (text.length > 300 ? `${text.slice(0, 300)}…` : text);

function HeaderTable({ headers }: { headers: [string, string][] }) {
  return (
    <div className="scroll">
      <table className="kv sel">
        <tbody>
          {headers.map(([name, value], i) => (
            <tr key={i}>
              <td className="mono k">{name}</td>
              <td className="mono">{value}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Phases as bars on one time line, like the network panel of a browser. */
function TimingChart({ meta }: { meta: ResponseMeta }) {
  const t = meta.timings;
  const phases: [string, number, string][] = [
    ["DNS", t.dns, "p-dns"],
    ["Подключение", t.connect, "p-tcp"],
    ...(t.tls != null ? ([["TLS", t.tls, "p-tls"]] as [string, number, string][]) : []),
    ["Ожидание ответа", t.wait, "p-wait"],
    ["Загрузка", t.download, "p-down"],
  ];
  const total = Math.max(t.total, phases.reduce((s, [, v]) => s + v, 0), 0.001);
  let start = 0;
  return (
    <div className="scroll pad">
      <div className="timing">
        {phases.map(([label, value, cls]) => {
          const left = (start / total) * 100;
          start += value;
          return (
            <div className="timing-row" key={label}>
              <span className="muted">{label}</span>
              <span className="timing-track">
                <span className={`timing-bar ${cls}`} style={{ left: `${left}%`, width: `max(2px, ${(value / total) * 100}%)` }} />
              </span>
              <span className="num mono">{ms(value)}</span>
            </div>
          );
        })}
        <div className="timing-row total">
          <span>Всего</span>
          <span />
          <span className="num mono">{ms(t.total)}</span>
        </div>
      </div>
      <table className="kv small">
        <tbody>
          <tr>
            <td className="k">Адрес</td>
            <td className="mono sel">{meta.url}</td>
          </tr>
          <tr>
            <td className="k">Сервер</td>
            <td className="mono sel">{meta.remoteAddr}</td>
          </tr>
          {meta.tls && (
            <tr>
              <td className="k">TLS</td>
              <td className="mono">
                {meta.tls.version}
                {meta.tls.alpn ? ` · ${meta.tls.alpn}` : ""}
              </td>
            </tr>
          )}
          {meta.redirects.map((r, i) => (
            <tr key={i}>
              <td className="k">{i === 0 ? "Перенаправления" : ""}</td>
              <td className="mono sel">
                {r.status} {r.url}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
