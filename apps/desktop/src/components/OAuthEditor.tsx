// OAuth 2.0 settings of a request and its token: the core gets it before sending, keeps it in
// the app's data folder and renews it; here it can be got anew or forgotten.
import { useEffect, useRef, useState } from "react";
import { api, OAuth2, Request, TokenInfo } from "../api";
import { lang, t } from "../i18n";
import { CodeEditor } from "./CodeEditor";
import { Picker } from "./Picker";

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
      // The core says it in the window's language.
      if (message !== "Отменено" && message !== "Cancelled" && message !== "Canceled") setProblem(message);
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
        <span>{t("Как получить токен", "How to get the token")}</span>
        <Picker
          label={t("Как получить токен", "How to get the token")}
          options={[
            { value: "authorization_code", label: t("Вход в браузере (authorization code)", "Sign-in in the browser (authorization code)") },
            { value: "client_credentials", label: "Client credentials" },
            { value: "password", label: t("Логин и пароль (password)", "Username and password (password)") },
          ]}
          value={auth.grant}
          onChange={(grant) => set({ grant: grant as OAuth2["grant"] })}
        />
      </label>
      {code && <Field label={t("Адрес входа", "Authorization URL")} value={auth.authUrl} onChange={(authUrl) => set({ authUrl })} placeholder="https://id.example.com/authorize" />}
      <Field label={t("Адрес токена", "Token URL")} value={auth.tokenUrl} onChange={(tokenUrl) => set({ tokenUrl })} placeholder="https://id.example.com/oauth/token" />
      <Field label="Client ID" value={auth.clientId} onChange={(clientId) => set({ clientId })} placeholder="{{client_id}}" />
      <Field
        label="Client secret"
        value={auth.clientSecret}
        onChange={(clientSecret) => set({ clientSecret })}
        placeholder={code ? t("не нужен приложению с PKCE", "not needed by an app with PKCE") : "{{client_secret}}"}
      />
      {auth.grant === "password" && (
        <>
          <Field label={t("Логин", "Username")} value={auth.username} onChange={(username) => set({ username })} />
          <Field label={t("Пароль", "Password")} value={auth.password} onChange={(password) => set({ password })} placeholder="{{password}}" />
        </>
      )}
      <Field label="Scope" value={auth.scope} onChange={(scope) => set({ scope })} placeholder={t("через пробел: orders:read profile", "space-separated: orders:read profile")} />
      <details className="oauth-more">
        <summary className="faint small">{t("Ещё: audience, адрес возврата, PKCE, как передавать секрет", "More: audience, redirect URL, PKCE, how to send the secret")}</summary>
        <div className="form">
          <Field label="Audience" value={auth.audience} onChange={(audience) => set({ audience })} placeholder={t("нужен Auth0 и некоторым другим", "needed by Auth0 and a few others")} />
          {code && (
            <>
              <Field
                label={t("Адрес возврата", "Redirect URL")}
                value={auth.redirectUri}
                onChange={(redirectUri) => set({ redirectUri })}
                placeholder={t("http://127.0.0.1:<свободный порт>/callback", "http://127.0.0.1:<free port>/callback")}
              />
              <label>
                <span>PKCE</span>
                <Picker
                  label="PKCE"
                  options={[
                    { value: "on", label: t("да, S256", "yes, S256") },
                    { value: "off", label: t("нет", "no") },
                  ]}
                  value={auth.pkce ? "on" : "off"}
                  onChange={(v) => set({ pkce: v === "on" })}
                />
              </label>
            </>
          )}
          <label>
            <span>{t("Client ID и секрет", "Client ID and secret")}</span>
            <Picker
              label={t("Client ID и секрет", "Client ID and secret")}
              options={[
                { value: "basic", label: t("в заголовке Authorization: Basic", "in the Authorization: Basic header") },
                { value: "body", label: t("в теле запроса токена", "in the body of the token request") },
              ]}
              value={auth.clientAuth}
              onChange={(clientAuth) => set({ clientAuth: clientAuth as OAuth2["clientAuth"] })}
            />
          </label>
        </div>
      </details>
      <div className="oauth-token">
        <div className="small">
          {token ? (
            <>
              <span className="good-text">✓</span> {t("Токен", "Token")} <code>{token.preview}</code>
              <span className="muted"> · {describe(token)}</span>
            </>
          ) : (
            <span className="muted">{t("Токена ещё нет — получу при отправке.", "No token yet — will get one when sending.")}</span>
          )}
        </div>
        <div className="oauth-actions">
          {busy ? (
            <>
              <span className="faint small">{code ? t("Жду вход в браузере…", "Waiting for sign-in in the browser…") : t("Получаю…", "Getting…")}</span>
              <button className="small" onClick={() => api.cancel(busy)}>
                {t("Отмена", "Cancel")}
              </button>
            </>
          ) : (
            <>
              <button className="small" onClick={getNew} disabled={sending} title={t("Получить токен заново, не дожидаясь, пока этот истечёт", "Get a new token now, without waiting for this one to expire")}>
                {token ? t("Получить новый", "Get new") : t("Получить сейчас", "Get now")}
              </button>
              {token && (
                <button className="small ghost" onClick={forget} title={t("Удалить сохранённый токен", "Delete the saved token")}>
                  {t("Забыть", "Forget")}
                </button>
              )}
            </>
          )}
        </div>
        {problem && <div className="error-box small">{problem}</div>}
      </div>
      <div className="hint">
        {code
          ? t(
              "Браузер откроет страницу входа, а код вернётся на этот компьютер. По RFC 8252 провайдер принимает любой порт на 127.0.0.1; если ему нужен точный адрес — впишите его в «Адрес возврата» и добавьте у провайдера.",
              'The browser opens the sign-in page, and the code comes back to this computer. Under RFC 8252 the provider accepts any port on 127.0.0.1; if it needs an exact address, enter it in "Redirect URL" and add it at the provider.',
            )
          : t("Токен запрашивается перед первой отправкой и живёт, пока не истечёт.", "The token is requested before the first send and lives until it expires.")}{" "}
        {t(
          "Истёкший обновляется по refresh token; если сервер ответил 401 на сохранённый, запрос уйдёт ещё раз с новым. Токены хранятся в папке данных приложения, не в коллекции.",
          "An expired one is renewed with the refresh token; if the server answers 401 to a saved one, the request goes again with a new one. Tokens are kept in the app's data folder, not in the collection.",
        )}
      </div>
    </>
  );
}

function describe(token: TokenInfo): string {
  const at = new Date(token.obtainedAt * 1000).toLocaleTimeString(lang() === "en" ? "en-US" : "ru", { hour: "2-digit", minute: "2-digit", hour12: false });
  const parts = [t(`получен ${at}`, `obtained ${at}`)];
  if (token.expiresAt != null) {
    const left = token.expiresAt - Date.now() / 1000;
    parts.push(
      left <= 0
        ? t("истёк", "expired")
        : left < 120
          ? t(`ещё ${Math.round(left)} с`, `${Math.round(left)} s left`)
          : left < 7200
            ? t(`ещё ${Math.round(left / 60)} мин`, `${Math.round(left / 60)} min left`)
            : t(`ещё ${Math.round(left / 3600)} ч`, `${Math.round(left / 3600)} h left`),
    );
  }
  if (token.hasRefresh) parts.push(t("есть refresh token", "has a refresh token"));
  if (token.scope) parts.push(`scope: ${token.scope}`);
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
