// The request as it is stored in its file: method and address on top, the rest in tabs.
import { useState } from "react";
import { Auth, Body, Part, Request } from "../api";
import { t } from "../i18n";
import { key } from "../platform";
import { missingVars, useVars } from "../vars";
import { ChecksEditor } from "./ChecksEditor";
import { CodeEditor } from "./CodeEditor";
import { MethodPicker } from "./MethodPicker";
import { Field, freshOAuth, OAuthEditor } from "./OAuthEditor";
import { PairsTable } from "./PairsTable";
import { Picker } from "./Picker";


type Tab = "query" | "headers" | "auth" | "body" | "checks" | "scripts" | "options";

export function RequestEditor({
  request,
  onChange,
  onSend,
  sending,
  saved,
  onPasteCurl,
  onShowCode,
  env,
}: {
  request: Request;
  onChange: (r: Request) => void;
  onSend: () => void;
  sending: boolean;
  /** Where the request lives, or null for a scratch request that is not saved. */
  saved: string | null;
  /** A curl command pasted into the address; true when it was imported. */
  onPasteCurl: (text: string) => boolean;
  onShowCode: () => void;
  /** The open environment: OAuth 2.0 tokens and cookies are kept per settings and per it. */
  env: string | null;
}) {
  const [tab, setTab] = useState<Tab>("query");
  const vars = useVars();
  const missing = missingVars(request, vars);
  const set = (patch: Partial<Request>) => onChange({ ...request, ...patch });
  const on = (rows: { enabled: boolean }[]) => rows.filter((r) => r.enabled).length;
  const tabs: [Tab, string, string | number | null][] = [
    ["query", t("Параметры", "Params"), on(request.query) || null],
    ["headers", t("Заголовки", "Headers"), on(request.headers) || null],
    ["auth", t("Авторизация", "Auth"), request.auth.type === "none" ? null : "●"],
    ["body", t("Тело", "Body"), request.body.type === "none" ? null : "●"],
    ["checks", t("Проверки", "Checks"), on(request.checks) + on(request.captures) || null],
    ["scripts", t("Скрипты", "Scripts"), request.scripts.before.trim() || request.scripts.after.trim() ? "●" : null],
    ["options", t("Настройки", "Settings"), Object.values(request.options).some((v) => v != null) ? "●" : null],
  ];

  return (
    <div className="request">
      <div className="req-head">
        <input
          className="req-name"
          value={request.name ?? ""}
          placeholder={saved ?? t("Черновик — не сохраняется", "Draft — not saved")}
          spellCheck={false}
          disabled={!saved}
          onChange={(e) => set({ name: e.target.value || null })}
        />
        <button className="ghost small" onClick={onShowCode} title={t("Запрос в виде кода: curl, Python, JavaScript, Go и другие", "The request as code: curl, Python, JavaScript, Go and more")}>
          {t("Код…", "Code…")}
        </button>
      </div>
      <div className="urlbar">
        <MethodPicker value={request.method} onChange={(method) => set({ method })} />
        <CodeEditor
          mode="line"
          className="url"
          value={request.url}
          placeholder={t("Адрес, {{base_url}}/users или вставьте команду curl", "URL, {{base_url}}/users, or paste a curl command")}
          // Only an empty address waits for typing. A blinking caret in WebKit on macOS costs
          // its GPU process ~120 MB while the window is in front, so it is not lit for nothing.
          autoFocus={!request.url}
          onChange={(url) => set({ url })}
          onSubmit={onSend}
          onPaste={onPasteCurl}
        />
        <button className="primary" disabled={sending || !request.url.trim()} onClick={onSend} title={t(`Отправить (${key("↵")})`, `Send (${key("↵")})`)}>
          {t("Отправить", "Send")}
        </button>
      </div>
      {missing.length > 0 && (
        <div className="missing small">
          {vars.length === 0
            ? t("Не заданы (откройте коллекцию или выберите окружение):", "Not set (open a collection or pick an environment):")
            : t("Не заданы:", "Not set:")}{" "}
          {missing.map((name) => (
            <code key={name}>{`{{${name}}}`}</code>
          ))}
        </div>
      )}
      <div className="tabs">
        {tabs.map(([id, label, badge]) => (
          <button key={id} className={tab === id ? "on" : ""} onClick={() => setTab(id)}>
            {label}
            {badge != null && <span className="badge">{badge}</span>}
          </button>
        ))}
      </div>
      <div className="tab-body scroll pad">
        {tab === "query" && <PairsTable rows={request.query} onChange={(query) => set({ query })} onSubmit={onSend} />}
        {tab === "headers" && (
          <PairsTable rows={request.headers} onChange={(headers) => set({ headers })} namePlaceholder={t("Заголовок", "Header")} onSubmit={onSend} />
        )}
        {tab === "auth" && <AuthEditor request={request} env={env} sending={sending} onChange={(auth) => set({ auth })} />}
        {tab === "body" && <BodyEditor body={request.body} onChange={(body) => set({ body })} onSubmit={onSend} />}
        {tab === "checks" && (
          <ChecksEditor
            checks={request.checks}
            captures={request.captures}
            onChecks={(checks) => set({ checks })}
            onCaptures={(captures) => set({ captures })}
            onSubmit={onSend}
            saved={Boolean(saved)}
          />
        )}
        {tab === "scripts" && <ScriptsEditor scripts={request.scripts} onChange={(scripts) => set({ scripts })} onSubmit={onSend} />}
        {tab === "options" && (
          <div className="form">
            <label>
              <span>{t("Перенаправления", "Redirects")}</span>
              <TriState label={t("Перенаправления", "Redirects")} value={request.options.followRedirects} yes={t("переходить", "follow")} no={t("не переходить", "don't follow")} onChange={(followRedirects) => set({ options: { ...request.options, followRedirects } })} />
            </label>
            <label>
              <span>{t("Проверка сертификата", "Certificate verification")}</span>
              <TriState label={t("Проверка сертификата", "Certificate verification")} value={request.options.verifyTls} yes={t("проверять", "verify")} no={t("не проверять", "don't verify")} onChange={(verifyTls) => set({ options: { ...request.options, verifyTls } })} />
            </label>
            <label>
              <span>{t("Cookie", "Cookies")}</span>
              <TriState
                label={t("Cookie", "Cookies")}
                value={request.options.cookies ?? null}
                yes={t("отправлять и запоминать", "send and remember")}
                no={t("без cookie", "no cookies")}
                onChange={(cookies) => set({ options: { ...request.options, cookies } })}
              />
            </label>
            <label>
              <span>{t("Ждать ответа, мс", "Response timeout, ms")}</span>
              <input
                className="mono narrow"
                inputMode="numeric"
                value={request.options.timeoutMs ?? ""}
                placeholder={t("без ограничения", "no limit")}
                onChange={(e) => {
                  const n = parseInt(e.target.value.replace(/\D/g, ""), 10);
                  set({ options: { ...request.options, timeoutMs: Number.isFinite(n) ? n : null } });
                }}
              />
            </label>
          </div>
        )}
      </div>
    </div>
  );
}

