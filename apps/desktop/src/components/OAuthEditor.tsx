// OAuth 2.0 settings of a request and its token: the core gets it before sending, keeps it in
// the app's data folder and renews it; here it can be got anew or forgotten.
import { useEffect, useRef, useState } from "react";
import { api, OAuth2, Request, TokenInfo } from "../api";
import { CodeEditor } from "./CodeEditor";

type Settings = { type: "oauth2" } & OAuth2;

export const freshOAuth = (): Settings => ({
  type: "oauth2",
  grant: "authorization_code",
  authUrl: "",
  tokenUrl: "",
  clientId: "",
  clientSecret: "",
  scope: "",
  audience: "",
  redirectUri: "",
  pkce: true,
  clientAuth: "basic",
  username: "",
  password: "",
});

export function OAuthEditor({
  auth,
  request,
  env,
  sending,
  onChange,
}: {
  auth: Settings;
  request: Request;
  env: string | null;
  sending: boolean;
  onChange: (a: Settings) => void;
}) {
  const set = (patch: Partial<OAuth2>) => onChange({ ...auth, ...patch });
  const code = auth.grant === "authorization_code";
  const [token, setToken] = useState<TokenInfo | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const settings = JSON.stringify(auth);
  const requestRef = useRef(request);
  requestRef.current = request;

  // The kept token for these settings: again when they change and after each send.
  useEffect(() => {
    if (sending) return;
    let live = true;
    const timer = setTimeout(() => {
      api.oauthStatus(requestRef.current, env).then(
        (t) => live && setToken(t),
        () => live && setToken(null),
      );
    }, 250);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [settings, env, sending]);

  const getNew = async () => {
    const sendId = `oauth-${Date.now()}`;
    setBusy(sendId);
    setProblem(null);
    try {
      setToken(await api.oauthToken(sendId, request, env));
    } catch (e) {
      const message = (e as Error).message;
      if (message !== "Отменено") setProblem(message);
    } finally {
      setBusy(null);
    }
  };
  const forget = async () => {
    await api.oauthForget(request, env);
    setToken(null);
  };

  return (
    <>
      <label>
        <span>Как получить токен</span>
        <select value={auth.grant} onChange={(e) => set({ grant: e.target.value as OAuth2["grant"] })}>
          <option value="authorization_code">Вход в браузере (authorization code)</option>
          <option value="client_credentials">Client credentials</option>
          <option value="password">Логин и пароль (password)</option>
        </select>
      </label>
      {code && <Field label="Адрес входа" value={auth.authUrl} onChange={(authUrl) => set({ authUrl })} placeholder="https://id.example.com/authorize" />}
      <Field label="Адрес токена" value={auth.tokenUrl} onChange={(tokenUrl) => set({ tokenUrl })} placeholder="https://id.example.com/oauth/token" />
      <Field label="Client ID" value={auth.clientId} onChange={(clientId) => set({ clientId })} placeholder="{{client_id}}" />
      <Field
        label="Client secret"
        value={auth.clientSecret}
        onChange={(clientSecret) => set({ clientSecret })}
        placeholder={code ? "не нужен приложению с PKCE" : "{{client_secret}}"}
      />
      {auth.grant === "password" && (
        <>
          <Field label="Логин" value={auth.username} onChange={(username) => set({ username })} />
          <Field label="Пароль" value={auth.password} onChange={(password) => set({ password })} placeholder="{{password}}" />
        </>
      )}
      <Field label="Scope" value={auth.scope} onChange={(scope) => set({ scope })} placeholder="через пробел: orders:read profile" />
      <details className="oauth-more">
        <summary className="faint small">Ещё: audience, адрес возврата, PKCE, как передавать секрет</summary>
        <div className="form">
          <Field label="Audience" value={auth.audience} onChange={(audience) => set({ audience })} placeholder="нужен Auth0 и некоторым другим" />
          {code && (
            <>
              <Field
                label="Адрес возврата"
                value={auth.redirectUri}
                onChange={(redirectUri) => set({ redirectUri })}
                placeholder="http://127.0.0.1:<свободный порт>/callback"
              />
              <label>
                <span>PKCE</span>
                <select value={auth.pkce ? "on" : "off"} onChange={(e) => set({ pkce: e.target.value === "on" })}>
                  <option value="on">да, S256</option>
                  <option value="off">нет</option>
                </select>
              </label>
            </>
          )}
          <label>
            <span>Client ID и секрет</span>
            <select value={auth.clientAuth} onChange={(e) => set({ clientAuth: e.target.value as OAuth2["clientAuth"] })}>
              <option value="basic">в заголовке Authorization: Basic</option>
              <option value="body">в теле запроса токена</option>
            </select>
          </label>
        </div>
      </details>
      <div className="oauth-token">
        <div className="small">
          {token ? (
            <>
              <span className="good-text">✓</span> Токен <code>{token.preview}</code>
              <span className="muted"> · {describe(token)}</span>
            </>
          ) : (
            <span className="muted">Токена ещё нет — получу при отправке.</span>
          )}
        </div>
        <div className="oauth-actions">
          {busy ? (
            <>
              <span className="faint small">{code ? "Жду вход в браузере…" : "Получаю…"}</span>
              <button className="small" onClick={() => api.cancel(busy)}>
                Отмена
              </button>
            </>
          ) : (
            <>
              <button className="small" onClick={getNew} disabled={sending} title="Получить токен заново, не дожидаясь, пока этот истечёт">
                {token ? "Получить новый" : "Получить сейчас"}
              </button>
              {token && (
                <button className="small ghost" onClick={forget} title="Удалить сохранённый токен">
                  Забыть
                </button>
              )}
            </>
          )}
        </div>
        {problem && <div className="error-box small">{problem}</div>}
      </div>
      <div className="hint">
        {code
          ? "Браузер откроет страницу входа, а код вернётся на этот компьютер. По RFC 8252 провайдер принимает любой порт на 127.0.0.1; если ему нужен точный адрес — впишите его в «Адрес возврата» и добавьте у провайдера."
          : "Токен запрашивается перед первой отправкой и живёт, пока не истечёт."}{" "}
        Истёкший обновляется по refresh token; если сервер ответил 401 на сохранённый, запрос уйдёт ещё раз с новым. Токены хранятся в папке
        данных приложения, не в коллекции.
      </div>
    </>
  );
}

function describe(t: TokenInfo): string {
  const parts = [`получен ${new Date(t.obtainedAt * 1000).toLocaleTimeString("ru", { hour: "2-digit", minute: "2-digit" })}`];
  if (t.expiresAt != null) {
    const left = t.expiresAt - Date.now() / 1000;
    parts.push(left <= 0 ? "истёк" : left < 120 ? `ещё ${Math.round(left)} с` : left < 7200 ? `ещё ${Math.round(left / 60)} мин` : `ещё ${Math.round(left / 3600)} ч`);
  }
  if (t.hasRefresh) parts.push("есть refresh token");
  if (t.scope) parts.push(`scope: ${t.scope}`);
  return parts.join(" · ");
}

export function Field({ label, value, onChange, placeholder }: { label: string; value: string; onChange: (v: string) => void; placeholder?: string }) {
  return (
    <label>
      <span>{label}</span>
      <CodeEditor mode="line" className="field" value={value} placeholder={placeholder} onChange={onChange} />
    </label>
  );
}
