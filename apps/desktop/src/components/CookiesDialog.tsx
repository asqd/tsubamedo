// The environment's cookie jar: what the servers set and the app sends back, by site.
import { useEffect, useState } from "react";
import { api, Cookie } from "../api";
import { lang, t } from "../i18n";

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
      <div className="modal cookies-dialog" role="dialog" aria-label={t("Cookie", "Cookies")} onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <h2>{env ? t(`Cookie окружения ${env}`, `Cookies of ${env}`) : t("Cookie без окружения", "Cookies without an environment")}</h2>
        <p className="faint small">
          {t(
            "Что поставили серверы. Уходят обратно, как в браузере: на свой сайт и путь, Secure — только по HTTPS (и на этот компьютер). Хранятся в папке данных приложения, у каждого окружения свои.",
            "What the servers set. They go back as in a browser: to their own site and path, Secure ones only over HTTPS (and to this computer). Kept in the app's data folder, separately for each environment.",
          )}
        </p>
        {problem && <div className="error-box small">{problem}</div>}
        {cookies && cookies.length === 0 && <p className="muted">{t("Пусто: ни один ответ не ставил cookie.", "Empty: no response has set a cookie.")}</p>}
        <div className="cookie-list">
          {[...sites].map(([domain, list]) => (
            <div key={domain}>
              <div className="cookie-site small">
                <b>{domain}</b>
                {list[0].hostOnly ? "" : <span className="faint">{t(" и поддомены", " and subdomains")}</span>}
              </div>
              {list.map((c) => (
                <div key={`${c.path} ${c.name}`} className="cookie-row small">
                  <span className="mono ell" title={`${c.name}=${c.value}`}>
                    {c.name}=<span className="muted">{c.value}</span>
                  </span>
                  <span className="faint">{c.path}</span>
                  <span className="faint" title={c.expires ? new Date(c.expires * 1000).toLocaleString(locale()) : undefined}>
                    {until(c.expires)}
                  </span>
                  <span className="faint">{[c.secure && "Secure", c.httpOnly && "HttpOnly", c.sameSite].filter(Boolean).join(" ")}</span>
                  <button className="icon" title={t("Удалить", "Delete")} onClick={() => api.deleteCookie(env, c).then(load)}>
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
              {t("Удалить все", "Delete all")}
            </button>
          )}
          <span className="sp" />
          <button className="primary" autoFocus onClick={onClose}>
            {t("Готово", "Done")}
          </button>
        </div>
      </div>
    </div>
  );
}

const locale = () => (lang() === "en" ? "en-US" : "ru");

function until(expires: number | null): string {
  if (expires == null) return t("пока не удалите", "until deleted");
  const left = expires - Date.now() / 1000;
  if (left < 3600) {
    const min = Math.max(1, Math.round(left / 60));
    return t(`ещё ${min} мин`, `${min} min left`);
  }
  if (left < 2 * 86400) {
    const h = Math.round(left / 3600);
    return t(`ещё ${h} ч`, `${h} h left`);
  }
  const date = new Date(expires * 1000).toLocaleDateString(locale());
  return t(`до ${date}`, `until ${date}`);
}
