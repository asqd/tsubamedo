// Types mirror the serde output of tsubamedo-core and tsubamedo-session (camelCase); every
// command goes through one `call`, served by the Tauri shell or, in bridge.html, by the bridge.
import { invoke } from "@tauri-apps/api/core";
import type { VarInfo } from "./vars";

export interface Pair {
  name: string;
  value: string;
  enabled: boolean;
}

export interface Part {
  name: string;
  value: string;
  isFile: boolean;
  enabled: boolean;
}

export type Auth =
  | { type: "none" }
  | { type: "basic"; username: string; password: string }
  | { type: "bearer"; token: string }
  | { type: "apikey"; key: string; value: string; location: "header" | "query" }
  | ({ type: "oauth2" } & OAuth2);

/** The token is got, kept and renewed by the core. */
export interface OAuth2 {
  grant: "authorization_code" | "client_credentials" | "password";
  authUrl: string;
  tokenUrl: string;
  clientId: string;
  clientSecret: string;
  scope: string;
  audience: string;
  /** Empty: a free port on this machine. */
  redirectUri: string;
  pkce: boolean;
  clientAuth: "basic" | "body";
  username: string;
  password: string;
}

/** A kept token, never whole. Times are Unix seconds. */
export interface TokenInfo {
  preview: string;
  obtainedAt: number;
  expiresAt: number | null;
  hasRefresh: boolean;
  scope: string | null;
}

export interface Cookie {
  name: string;
  value: string;
  domain: string;
  hostOnly: boolean;
  path: string;
  /** Unix seconds; null — until the jar is cleared. */
  expires: number | null;
  secure: boolean;
  httpOnly: boolean;
  sameSite?: string;
}

export type Body =
  | { type: "none" }
  | { type: "json"; content: string }
  | { type: "text"; content: string; contentType: string | null }
  | { type: "form"; fields: Pair[] }
  | { type: "multipart"; parts: Part[] }
  | { type: "file"; path: string; contentType: string | null };

export interface RequestOptions {
  followRedirects: boolean | null;
  timeoutMs: number | null;
  verifyTls: boolean | null;
  /** Send the environment's cookies and keep new ones; on unless false. */
  cookies?: boolean | null;
}

/** A response check: `status == 200`, `$.items count > 0`. */
export interface Check {
  text: string;
  enabled: boolean;
}

export interface Request {
  name: string | null;
  method: string;
  url: string;
  query: Pair[];
  headers: Pair[];
  auth: Auth;
  body: Body;
  options: RequestOptions;
  /** Variables kept from the response: name → `$.token`, `header Location`… */
  captures: Pair[];
  checks: Check[];
  /** JavaScript before and after the request. */
  scripts: { before: string; after: string };
}

export interface RequestItem {
  /** Path from the collection root without `.toml`: `orders/01-create`. */
  id: string;
  name: string;
  method: string;
  folder: string;
}

export interface CollectionInfo {
  root: string;
  name: string;
  environments: string[];
  /** Every folder, empty ones too: `orders`, `orders/archive`. */
  folders: string[];
  requests: RequestItem[];
  broken: string[];
}

/** Milliseconds. */
export interface Timings {
  dns: number;
  connect: number;
  tls: number | null;
  wait: number;
  download: number;
  total: number;
}

export interface ResponseMeta {
  responseId: number;
  status: number;
  reason: string;
  version: string;
  headers: [string, string][];
  size: number;
  wireSize: number;
  timings: Timings;
  url: string;
  remoteAddr: string;
  tls: { version: string; alpn: string | null } | null;
  redirects: { status: number; url: string }[];
  isJson: boolean;
  contentType: string | null;
  receivedAt: number;
  sent: { method: string; url: string; headers: [string, string][]; bodySize: number };
  /** Absent in history entries from before checks existed. */
  checks?: CheckResult[];
  captures?: Captured[];
  /** Requests sent first for the values this one needed. */
  before?: Before[];
  /** What the scripts wrote with console.log. */
  logs?: string[];
}

