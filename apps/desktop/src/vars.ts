// {{Variables}} for the editors: what is defined in the open environment, where it comes from,
// and which names a request uses but nobody defined.
import { createContext, useContext } from "react";
import { api, Request } from "./api";
import { t } from "./i18n";

export interface VarInfo {
  name: string;
  value: string;
  /** «коллекция», «окружение stage», «stage, секрет», «из ответа «auth/login»»… */
  source: string;
  /** From a *.local.toml file: shown masked. */
  secret: boolean;
}

/** A built-in variable, described in the window's language. */
const builtin = (name: string, value: string): VarInfo => ({ name, value, source: t("встроенная", "built-in"), secret: false });

/** Built when asked, not at import, so the descriptions follow the language. */
const fixed = (): VarInfo[] => [
  builtin("$uuid", t("случайный UUID", "random UUID")),
  builtin("$timestamp", t("секунды с 1970 года", "seconds since 1970")),
  builtin("$timestampMs", t("миллисекунды с 1970 года", "milliseconds since 1970")),
  builtin("$isoTimestamp", t("время в ISO 8601, UTC", "time in ISO 8601, UTC")),
  builtin("$randomInt", t("случайное число 0–999", "random number 0–999")),
  builtin("$env.", t("переменная окружения процесса: {{$env.TOKEN}}", "process environment variable: {{$env.TOKEN}}")),
];

/** Postman's made-up values (`$randomEmail`…), named by the core once the window starts. */
let fakes: string[] = [];
export async function loadFakes() {
  try {
    fakes = await api.fakeNames();
  } catch {
    // Only suggestions.
  }
}

/** Every built-in, for suggestions. */
export const builtins = (): VarInfo[] => [
  ...fixed(),
  ...fakes.map((n) => builtin(`$${n}`, t("случайное значение, как в Postman", "random value, as in Postman"))),
];

export const VarsContext = createContext<VarInfo[]>([]);
export const useVars = () => useContext(VarsContext);

export const VAR_PATTERN = /\{\{\s*([^{}\s][^{}]*?)\s*\}\}/g;

/** The variable, a built-in, or null when it is not defined. */
export function lookup(name: string, vars: VarInfo[]): VarInfo | null {
  const found = vars.find((v) => v.name === name);
  if (found) return found;
  if (name.startsWith("$env.") && name.length > 5) {
    return builtin(name, t("берётся из окружения процесса при отправке", "taken from the process environment when sent"));
  }
  return builtins().find((b) => b.name === name && !b.name.endsWith(".")) ?? null;
}

export function shown(v: VarInfo): string {
  if (!v.secret) return v.value;
  return v.value.length <= 4 ? "••••" : `${v.value.slice(0, 2)}••••${v.value.slice(-2)}`;
}

/** Undefined variables in everything the request would send. Values of variables count too:
 *  `{{base}}` defined as `https://{{host}}` needs `host`. */
export function missingVars(request: Request, vars: VarInfo[]): string[] {
  const texts: string[] = [request.url, request.method];
  for (const p of [...request.query, ...request.headers]) if (p.enabled) texts.push(p.name, p.value);
  const a = request.auth;
  if (a.type === "basic" || a.type === "digest") texts.push(a.username, a.password);
  if (a.type === "bearer") texts.push(a.token);
  if (a.type === "apikey") texts.push(a.key, a.value);
  if (a.type === "oauth2") texts.push(a.authUrl, a.tokenUrl, a.clientId, a.clientSecret, a.scope, a.audience, a.redirectUri, a.username, a.password);
  if (a.type === "awsv4") texts.push(a.accessKey, a.secretKey, a.sessionToken, a.region, a.service);
  const b = request.body;
  if (b.type === "json" || b.type === "text") texts.push(b.content);
  if (b.type === "form") for (const f of b.fields) if (f.enabled) texts.push(f.name, f.value);
  if (b.type === "multipart") for (const p of b.parts) if (p.enabled) texts.push(p.name, p.value);
  if (b.type === "file") texts.push(b.path);
  for (const c of request.checks ?? []) if (c.enabled) texts.push(c.text);

  const missing = new Set<string>();
  const seen = new Set<string>();
  const scan = (text: string) => {
    for (const m of text.matchAll(VAR_PATTERN)) {
      const name = m[1];
      if (seen.has(name)) continue;
      seen.add(name);
      const v = lookup(name, vars);
      if (!v) missing.add(name);
      else if (!v.name.startsWith("$")) scan(v.value);
    }
  };
  texts.forEach(scan);
  return [...missing];
}
