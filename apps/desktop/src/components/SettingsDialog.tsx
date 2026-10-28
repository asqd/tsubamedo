// Settings: the language and the proxy (the app's, for every collection on this machine) and the
// client certificates of the open collection (in its tsubamedo.toml, chosen by host).
import { useEffect, useState } from "react";
import { api, CertEntry, CertSpec, CollectionInfo, ProxyConfig, SystemProxy } from "../api";
import { chooseLang, LangChoice, systemLang, t } from "../i18n";
import { home, pickCertFile } from "../platform";

export function SettingsDialog({ collection, env, onClose }: { collection: CollectionInfo | null; env: string | null; onClose: () => void }) {
  const [proxy, setProxy] = useState<ProxyConfig | null>(null);
  const [system, setSystem] = useState<SystemProxy | null>(null);
  const [certs, setCerts] = useState<CertSpec[] | null>(null);
  const [status, setStatus] = useState<CertEntry["status"][]>([]);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [language, setLanguage] = useState<LangChoice | null>(null);

  useEffect(() => {
    api.language().then((l) => setLanguage(l.language), fail);
    api.proxySettings().then(({ proxy, system }) => {
      setProxy(proxy);
      setSystem(system);
    }, fail);
    if (collection) api.certificates(env).then(showCerts, fail);
  }, [collection, env]);

  function fail(e: unknown) {
    setProblem((e as Error).message);
  }
  function showCerts(list: CertEntry[]) {
    setCerts(list.map((c) => c.spec));
    setStatus(list.map((c) => c.status));
  }

  const custom = proxy?.mode === "custom" ? proxy : { mode: "custom" as const, url: "", noProxy: "" };
  const setCert = (i: number, patch: Partial<CertSpec>) => {
    setCerts((list) => list && list.map((c, j) => (j === i ? { ...c, ...patch } : c)));
    setStatus((s) => s.map((x, j) => (j === i ? { ok: true, text: "" } : x)));
  };
  // A file inside the collection is written relative to it, so the path works on any machine.
  const relative = (path: string) => (collection && path.startsWith(`${collection.root}/`) ? path.slice(collection.root.length + 1) : path);

  const save = async () => {
    setBusy(true);
    setProblem(null);
    try {
      if (proxy) await api.saveProxy(proxy);
      if (collection && certs) {
        const kept = certs.filter((c) => c.host.trim() || c.cert.trim());
        const checked = await api.saveCertificates(env, kept);
        showCerts(checked);
        // Stay open when a certificate does not read: the reason is next to it.
        if (checked.some((c) => !c.status.ok)) return;
      }
      onClose();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-back" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal settings-dialog" role="dialog" aria-label={t("Настройки", "Settings")} onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <h2>{t("Язык", "Language")}</h2>
        {language && (
          <fieldset className="choice">
            {(
              [
                ["auto", t(`Как в системе — ${systemLang() === "ru" ? "русский" : "английский"}`, `As the system — ${systemLang() === "ru" ? "Russian" : "English"}`)],
                ["ru", "Русский"],
                ["en", "English"],
              ] as [LangChoice, string][]
            ).map(([value, label]) => (
              <label key={value}>
                <input type="radio" checked={language === value} disabled={busy} onChange={() => (setBusy(true), chooseLang(value).catch(fail))} />
                <span>{label}</span>
              </label>
            ))}
          </fieldset>
        )}
        <h2 className="section-gap">{t("Прокси", "Proxy")}</h2>
        <p className="faint small">
          {t(
            "Для всех коллекций на этом компьютере. Запросы на localhost и 127.0.0.1 всегда идут напрямую.",
            "For every collection on this computer. Requests to localhost and 127.0.0.1 always go direct.",
          )}
        </p>
        {proxy && (
          <fieldset className="choice">
            <label>
              <input type="radio" checked={proxy.mode === "system"} onChange={() => setProxy({ mode: "system" })} />
              <span>
                {t("Системный", "System")}
                <span className="faint small"> — {describeSystem(system)}</span>
              </span>
            </label>
            <label>
              <input type="radio" checked={proxy.mode === "custom"} onChange={() => setProxy(custom)} />
              <span>{t("Свой", "Custom")}</span>
            </label>
            {proxy.mode === "custom" && (
              <div className="form proxy-form">
                <label>
                  <span>{t("Адрес", "Address")}</span>
                  <input
                    className="mono"
                    value={proxy.url}
                    placeholder={t("proxy.corp:3128, http://логин:пароль@proxy:3128, socks5://proxy:1080", "proxy.corp:3128, http://user:password@proxy:3128, socks5://proxy:1080")}
                    spellCheck={false}
                    onChange={(e) => setProxy({ ...proxy, url: e.target.value })}
                  />
                </label>
                <label>
                  <span>{t("Без прокси", "No proxy")}</span>
                  <input
                    className="mono"
                    value={proxy.noProxy}
                    placeholder=".corp.local, 10.0.0.0/8, internal:8080"
                    spellCheck={false}
                    onChange={(e) => setProxy({ ...proxy, noProxy: e.target.value })}
                  />
                </label>
              </div>
            )}
            <label>
              <input type="radio" checked={proxy.mode === "none"} onChange={() => setProxy({ mode: "none" })} />
              <span>{t("Без прокси", "No proxy")}</span>
            </label>
          </fieldset>
        )}

        <h2 className="section-gap">{t("Клиентские сертификаты", "Client certificates")}</h2>
        {!collection ? (
          <p className="faint small">{t("Сертификаты задаются у коллекции — откройте её.", "Certificates belong to a collection — open one.")}</p>
        ) : (
          <>
            <p className="faint small">
              {t(
                "Для серверов, которые спрашивают сертификат (mutual TLS). Хранятся в tsubamedo.toml коллекции, поэтому пароль лучше записать переменной ",
                "For servers that ask for a certificate (mutual TLS). They are kept in the collection's tsubamedo.toml, so better write the password as a variable ",
              )}
              <code>{"{{cert_password}}"}</code>
              {t(
                ", а значение — в tsubamedo.local.toml. PEM (ключ в том же файле или отдельно) или .p12/.pfx.",
                " and put its value in tsubamedo.local.toml. PEM (key in the same file or separate) or .p12/.pfx.",
              )}
            </p>
            {(certs ?? []).map((c, i) => (
              <div key={i} className="cert-card">
                <div className="form">
                  <label>
                    <span>{t("Хост", "Host")}</span>
                    <input className="mono" value={c.host} placeholder="api.bank.test, *.bank.test, host:8443" spellCheck={false} onChange={(e) => setCert(i, { host: e.target.value })} />
                  </label>
                  <FileField
                    label={t("Сертификат", "Certificate")}
                    value={c.cert}
                    placeholder={t("certs/client.pem или client.p12", "certs/client.pem or client.p12")}
                    onChange={(cert) => setCert(i, { cert })}
                    onPick={async () => {
                      const path = await pickCertFile(t("Клиентский сертификат", "Client certificate"));
                      if (path) setCert(i, { cert: relative(path) });
                    }}
                  />
                  <FileField
                    label={t("Ключ", "Key")}
                    value={c.key}
                    placeholder={t("если ключ в отдельном файле", "if the key is in a separate file")}
                    onChange={(key) => setCert(i, { key })}
                    onPick={async () => {
                      const path = await pickCertFile(t("Закрытый ключ", "Private key"));
                      if (path) setCert(i, { key: relative(path) });
                    }}
                  />
                  <label>
                    <span>{t("Пароль", "Password")}</span>
                    <input className="mono" value={c.passphrase} placeholder="{{cert_password}}" spellCheck={false} onChange={(e) => setCert(i, { passphrase: e.target.value })} />
                  </label>
                  {status[i]?.text && <div className={`hint ${status[i].ok ? "good-text" : "bad-text"}`}>{(status[i].ok ? "✓ " : "✗ ") + status[i].text}</div>}
                </div>
                <button className="icon" title={t("Убрать сертификат", "Remove certificate")} onClick={() => {
                  setCerts((list) => list && list.filter((_, j) => j !== i));
                  setStatus((s) => s.filter((_, j) => j !== i));
                }}>
                  ×
                </button>
              </div>
            ))}
            <button className="ghost small" onClick={() => setCerts((list) => [...(list ?? []), { host: "", cert: "", key: "", passphrase: "" }])}>
              + {t("Добавить сертификат", "Add certificate")}
            </button>
          </>
        )}
        {problem && <div className="error-box small">{problem}</div>}
        <div className="modal-actions">
          <button onClick={onClose} disabled={busy}>
            {t("Отмена", "Cancel")}
          </button>
          <button className="primary" onClick={save} disabled={busy}>
            {busy ? t("Проверяю…", "Checking…") : t("Сохранить", "Save")}
          </button>
        </div>
      </div>
    </div>
  );
}

function FileField({ label, value, placeholder, onChange, onPick }: { label: string; value: string; placeholder: string; onChange: (v: string) => void; onPick: () => void }) {
  return (
    <label>
      <span>{label}</span>
      <span className="file-field">
        <input className="mono" value={value} placeholder={placeholder} spellCheck={false} title={home(value)} onChange={(e) => onChange(e.target.value)} />
        <button type="button" className="small" onClick={onPick}>
          {t("Выбрать…", "Choose…")}
        </button>
      </span>
    </label>
  );
}

function describeSystem(s: SystemProxy | null): string {
  if (!s) return "…";
  const parts: string[] = [];
  if (s.http && s.http === s.https) parts.push(t(`${s.http} для HTTP и HTTPS`, `${s.http} for HTTP and HTTPS`));
  else {
    if (s.http) parts.push(t(`${s.http} для HTTP`, `${s.http} for HTTP`));
    if (s.https) parts.push(t(`${s.https} для HTTPS`, `${s.https} for HTTPS`));
  }
  if (s.socks) parts.push(`SOCKS ${s.socks.replace("socks5://", "")}`);
  if (s.pac != null) parts.push(t("автоматическая настройка (PAC) не выполняется", "automatic configuration (PAC) is not run"));
  if (parts.length === 0) return t("сейчас не задан, запросы идут напрямую", "not set now, requests go direct");
  const bypass = s.bypass.length ? t(` · без прокси: ${s.bypass.join(", ")}`, ` · no proxy for: ${s.bypass.join(", ")}`) : "";
  return `${parts.join(", ")}${bypass} (${s.source})`;
}