export interface CheckResult {
  check: string;
  ok: boolean;
  actual: string | null;
  error: string | null;
}

export interface Captured {
  name: string;
  value: string | null;
  error: string | null;
}

export interface Before {
  id: string;
  status: number | null;
  passed: boolean;
  failures: string[];
}

/** A request of a run's plan. */
export interface RunStep {
  id: string;
  method: string;
  name: string | null;
}

export interface RunProgress {
  steps: RunStep[];
  running: Record<string, number>;
  results: RunResult[];
}

export interface RunResult {
  id: string;
  /** Its place in the plan; none for a request sent first for a missing value. */
  step: number | null;
  attempts: number;
  method: string;
  status: number | null;
  totalMs: number | null;
  size: number | null;
  passed: boolean;
  failures: string[];
  checks: number;
  checksPassed: number;
}

export interface ViewInfo {
  viewId: number;
  lines: number;
  bytes: number;
  pretty: boolean;
  /** Why the pretty view fell back to the raw one. */
  note: string | null;
}

export interface Line {
  n: number;
  text: string;
  foldEnd: number | null;
  /** UTF-16 ranges of search matches. */
  marks: [number, number][];
}

export interface SearchResult {
  total: number;
  lines: number[];
  capped: boolean;
}

/** The result of a file operation: the new id (empty when something was deleted). */
export interface ImportPreview {
  /** «Insomnia», «коллекция Postman», «OpenAPI»… */
  format: string;
  name: string;
  requests: number;
  folders: number;
  environments: string[];
  vars: number;
  warnings: string[];
}

export interface Imported {
  written: { root: string; folder: string; requests: number; environments: string[]; kept: string[] };
  collection: CollectionInfo;
}

export interface Reorganized {
  id: string;
  collection: CollectionInfo;
}

export interface HistoryItem {
  id: string;
  /** Unix milliseconds. */
  at: number;
  requestId: string | null;
  env: string | null;
  method: string;
  url: string;
  name: string | null;
  status: number | null;
  error: string | null;
  totalMs: number | null;
  size: number | null;
  hasBody: boolean;
}

export interface HistoryEntry {
  requestId: string | null;
  env: string | null;
  request: Request;
  /** Null when the body was not kept (over 1 MB) or the request failed. */
  response: ResponseMeta | null;
  status: number | null;
  error: string | null;
  at: number;
}

export async function call<T>(method: string, args: object = {}): Promise<T> {
  const bridge = window.__TSUBAMEDO_BRIDGE__;
  if (bridge) {
    const response = await fetch(bridge, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ method, args }),
    });
    const reply = await response.json();
    if ("error" in reply) throw new Error(reply.error);
    return reply.ok as T;
  }
  try {
    return await invoke<T>("call", { method, args });
  } catch (e) {
    throw new Error(String(e));
  }
}

