// The environment's cookie jar: what the servers set and the app sends back, by site.
import { useEffect, useState } from "react";
import { api, Cookie } from "../api";

export function CookiesDialog({ env, onClose }: { env: string | null; onClose: () => void }) {
  const [cookies, setCookies] = useState<Cookie[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const load = () => api.cookies(env).then(setCookies, (e) => setProblem((e as Error).message));
  useEffect(() => {
    load();
  }, [env]);

  const sites = new Map<string, Cookie[]>();
  for (const c of cookies ?? []) sites.set(c.domain, [...(sites.get(c.domain) ?? []), c]);

  return (
    <div className="modal-back" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal cookies-dialog" role="dialog" aria-label="Cookie" onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <h2>Cookie {env ? `окружения ${env}` : "без окружения"}</h2>
        <p className="faint small">
          Что поставили серверы. Уходят обратно, как в браузере: на свой сайт и путь, Secure — только по HTTPS (и на этот компьютер). Хранятся в
          папке данных приложения, у каждого окружения свои.
        </p>
        {problem && <div className="error-box small">{problem}</div>}
        {cookies && cookies.length === 0 && <p className="muted">Пусто: ни один ответ не ставил cookie.</p>}
        <div className="cookie-list">
          {[...sites].map(([domain, list]) => (
            <div key={domain}>
              <div className="cookie-site small">
                <b>{domain}</b>
                {list[0].hostOnly ? "" : <span className="faint"> и поддомены</span>}
              </div>
              {list.map((c) => (
                <div key={`${c.path} ${c.name}`} className="cookie-row small">
                  <span className="mono ell" title={`${c.name}=${c.value}`}>
                    {c.name}=<span className="muted">{c.value}</span>
                  </span>
                  <span className="faint">{c.path}</span>
                  <span className="faint" title={c.expires ? new Date(c.expires * 1000).toLocaleString("ru") : undefined}>
                    {until(c.expires)}
                  </span>
                  <span className="faint">{[c.secure && "Secure", c.httpOnly && "HttpOnly", c.sameSite].filter(Boolean).join(" ")}</span>
                  <button className="icon" title="Удалить" onClick={() => api.deleteCookie(env, c).then(load)}>
                    ×
                  </button>
                </div>
              ))}
            </div>
          ))}
        </div>
        <div className="modal-actions">
          {cookies && cookies.length > 0 && (
            <button className="ghost" onClick={() => api.clearCookies(env).then(load)}>
              Удалить все
            </button>
          )}
          <span className="sp" />
          <button className="primary" autoFocus onClick={onClose}>
            Готово
          </button>
        </div>
      </div>
    </div>
  );
}

function until(expires: number | null): string {
  if (expires == null) return "пока не удалите";
  const left = expires - Date.now() / 1000;
  if (left < 3600) return `ещё ${Math.max(1, Math.round(left / 60))} мин`;
  if (left < 2 * 86400) return `ещё ${Math.round(left / 3600)} ч`;
  return `до ${new Date(expires * 1000).toLocaleDateString("ru")}`;
}
