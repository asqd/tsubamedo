// {{Variables}} for the editors: what is defined in the open environment, where it comes from,
// and which names a request uses but nobody defined.
import { createContext, useContext } from "react";
import { Request } from "./api";

export interface VarInfo {
  name: string;
  value: string;
  /** «коллекция», «окружение stage», «stage, секрет», «из ответа «auth/login»»… */
  source: string;
  /** From a *.local.toml file: shown masked. */
  secret: boolean;
}

export const BUILTINS: VarInfo[] = [
  { name: "$uuid", value: "случайный UUID", source: "встроенная", secret: false },
  { name: "$timestamp", value: "секунды с 1970 года", source: "встроенная", secret: false },
  { name: "$timestampMs", value: "миллисекунды с 1970 года", source: "встроенная", secret: false },
  { name: "$randomInt", value: "случайное число 0–999", source: "встроенная", secret: false },
  { name: "$env.", value: "переменная окружения процесса: {{$env.TOKEN}}", source: "встроенная", secret: false },
];

export const VarsContext = createContext<VarInfo[]>([]);
export const useVars = () => useContext(VarsContext);

export const VAR_PATTERN = /\{\{\s*([^{}\s][^{}]*?)\s*\}\}/g;

/** The variable, a built-in, or null when it is not defined. */
export function lookup(name: string, vars: VarInfo[]): VarInfo | null {
  const found = vars.find((v) => v.name === name);
  if (found) return found;
  if (name.startsWith("$env.") && name.length > 5) {
    return { name, value: "берётся из окружения процесса при отправке", source: "встроенная", secret: false };
  }
  return BUILTINS.find((b) => b.name === name && !b.name.endsWith(".")) ?? null;
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
