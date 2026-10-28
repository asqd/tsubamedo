// Sent requests, newest first, grouped by day. An entry opens as a scratch tab with the
// request as it was and, when the body was kept (up to 1 MB), the answer as it came.
import { useEffect, useState } from "react";
import { api, HistoryItem } from "../api";
import { ms, size, statusTone } from "../format";
import { lang, t } from "../i18n";
import { ContextMenu } from "./ContextMenu";

function day(at: number): string {
  const d = new Date(at);
  const today = new Date();
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((start(today) - start(d)) / 86_400_000);
  if (diff === 0) return t("Сегодня", "Today");
  if (diff === 1) return t("Вчера", "Yesterday");
  return d.toLocaleDateString(locale(), { day: "numeric", month: "long", year: d.getFullYear() === today.getFullYear() ? undefined : "numeric" });
}

const locale = () => (lang() === "en" ? "en-US" : "ru-RU");

const time = (at: number) => new Date(at).toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit", hour12: false });

export function HistoryList({
  version,
  currentId,
  filter,
  onOpen,
}: {
  /** Changes after every send, so the list reads the file again. */
  version: number;
  /** The request of the active tab, for "только этот запрос". */
  currentId: string | null;
  filter: string;
  onOpen: (entryId: string) => void;
}) {
  const [items, setItems] = useState<HistoryItem[] | null>(null);
  const [onlyThis, setOnlyThis] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [cleared, setCleared] = useState(0);

  const scope = onlyThis && currentId ? currentId : null;
  useEffect(() => {
    let alive = true;
    api
      .history(scope)
      .then((list) => alive && (setItems(list), setError(null)))
      .catch((e) => alive && setError((e as Error).message));
    return () => {
      alive = false;
    };
  }, [version, scope, cleared]);

  const f = filter.trim().toLowerCase();
  const shown = (items ?? []).filter((h) => !f || h.url.toLowerCase().includes(f) || (h.name ?? "").toLowerCase().includes(f) || h.method.toLowerCase() === f);
  let lastDay = "";

  return (
    <div className="history">
      <div className="history-head small">
        <label className="muted" title={currentId ? undefined : t("Откройте запрос из коллекции", "Open a request from the collection")}>
          <input type="checkbox" checked={onlyThis} disabled={!currentId} onChange={(e) => setOnlyThis(e.target.checked)} /> {t("только этот запрос", "only this request")}
        </label>
        <span className="sp" />
        <button className="icon" title={t("Очистить историю", "Clear history")} disabled={!items?.length} onClick={(e) => setMenu({ x: e.clientX, y: e.clientY })}>
          ⌫
        </button>
      </div>
      {error && <div className="side-empty small bad-text">{error}</div>}
      {items && shown.length === 0 && <div className="side-empty small faint">{f ? t("Ничего не нашлось", "Nothing found") : t("Здесь появятся отправленные запросы", "Sent requests will appear here")}</div>}
      <div className="history-list">
        {shown.map((h) => {
          const d = day(h.at);
          const header = d !== lastDay ? <div className="folder">{d}</div> : null;
          lastDay = d;
          return (
            <div key={h.id}>
              {header}
              <button className="item hist" onClick={() => onOpen(h.id)} title={`${h.method} ${h.url}${h.env ? t(`\nокружение: ${h.env}`, `\nenvironment: ${h.env}`) : ""}${h.hasBody ? "" : t("\nтело ответа не сохранено", "\nresponse body not kept")}`}>
                <span className="faint small num hist-time">{time(h.at)}</span>
                <span className={`m m-${h.method.toLowerCase()}`}>{h.method === "DELETE" ? "DEL" : h.method.slice(0, 4)}</span>
                <span className="ell hist-name">{h.name || h.url.replace(/^\{\{[^}]+\}\}|^https?:\/\/[^/]+/, "") || h.url}</span>
                {h.status != null ? (
                  <span className={`hist-status ${statusTone(h.status)}`}>{h.status}</span>
                ) : (
                  <span className="hist-status bad" title={h.error ?? ""}>
                    {t("ошибка", "error")}
                  </span>
                )}
                <span className="faint small num hist-meta">
                  {h.totalMs != null ? ms(h.totalMs) : ""}
                  {h.size != null ? ` · ${size(h.size)}` : ""}
                </span>
              </button>
            </div>
          );
        })}
      </div>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            {
              label: t("Очистить историю", "Clear history"),
              danger: true,
              confirm: t(
                "Удалить всю историю этой коллекции вместе с сохранёнными ответами, значениями из них ({{token}} и подобными), cookie и токенами OAuth 2.0?",
                "Delete this collection's whole history, with the kept responses, the values taken from them ({{token}} and the like), cookies and OAuth 2.0 tokens?",
              ),
              onClick: () => api.clearHistory().then(() => setCleared((c) => c + 1)),
            },
          ]}
        />
      )}
    </div>
  );
}
