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
  | { type: "apikey"; key: string; value: string; location: "header" | "query" };

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
});