function ScriptsEditor({ scripts, onChange, onSubmit }: { scripts: Request["scripts"]; onChange: (s: Request["scripts"]) => void; onSubmit: () => void }) {
  return (
    <div className="scripts-editor">
      <div className="section-head">{t("До запроса", "Before the request")}</div>
      <div className="hint">
        {t("Видит запрос как в файле, с", "Sees the request as in the file, with")} <code>{t("{{переменными}}", "{{variables}}")}</code>
        {t(", и может его менять. Переменные из", ", and can change it. Variables from")} <code>vars.set</code>{" "}
        {t("подставятся в этот же запрос.", "are filled into this same request.")}
      </div>
      <CodeEditor
        mode="js"
        className="code script"
        value={scripts.before}
        placeholder={'vars.set("ts", Date.now());\nrequest.headers.set("X-Sign", crypto.hmac("sha256", vars.get("secret"), vars.render(request.body)));'}
        onChange={(before) => onChange({ ...scripts, before })}
        onSubmit={onSubmit}
      />
      <div className="section-head">{t("После ответа", "After the response")}</div>
      <div className="hint">
        <code>response.json()</code>, <code>response.status</code>, <code>response.headers.get(…)</code>.{" "}
        {t("Тесты встают рядом с проверками, а", "Tests show up next to the checks, and")} <code>vars.set</code>{" "}
        {t(
          "отдаёт значение другим запросам — они отправят этот сами, если значения ещё нет.",
          "hands the value to other requests — they send this one themselves if the value is not there yet.",
        )}
      </div>
      <CodeEditor
        mode="js"
        className="code script"
        value={scripts.after}
        placeholder={t(
          'const order = response.json();\nvars.set("order_id", order.id);\ntest("заказ создан", () => expect(order.status).toBe("new"));',
          'const order = response.json();\nvars.set("order_id", order.id);\ntest("order created", () => expect(order.status).toBe("new"));',
        )}
        onChange={(after) => onChange({ ...scripts, after })}
        onSubmit={onSubmit}
      />
      <div className="hint">
        {t("JavaScript в QuickJS, без файлов и сети, до 5 секунд. Работает и в", "JavaScript in QuickJS, with no files or network, up to 5 seconds. Works in")}{" "}
        <code>tsubamedo run</code>
        {t(". Ещё есть", " too. There are also")} <code>crypto.sha256</code>, <code>crypto.hmac</code>, <code>btoa</code>/<code>atob</code>,{" "}
        <code>console.log</code> {t("— в журнал под ответом.", "— to the log under the response.")}
      </div>
    </div>
  );
}

