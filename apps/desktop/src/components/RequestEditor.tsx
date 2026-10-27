// The request as it is stored in its file: method and address on top, the rest in tabs.
import { useState } from "react";
import { Auth, Body, Part, Request } from "../api";
import { key } from "../platform";
import { PairsTable } from "./PairsTable";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

type Tab = "query" | "headers" | "auth" | "body" | "options";

export function RequestEditor({
  request,
  onChange,
  onSend,
  sending,
  saved,
}: {
  request: Request;
  onChange: (r: Request) => void;
  onSend: () => void;
  sending: boolean;
  /** Where the request lives, or null for a scratch request that is not saved. */
  saved: string | null;
}) {
  const [tab, setTab] = useState<Tab>("query");
  const set = (patch: Partial<Request>) => onChange({ ...request, ...patch });
  const on = (rows: { enabled: boolean }[]) => rows.filter((r) => r.enabled).length;
  const tabs: [Tab, string, string | number | null][] = [
    ["query", "Параметры", on(request.query) || null],
    ["headers", "Заголовки", on(request.headers) || null],
    ["auth", "Авторизация", request.auth.type === "none" ? null : "●"],
    ["body", "Тело", request.body.type === "none" ? null : "●"],
    ["options", "Настройки", Object.values(request.options).some((v) => v != null) ? "●" : null],
  ];

  return (
    <div className="request">
      <input
        className="req-name"
        value={request.name ?? ""}
        placeholder={saved ?? "Черновик — не сохраняется"}
        spellCheck={false}
        disabled={!saved}
        onChange={(e) => set({ name: e.target.value || null })}
      />
      <form
        className="urlbar"
        onSubmit={(e) => {
          e.preventDefault();
          onSend();
        }}
      >
        <select className={`method m-${request.method.toLowerCase()}`} value={request.method} onChange={(e) => set({ method: e.target.value })}>
          {(METHODS.includes(request.method) ? METHODS : [request.method, ...METHODS]).map((m) => (
            <option key={m}>{m}</option>
          ))}
        </select>
        <input
          className="url mono"
          value={request.url}
          placeholder="https://api.example.com/users или {{base_url}}/users"
          spellCheck={false}
          autoFocus
          onChange={(e) => set({ url: e.target.value })}
        />
        <button className="primary" type="submit" disabled={sending || !request.url.trim()} title={`Отправить (${key("↵")})`}>
          Отправить
        </button>
      </form>
      <div className="tabs">
        {tabs.map(([id, label, badge]) => (
          <button key={id} className={tab === id ? "on" : ""} onClick={() => setTab(id)}>
            {label}
            {badge != null && <span className="badge">{badge}</span>}
          </button>
        ))}
      </div>
      <div className="tab-body scroll pad">
        {tab === "query" && <PairsTable rows={request.query} onChange={(query) => set({ query })} />}
        {tab === "headers" && (
          <PairsTable rows={request.headers} onChange={(headers) => set({ headers })} namePlaceholder="Заголовок" />
        )}
        {tab === "auth" && <AuthEditor auth={request.auth} onChange={(auth) => set({ auth })} />}
        {tab === "body" && <BodyEditor body={request.body} onChange={(body) => set({ body })} />}
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

function TriState({ value, yes, no, onChange }: { value: boolean | null; yes: string; no: string; onChange: (v: boolean | null) => void }) {
  return (
    <select value={value == null ? "" : value ? "yes" : "no"} onChange={(e) => onChange(e.target.value === "" ? null : e.target.value === "yes")}>
      <option value="">как обычно</option>
      <option value="yes">{yes}</option>
      <option value="no">{no}</option>
    </select>
  );
}

function AuthEditor({ auth, onChange }: { auth: Auth; onChange: (a: Auth) => void }) {
  const switchTo = (type: Auth["type"]) => {
    const fresh: Record<Auth["type"], Auth> = {
      none: { type: "none" },
      basic: { type: "basic", username: "", password: "" },
      bearer: { type: "bearer", token: "" },
      apikey: { type: "apikey", key: "X-Api-Key", value: "", location: "header" },
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
          <option value="apikey">API key</option>
        </select>
      </label>
      {auth.type === "bearer" && <Field label="Токен" value={auth.token} onChange={(token) => onChange({ ...auth, token })} placeholder="{{token}}" />}
      {auth.type === "basic" && (
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
      {auth.type !== "none" && <div className="hint">Секреты лучше держать в переменных из файла *.local.toml — он не попадает в git.</div>}
    </div>
  );
}

function Field({ label, value, onChange, placeholder }: { label: string; value: string; onChange: (v: string) => void; placeholder?: string }) {
  return (
    <label>
      <span>{label}</span>
      <input className="mono" value={value} placeholder={placeholder} spellCheck={false} onChange={(e) => onChange(e.target.value)} />
    </label>
  );
}

function BodyEditor({ body, onChange }: { body: Body; onChange: (b: Body) => void }) {
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
        <textarea
          className="code mono"
          value={body.content}
          spellCheck={false}
          placeholder={body.type === "json" ? '{"name": "{{name}}"}' : "Текст запроса"}
          onChange={(e) => onChange({ ...body, content: e.target.value })}
          onKeyDown={(e) => {
            // Tab indents instead of leaving the editor.
            if (e.key !== "Tab") return;
            e.preventDefault();
            const el = e.currentTarget;
            const { selectionStart: s, selectionEnd: end } = el;
            const next = el.value.slice(0, s) + "  " + el.value.slice(end);
            onChange({ ...body, content: next });
            requestAnimationFrame(() => el.setSelectionRange(s + 2, s + 2));
          }}
        />
      )}
      {body.type === "text" && (
        <Field label="Content-Type" value={body.contentType ?? ""} placeholder="text/plain; charset=utf-8" onChange={(v) => onChange({ ...body, contentType: v || null })} />
      )}
      {body.type === "form" && <PairsTable rows={body.fields} onChange={(fields) => onChange({ ...body, fields })} namePlaceholder="Поле" />}
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
                <input className="mono" value={p.value} placeholder={p.isFile ? "путь к файлу" : "Значение"} spellCheck={false} onChange={(e) => set(i, { value: e.target.value })} />
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
