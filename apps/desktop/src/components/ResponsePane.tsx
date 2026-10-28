// The answer: status line, then body, headers, time by phase and what was actually sent.
import { useState } from "react";
import { ResponseMeta } from "../api";
import { ms, size, statusTone } from "../format";
import { t } from "../i18n";
import { BodyViewer } from "./BodyViewer";

type Tab = "body" | "checks" | "headers" | "time" | "sent";

export function ResponsePane({
  meta,
  error,
  sending,
  onCancel,
  onCompare,
}: {
  meta: ResponseMeta | null;
  error: string | null;
  sending: boolean;
  onCancel: () => void;
  /** Where to open the menu of what to compare with. */
  onCompare?: (x: number, y: number) => void;
}) {
  const [tab, setTab] = useState<Tab>("body");

  if (sending) {
    return (
      <div className="pane-empty">
        <span className="spin big" />
        <div className="muted">{t("Отправка…", "Sending…")}</div>
        <button onClick={onCancel}>{t("Отменить", "Cancel")}</button>
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
        <div>{t("Ответ появится здесь", "The response will appear here")}</div>
        <div className="small">{t("⌘↵ — отправить запрос", "⌘↵ — send the request")}</div>
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
        <span className="metric" title={t("Всё время запроса", "Total request time")}>
          {ms(meta.timings.total)}
        </span>
        <span className="metric" title={compressed ? t(`По сети ${size(meta.wireSize)}`, `Over the network ${size(meta.wireSize)}`) : undefined}>
          {size(meta.size)}
          {compressed && <span className="faint">{t(` · сжато ${size(meta.wireSize)}`, ` · compressed ${size(meta.wireSize)}`)}</span>}
        </span>
        <span className="faint small">{meta.version}</span>
        {meta.redirects.length > 0 && (
          <span className="faint small" title={meta.redirects.map((r) => `${r.status} ${r.url}`).join("\n")}>
            {meta.redirects.length} {t("перенаправл.", meta.redirects.length === 1 ? "redirect" : "redirects")}
          </span>
        )}
        {checks.length > 0 && (
          <button className={`check-badge ${failed ? "bad" : "ok"}`} onClick={() => setTab("checks")} title={t("Проверки ответа", "Response checks")}>
            {failed ? "✗" : "✓"} {checks.filter((c) => c.ok).length}/{checks.length}
          </button>
        )}
        {onCompare && (
          <button
            className="ghost small compare-btn"
            title={t("Сравнить с ответом из другого окружения или с прошлым", "Compare with a response from another environment or an earlier one")}
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              onCompare(r.left, r.bottom + 4);
            }}
          >
            {t("Сравнить…", "Compare…")}
          </button>
        )}
      </div>
      <div className="tabs">
        {(
          [
            ["body", t("Тело", "Body")],
            ...(hasChecks
              ? [["checks", failed ? `${t("Проверки", "Checks")} ✗${failed}` : checks.length + captures.length + before.length ? t("Проверки", "Checks") : t("Журнал", "Log")]]
              : []),
            ["headers", `${t("Заголовки", "Headers")} ${meta.headers.length}`],
            ["time", t("Время", "Timing")],
            ["sent", t("Отправлено", "Sent")],
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
            <div className="faint small pad-top">{t("Тело", "Body")}: {meta.sent.bodySize ? size(meta.sent.bodySize) : t("нет", "none")}</div>
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
          <div className="section-head">{t("Сначала отправлены — за значениями для этого запроса", "Sent first — for values this request needs")}</div>
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
      {checks.length > 0 && <div className="section-head">{t("Проверки", "Checks")}</div>}
      {checks.map((c, i) => (
        <div key={i} className="check-row">
          <span className={c.ok ? "good-text" : "bad-text"}>{c.ok ? "✓" : "✗"}</span>
          <span className="mono sel">{c.check}</span>
          {(c.error || c.actual != null) && (
            <div className={`check-why small ${c.ok ? "faint" : "bad-text"}`}>
              {c.error ?? (c.ok ? c.actual : t(`пришло ${c.actual}`, `got ${c.actual}`))}
            </div>
          )}
        </div>
      ))}
      {captures.length > 0 && <div className="section-head">{t("Значения из ответа", "Values from the response")}</div>}
      {captures.map((c, i) => (
        <div key={i} className="check-row">
          <span className={c.error ? "bad-text" : "good-text"}>{c.error ? "✗" : "→"}</span>
          <span className="mono">{`{{${c.name}}}`}</span>
          <div className={`check-why small mono sel ${c.error ? "bad-text" : "faint"}`}>{c.error ?? short(c.value ?? "")}</div>
        </div>
      ))}
      {(meta.logs ?? []).length > 0 && (
        <>
          <div className="section-head">{t("Журнал: скрипты и авторизация", "Log: scripts and authorization")}</div>
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
  const tm = meta.timings;
  const phases: [string, number, string][] = [
    ["DNS", tm.dns, "p-dns"],
    [t("Подключение", "Connect"), tm.connect, "p-tcp"],
    ...(tm.tls != null ? ([["TLS", tm.tls, "p-tls"]] as [string, number, string][]) : []),
    [t("Ожидание ответа", "Waiting"), tm.wait, "p-wait"],
    [t("Загрузка", "Download"), tm.download, "p-down"],
  ];
  const total = Math.max(tm.total, phases.reduce((s, [, v]) => s + v, 0), 0.001);
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
          <span>{t("Всего", "Total")}</span>
          <span />
          <span className="num mono">{ms(tm.total)}</span>
        </div>
      </div>
      <table className="kv small">
        <tbody>
          <tr>
            <td className="k">{t("Адрес", "URL")}</td>
            <td className="mono sel">{meta.url}</td>
          </tr>
          {!meta.via && (
            <tr>
              <td className="k">{t("Сервер", "Server")}</td>
              <td className="mono sel">{meta.remoteAddr}</td>
            </tr>
          )}
          {meta.tls && (
            <tr>
              <td className="k">TLS</td>
              <td className="mono">
                {meta.tls.version}
                {meta.tls.alpn ? ` · ${meta.tls.alpn}` : ""}
                {meta.tls.clientCert ? t(` · клиентский сертификат ${meta.tls.clientCert}`, ` · client certificate ${meta.tls.clientCert}`) : ""}
              </td>
            </tr>
          )}
          {meta.via && (
            <tr>
              <td className="k">{t("Прокси", "Proxy")}</td>
              <td className="mono sel">{meta.via}</td>
            </tr>
          )}
          {meta.redirects.map((r, i) => (
            <tr key={i}>
              <td className="k">{i === 0 ? t("Перенаправления", "Redirects") : ""}</td>
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