function TriState({ label, value, yes, no, onChange }: { label: string; value: boolean | null; yes: string; no: string; onChange: (v: boolean | null) => void }) {
  return (
    <Picker
      label={label}
      options={[
        { value: "", label: t("как обычно", "default"), faint: true },
        { value: "yes", label: yes },
        { value: "no", label: no },
      ]}
      value={value == null ? "" : value ? "yes" : "no"}
      onChange={(v) => onChange(v === "" ? null : v === "yes")}
    />
  );
}

function AuthEditor({ request, env, sending, onChange }: { request: Request; env: string | null; sending: boolean; onChange: (a: Auth) => void }) {
  const auth = request.auth;
  const switchTo = (type: Auth["type"]) => {
    const fresh: Record<Auth["type"], Auth> = {
      none: { type: "none" },
      basic: { type: "basic", username: "", password: "" },
      bearer: { type: "bearer", token: "" },
      digest: { type: "digest", username: "", password: "" },
      apikey: { type: "apikey", key: "X-Api-Key", value: "", location: "header" },
      oauth2: freshOAuth(),
      awsv4: { type: "awsv4", accessKey: "{{aws_access_key}}", secretKey: "{{aws_secret_key}}", sessionToken: "", region: "", service: "" },
    };
    onChange(fresh[type]);
  };
  return (
    <div className="form">
      <label>
        <span>{t("Тип", "Type")}</span>
        <Picker
          label={t("Тип авторизации", "Auth type")}
          options={[
            { value: "none", label: t("Нет", "None"), faint: true },
            { value: "bearer", label: t("Bearer-токен", "Bearer token") },
            { value: "basic", label: t("Basic: логин и пароль", "Basic: username and password") },
            { value: "digest", label: t("Digest: логин и пароль", "Digest: username and password") },
            { value: "apikey", label: "API key" },
            { value: "oauth2", label: "OAuth 2.0" },
            { value: "awsv4", label: "AWS Signature v4" },
          ]}
          value={auth.type}
          onChange={(type) => switchTo(type as Auth["type"])}
        />
      </label>
      {auth.type === "oauth2" && <OAuthEditor auth={auth} request={request} env={env} sending={sending} onChange={onChange} />}
      {auth.type === "bearer" && <Field label={t("Токен", "Token")} value={auth.token} onChange={(token) => onChange({ ...auth, token })} placeholder="{{token}}" />}
      {(auth.type === "basic" || auth.type === "digest") && (
        <>
          <Field label={t("Логин", "Username")} value={auth.username} onChange={(username) => onChange({ ...auth, username })} />
          <Field label={t("Пароль", "Password")} value={auth.password} onChange={(password) => onChange({ ...auth, password })} placeholder="{{password}}" />
        </>
      )}
      {auth.type === "apikey" && (
        <>
          <Field label={t("Имя", "Name")} value={auth.key} onChange={(k) => onChange({ ...auth, key: k })} />
          <Field label={t("Значение", "Value")} value={auth.value} onChange={(value) => onChange({ ...auth, value })} placeholder="{{api_key}}" />
          <label>
            <span>{t("Куда", "Add to")}</span>
            <Picker
              label={t("Куда", "Add to")}
              options={[
                { value: "header", label: t("в заголовок", "header") },
                { value: "query", label: t("в параметры адреса", "query params") },
              ]}
              value={auth.location}
              onChange={(location) => onChange({ ...auth, location: location as "header" | "query" })}
            />
          </label>
        </>
      )}
      {auth.type === "awsv4" && (
        <>
          <Field label={t("Ключ доступа", "Access key")} value={auth.accessKey} onChange={(accessKey) => onChange({ ...auth, accessKey })} placeholder="{{aws_access_key}}" />
          <Field label={t("Секретный ключ", "Secret key")} value={auth.secretKey} onChange={(secretKey) => onChange({ ...auth, secretKey })} placeholder="{{aws_secret_key}}" />
          <Field label={t("Токен сессии", "Session token")} value={auth.sessionToken} onChange={(sessionToken) => onChange({ ...auth, sessionToken })} placeholder={t("для временных ключей", "for temporary keys")} />
          <Field label={t("Регион", "Region")} value={auth.region} onChange={(region) => onChange({ ...auth, region })} placeholder="eu-west-1" />
          <Field label={t("Сервис", "Service")} value={auth.service} onChange={(service) => onChange({ ...auth, service })} placeholder="s3, execute-api, lambda…" />
          <div className="hint">
            {t(
              "Запрос подписывается прямо перед отправкой: метод, путь, параметры, заголовки и тело. Для S3 добавляется хеш тела.",
              "The request is signed right before sending: method, path, params, headers and body. For S3 the body hash is added.",
            )}
          </div>
        </>
      )}
      {auth.type === "digest" && (
        <div className="hint">
          {t(
            "Запрос уходит без авторизации, сервер отвечает 401 с вызовом Digest, и запрос уходит ещё раз с ответом на него (MD5 или SHA-256).",
            "The request goes without authorization, the server answers 401 with a Digest challenge, and the request goes again with the answer to it (MD5 or SHA-256).",
          )}
        </div>
      )}
      {auth.type !== "none" && auth.type !== "oauth2" && <div className="hint">{t("Секреты лучше держать в переменных из файла *.local.toml — он не попадает в git.", "Secrets are best kept in variables from a *.local.toml file — it stays out of git.")}</div>}
    </div>
  );
}