export const api = {
  openCollection: (path: string) => call<CollectionInfo>("openCollection", { path }),
  reloadCollection: () => call<CollectionInfo>("reloadCollection"),
  getRequest: (id: string) => call<Request>("getRequest", { id }),
  saveRequest: (id: string, request: Request) => call<{ written: boolean }>("saveRequest", { id, request }),
  createRequest: (folder: string, name: string) =>
    call<{ id: string; collection: CollectionInfo }>("createRequest", { folder, name }),
  send: (sendId: string, id: string | null, request: Request, env: string | null, insecure: boolean) =>
    call<ResponseMeta>("send", { sendId, id, request, env, insecure }),
  cancel: (sendId: string) => call<null>("cancel", { sendId }),
  renameRequest: (id: string, name: string) => call<Reorganized>("renameRequest", { id, name }),
  moveRequest: (id: string, folder: string) => call<Reorganized>("moveRequest", { id, folder }),
  duplicateRequest: (id: string) => call<Reorganized>("duplicateRequest", { id }),
  deleteRequest: (id: string) => call<Reorganized>("deleteRequest", { id }),
  createFolder: (folder: string, name: string) => call<Reorganized>("createFolder", { folder, name }),
  renameFolder: (folder: string, name: string) => call<Reorganized>("renameFolder", { folder, name }),
  moveFolder: (folder: string, parent: string) => call<Reorganized>("moveFolder", { folder, parent }),
  deleteFolder: (folder: string) => call<Reorganized>("deleteFolder", { folder }),
  history: (requestId: string | null) => call<HistoryItem[]>("history", { requestId }),
  openHistory: (id: string) => call<HistoryEntry>("openHistory", { id }),
  clearHistory: () => call<null>("clearHistory"),
  vars: (env: string | null) => call<VarInfo[]>("vars", { env }),
  importPreview: (path: string) => call<ImportPreview>("importPreview", { path }),
  /** `parent` makes a new collection there; without it the file goes into the open one. */
  importCollection: (path: string, parent: string | null) => call<Imported>("importCollection", { path, parent }),
  importCurl: (text: string) => call<Request>("importCurl", { text }),
  curl: (id: string | null, request: Request, env: string | null) => call<string>("curl", { id, request, env }),
  /** Lets the core drop the bodies of responses up to `seen` that no tab shows. */
  retain: (responses: number[], seen: number) => call<null>("retain", { responses, seen }),
  view: (responseId: number, pretty: boolean) => call<ViewInfo>("view", { responseId, pretty }),
  viewLines: (viewId: number, lines: number[], query: string | null, caseSensitive: boolean) =>
    call<Line[]>("viewLines", { viewId, lines, query, caseSensitive }),
  viewSearch: (viewId: number, query: string, caseSensitive: boolean) =>
    call<SearchResult>("viewSearch", { viewId, query, caseSensitive }),
  /** `[open, close, open, close, …]` of the ranges inside the top-level object or array. */
  viewFolds: (viewId: number) => call<number[]>("viewFolds", { viewId }),
  bodyText: (responseId: number, viewId: number | null) => call<string>("bodyText", { responseId, viewId }),
  saveBody: (responseId: number, viewId: number | null, path: string) =>
    call<null>("saveBody", { responseId, viewId, path }),
  /** The collection (`folder` empty) or a folder of it, each request after those it takes values from. */
  runCollection: (sendId: string, folder: string, env: string | null, parallel: number, retries: number, only: string[] | null) =>
    call<{ steps: RunStep[]; results: RunResult[]; totalMs: number }>("runCollection", { sendId, folder, env, parallel, retries, only }),
  /** What a run has done so far; null once it is over. */
  runProgress: (sendId: string) => call<RunProgress | null>("runProgress", { sendId }),
  cookies: (env: string | null) => call<Cookie[]>("cookies", { env }),
  deleteCookie: (env: string | null, c: Cookie) => call<null>("deleteCookie", { env, domain: c.domain, path: c.path, name: c.name }),
  clearCookies: (env: string | null) => call<null>("clearCookies", { env }),
  /** The kept token for the request's OAuth 2.0 settings, or null. */
  oauthStatus: (request: Request, env: string | null) => call<TokenInfo | null>("oauthStatus", { request, env }),
  /** A new token now; a sign-in in the browser waits until `cancel(sendId)`. */
  oauthToken: (sendId: string, request: Request, env: string | null) => call<TokenInfo>("oauthToken", { sendId, request, env }),
  oauthForget: (request: Request, env: string | null) => call<null>("oauthForget", { request, env }),
  /** Why a check line is not understood, or null. */
  validateCheck: (text: string) => call<string | null>("validateCheck", { text }),
};

export const emptyRequest = (): Request => ({
  name: null,
  method: "GET",
  url: "",
  query: [],
  headers: [],
  auth: { type: "none" },
  body: { type: "none" },
  options: { followRedirects: null, timeoutMs: null, verifyTls: null },
  captures: [],
  checks: [],
  scripts: { before: "", after: "" },
});
