// The request as it is stored in its file: method and address on top, the rest in tabs.
import { useState } from "react";
import { Auth, Body, Part, Request } from "../api";
import { key } from "../platform";
import { missingVars, useVars } from "../vars";
import { ChecksEditor } from "./ChecksEditor";
import { CodeEditor } from "./CodeEditor";
import { Field, freshOAuth, OAuthEditor } from "./OAuthEditor";
import { PairsTable } from "./PairsTable";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

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
    ["query", "Параметры", on(request.query) || null],
    ["headers", "Заголовки", on(request.headers) || null],
    ["auth", "Авторизация", request.auth.type === "none" ? null : "●"],
    ["body", "Тело", request.body.type === "none" ? null : "●"],
    ["checks", "Проверки", on(request.checks) + on(request.captures) || null],
    ["scripts", "Скрипты", request.scripts.before.trim() || request.scripts.after.trim() ? "●" : null],
    ["options", "Настройки", Object.values(request.options).some((v) => v != null) ? "●" : null],
  ];

  return (
    <div className="request">
      <div className="req-head">
        <input
          className="req-name"
          value={request.name ?? ""}
          placeholder={saved ?? "Черновик — не сохраняется"}
          spellCheck={false}
          disabled={!saved}
          onChange={(e) => set({ name: e.target.value || null })}
        />
        <button className="ghost small" onClick={onShowCode} title="Запрос в виде кода: curl, Python, JavaScript, Go и другие">
          Код…
        </button>
      </div>
      <div className="urlbar">
        <select className={`method m-${request.method.toLowerCase()}`} value={request.method} onChange={(e) => set({ method: e.target.value })}>
          {(METHODS.includes(request.method) ? METHODS : [request.method, ...METHODS]).map((m) => (
            <option key={m}>{m}</option>
          ))}
        </select>
        <CodeEditor
          mode="line"
          className="url"
          value={request.url}
          placeholder="Адрес, {{base_url}}/users или вставьте команду curl"
          autoFocus
          onChange={(url) => set({ url })}
          onSubmit={onSend}
          onPaste={onPasteCurl}
        />
        <button className="primary" disabled={sending || !request.url.trim()} onClick={onSend} title={`Отправить (${key("↵")})`}>
          Отправить
        </button>
      </div>
      {missing.length > 0 && (
        <div className="missing small">
          Не заданы{vars.length === 0 ? " (откройте коллекцию или выберите окружение)" : ""}:{" "}
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
          <PairsTable rows={request.headers} onChange={(headers) => set({ headers })} namePlaceholder="Заголовок" onSubmit={onSend} />
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
              <span>Перенаправления</span>
              <TriState value={request.options.followRedirects} yes="переходить" no="не переходить" onChange={(followRedirects) => set({ options: { ...request.options, followRedirects } })} />
            </label>
            <label>
              <span>Проверка сертификата</span>
              <TriState value={request.options.verifyTls} yes="проверять" no="не проверять" onChange={(verifyTls) => set({ options: { ...request.options, verifyTls } })} />
            </label>
            <label>
              <span>Cookie</span>
              <TriState
                value={request.options.cookies ?? null}
                yes="отправлять и запоминать"
                no="без cookie"
                onChange={(cookies) => set({ options: { ...request.options, cookies } })}
              />
            </label>
            <label>
              <span>Ждать ответа, мс</span>
              <input
                className="mono narrow"
                inputMode="numeric"
                value={request.options.timeoutMs ?? ""}
                placeholder="без ограничения"
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
      <div className="section-head">До запроса</div>
      <div className="hint">
        Видит запрос как в файле, с <code>{"{{переменными}}"}</code>, и может его менять. Переменные из <code>vars.set</code> подставятся в этот же
        запрос.
      </div>
      <CodeEditor
        mode="js"
        className="code script"
        value={scripts.before}
        placeholder={'vars.set("ts", Date.now());\nrequest.headers.set("X-Sign", crypto.hmac("sha256", vars.get("secret"), vars.render(request.body)));'}
        onChange={(before) => onChange({ ...scripts, before })}
        onSubmit={onSubmit}
      />
      <div className="section-head">После ответа</div>
      <div className="hint">
        <code>response.json()</code>, <code>response.status</code>, <code>response.headers.get(…)</code>. Тесты встают рядом с проверками, а{" "}
        <code>vars.set</code> отдаёт значение другим запросам — они отправят этот сами, если значения ещё нет.
      </div>
      <CodeEditor
        mode="js"
        className="code script"
        value={scripts.after}
        placeholder={'const order = response.json();\nvars.set("order_id", order.id);\ntest("заказ создан", () => expect(order.status).toBe("new"));'}
        onChange={(after) => onChange({ ...scripts, after })}
        onSubmit={onSubmit}
      />
      <div className="hint">
        JavaScript в QuickJS, без файлов и сети, до 5 секунд. Работает и в <code>tsubamedo run</code>. Ещё есть <code>crypto.sha256</code>,{" "}
        <code>crypto.hmac</code>, <code>btoa</code>/<code>atob</code>, <code>console.log</code> — в журнал под ответом.
      </div>
    </div>
  );
}

function TriState({ value, yes, no, onChange }: { value: boolean | null; yes: string; no: string; onChange: (v: boolean | null) => void }) {
  return (
    <select value={value == null ? "" : value ? "yes" : "no"} onChange={(e) => onChange(e.target.value === "" ? null : e.target.value === "yes")}>
      <option value="">как обычно</option>
      <option value="yes">{yes}</option>
      <option value="no">{no}</option>
    </select>
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
    };
    onChange(fresh[type]);
  };
  return (
    <div className="form">
      <label>
        <span>Тип</span>
        <select value={auth.type} onChange={(e) => switchTo(e.target.value as Auth["type"])}>
          <option value="none">Нет</option>
          <option value="bearer">Bearer-токен</option>
          <option value="basic">Basic: логин и пароль</option>
          <option value="digest">Digest: логин и пароль</option>
          <option value="apikey">API key</option>
          <option value="oauth2">OAuth 2.0</option>
        </select>
      </label>
      {auth.type === "oauth2" && <OAuthEditor auth={auth} request={request} env={env} sending={sending} onChange={onChange} />}
      {auth.type === "bearer" && <Field label="Токен" value={auth.token} onChange={(token) => onChange({ ...auth, token })} placeholder="{{token}}" />}
      {(auth.type === "basic" || auth.type === "digest") && (
        <>
          <Field label="Логин" value={auth.username} onChange={(username) => onChange({ ...auth, username })} />
          <Field label="Пароль" value={auth.password} onChange={(password) => onChange({ ...auth, password })} placeholder="{{password}}" />
        </>
      )}
      {auth.type === "apikey" && (
        <>
          <Field label="Имя" value={auth.key} onChange={(k) => onChange({ ...auth, key: k })} />
          <Field label="Значение" value={auth.value} onChange={(value) => onChange({ ...auth, value })} placeholder="{{api_key}}" />
          <label>
            <span>Куда</span>
            <select value={auth.location} onChange={(e) => onChange({ ...auth, location: e.target.value as "header" | "query" })}>
              <option value="header">в заголовок</option>
              <option value="query">в параметры адреса</option>
            </select>
          </label>
        </>
      )}
      {auth.type === "digest" && (
        <div className="hint">Запрос уходит без авторизации, сервер отвечает 401 с вызовом Digest, и запрос уходит ещё раз с ответом на него (MD5 или SHA-256).</div>
      )}
      {auth.type !== "none" && auth.type !== "oauth2" && <div className="hint">Секреты лучше держать в переменных из файла *.local.toml — он не попадает в git.</div>}
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
      setProblem(/\{\{/.test(body.content) ? "С {{переменными}} вне кавычек JSON не разобрать — форматирование недоступно" : `Не JSON: ${(e as Error).message}`);
    }
  };
  return (
    <div className="body-editor">
      <div className="row">
        <div className="seg">
          {(
            [
              ["none", "Нет"],
              ["json", "JSON"],
              ["text", "Текст"],
              ["form", "Форма"],
              ["multipart", "Multipart"],
              ["file", "Файл"],
            ] as [Body["type"], string][]
          ).map(([type, label]) => (
            <button key={type} className={body.type === type ? "on" : ""} onClick={() => switchTo(type)}>
              {label}
            </button>
          ))}
        </div>
        {body.type === "json" && (
          <button className="ghost" onClick={tidy}>
            Форматировать
          </button>
        )}
      </div>
      {problem && <div className="hint warn">{problem}</div>}
      {(body.type === "json" || body.type === "text") && (
        <CodeEditor
          mode={body.type}
          className="code"
          value={body.content}
          placeholder={body.type === "json" ? '{"name": "{{name}}"}' : "Текст запроса"}
          onChange={(content) => onChange({ ...body, content })}
          onSubmit={onSubmit}
        />
      )}
      {body.type === "text" && (
        <Field label="Content-Type" value={body.contentType ?? ""} placeholder="text/plain; charset=utf-8" onChange={(v) => onChange({ ...body, contentType: v || null })} />
      )}
      {body.type === "form" && <PairsTable rows={body.fields} onChange={(fields) => onChange({ ...body, fields })} namePlaceholder="Поле" onSubmit={onSubmit} />}
      {body.type === "multipart" && <PartsTable parts={body.parts} onChange={(parts) => onChange({ ...body, parts })} />}
      {body.type === "file" && (
        <div className="form">
          <Field label="Путь" value={body.path} placeholder="относительно папки запроса" onChange={(path) => onChange({ ...body, path })} />
          <Field label="Content-Type" value={body.contentType ?? ""} placeholder="по расширению" onChange={(v) => onChange({ ...body, contentType: v || null })} />
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
                <input className="mono" value={p.name} placeholder="Поле" spellCheck={false} onChange={(e) => set(i, { name: e.target.value })} />
              </td>
              <td className="kind">
                <select value={p.isFile ? "file" : "text"} onChange={(e) => set(i, { isFile: e.target.value === "file" })}>
                  <option value="text">текст</option>
                  <option value="file">файл</option>
                </select>
              </td>
              <td>
                <CodeEditor mode="line" className="field" value={p.value} placeholder={p.isFile ? "путь к файлу" : "Значение"} onChange={(value) => set(i, { value })} />
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