function BodyEditor({ body, onChange, onSubmit }: { body: Body; onChange: (b: Body) => void; onSubmit: () => void }) {
  const [problem, setProblem] = useState<string | null>(null);
  const text = body.type === "json" || body.type === "text" ? body.content : "";
  const switchTo = (type: Body["type"]) => {
    const fresh: Record<Body["type"], Body> = {
      none: { type: "none" },
      json: { type: "json", content: text || "{\n  \n}\n" },
      text: { type: "text", content: text, contentType: null },
      form: { type: "form", fields: [] },
      multipart: { type: "multipart", parts: [] },
      file: { type: "file", path: "", contentType: null },
    };
    setProblem(null);
    onChange(fresh[type]);
  };
  const tidy = () => {
    if (body.type !== "json") return;
    try {
      onChange({ ...body, content: JSON.stringify(JSON.parse(body.content), null, 2) + "\n" });
      setProblem(null);
    } catch (e) {
      setProblem(/\{\{/.test(body.content) ? t("С {{переменными}} вне кавычек JSON не разобрать — форматирование недоступно", "JSON with {{variables}} outside quotes can't be parsed — formatting is unavailable")
          : t(`Не JSON: ${(e as Error).message}`, `Not JSON: ${(e as Error).message}`));
    }
  };
  return (
    <div className="body-editor">
      <div className="row">
        <div className="seg">
          {(
            [
              ["none", t("Нет", "None")],
              ["json", "JSON"],
              ["text", t("Текст", "Text")],
              ["form", t("Форма", "Form")],
              ["multipart", "Multipart"],
              ["file", t("Файл", "File")],
            ] as [Body["type"], string][]
          ).map(([type, label]) => (
            <button key={type} className={body.type === type ? "on" : ""} onClick={() => switchTo(type)}>
              {label}
            </button>
          ))}
        </div>
        {body.type === "json" && (
          <button className="ghost" onClick={tidy}>
            {t("Форматировать", "Format")}
          </button>
        )}
      </div>
      {problem && <div className="hint warn">{problem}</div>}
      {(body.type === "json" || body.type === "text") && (
        <CodeEditor
          mode={body.type}
          className="code"
          value={body.content}
          placeholder={body.type === "json" ? '{"name": "{{name}}"}' : t("Текст запроса", "Request text")}
          onChange={(content) => onChange({ ...body, content })}
          onSubmit={onSubmit}
        />
      )}
      {body.type === "text" && (
        <Field label="Content-Type" value={body.contentType ?? ""} placeholder="text/plain; charset=utf-8" onChange={(v) => onChange({ ...body, contentType: v || null })} />
      )}
      {body.type === "form" && <PairsTable rows={body.fields} onChange={(fields) => onChange({ ...body, fields })} namePlaceholder={t("Поле", "Field")} onSubmit={onSubmit} />}
      {body.type === "multipart" && <PartsTable parts={body.parts} onChange={(parts) => onChange({ ...body, parts })} />}
      {body.type === "file" && (
        <div className="form">
          <Field label={t("Путь", "Path")} value={body.path} placeholder={t("относительно папки запроса", "relative to the request's folder")} onChange={(path) => onChange({ ...body, path })} />
          <Field label="Content-Type" value={body.contentType ?? ""} placeholder={t("по расширению", "by extension")} onChange={(v) => onChange({ ...body, contentType: v || null })} />
        </div>
      )}
    </div>
  );
}

function PartsTable({ parts, onChange }: { parts: Part[]; onChange: (p: Part[]) => void }) {
  const all = [...parts, { name: "", value: "", isFile: false, enabled: true }];
  const set = (i: number, patch: Partial<Part>) => {
    const next = all.map((p, j) => (j === i ? { ...p, ...patch } : p));
    onChange(next.filter((p, j) => j < parts.length || p.name || p.value));
  };
  return (
    <table className="pairs">
      <tbody>
        {all.map((p, i) => {
          const fresh = i === parts.length;
          return (
            <tr key={i} className={p.enabled ? "" : "off"}>
              <td className="chk">{!fresh && <input type="checkbox" checked={p.enabled} onChange={(e) => set(i, { enabled: e.target.checked })} />}</td>
              <td>
                <input className="mono" value={p.name} placeholder={t("Поле", "Field")} spellCheck={false} onChange={(e) => set(i, { name: e.target.value })} />
              </td>
              <td className="kind">
                <Picker
                  label={t("Тип поля", "Field type")}
                  options={[
                    { value: "text", label: t("текст", "text") },
                    { value: "file", label: t("файл", "file") },
                  ]}
                  value={p.isFile ? "file" : "text"}
                  onChange={(kind) => set(i, { isFile: kind === "file" })}
                />
              </td>
              <td>
                <CodeEditor mode="line" className="field" value={p.value} placeholder={p.isFile ? t("путь к файлу", "file path") : t("Значение", "Value")} onChange={(value) => set(i, { value })} />
              </td>
              <td className="del">
                {!fresh && (
                  <button className="icon" onClick={() => onChange(parts.filter((_, j) => j !== i))}>
                    ×
                  </button>
                )}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
