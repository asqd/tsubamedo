// What a request script sees: vars, request, response, test/expect, console, crypto, btoa/atob.
// The core passes the input as JSON in __input and reads the result back with __output().
"use strict";
(() => {
  const input = JSON.parse(globalThis.__input);
  delete globalThis.__input;
  const out = { vars: {}, tests: [], logs: [], request: input.request };
  const native = { hash: globalThis.__hash, hmac: globalThis.__hmac, b64: globalThis.__b64, uuid: globalThis.__uuid };
  for (const name of ["__hash", "__hmac", "__b64", "__uuid"]) delete globalThis[name];
  Object.defineProperty(globalThis, "__output", { value: () => JSON.stringify(out) });

  const show = (v) => {
    if (typeof v === "string") return v;
    if (v === undefined) return "undefined";
    if (typeof v === "function") return "функция";
    if (v instanceof Error) return `${v.name}: ${v.message}`;
    try {
      return JSON.stringify(v) ?? String(v);
    } catch {
      return String(v);
    }
  };
  const quoted = (v) => (typeof v === "string" ? JSON.stringify(v) : show(v));

  // ---- variables ----
  const own = (o, k) => Object.prototype.hasOwnProperty.call(o, k);
  // Names the script removed: for it they are gone, other requests still have theirs.
  const removed = new Set();
  const vars = {
    get(name) {
      name = String(name);
      if (removed.has(name)) return undefined;
      return own(out.vars, name) ? out.vars[name] : own(input.vars, name) ? input.vars[name] : undefined;
    },
    set(name, value) {
      removed.delete(String(name));
      out.vars[String(name)] = value == null ? "" : typeof value === "object" ? JSON.stringify(value) : String(value);
    },
    has(name) {
      return vars.get(name) !== undefined;
    },
    unset(name) {
      delete out.vars[String(name)];
      removed.add(String(name));
    },
    /** Every variable, as an object. */
    all() {
      const all = { ...input.vars, ...out.vars };
      for (const name of removed) delete all[name];
      return all;
    },
    /** `{{name}}` in a text replaced, with variables inside variables. */
    render(text) {
      let result = String(text);
      for (let depth = 0; depth < 10; depth++) {
        const next = result.replace(/\{\{\s*([^{}\s$][^{}]*?)\s*\}\}/g, (m, n) => vars.get(n) ?? m);
        if (next === result) break;
        result = next;
      }
      return result;
    },
  };
  globalThis.vars = Object.freeze(vars);

  // ---- request and response ----
  const pairs = (list, ignoreCase, readonly, what) => {
    const same = (a, b) => (ignoreCase ? a.toLowerCase() === b.toLowerCase() : a === b);
    const guard = () => {
      if (readonly) throw new Error(`${what} уже отправлены — менять их можно в скрипте до запроса`);
    };
    return Object.freeze({
      get(name) {
        const p = list.find((p) => p.enabled && same(p.name, String(name)));
        return p ? p.value : undefined;
      },
      has(name) {
        return list.some((p) => p.enabled && same(p.name, String(name)));
      },
      set(name, value) {
        guard();
        name = String(name);
        const i = list.findIndex((p) => same(p.name, name));
        const pair = { name: i >= 0 ? list[i].name : name, value: String(value), enabled: true };
        if (i >= 0) list[i] = pair;
        else list.push(pair);
      },
      remove(name) {
        guard();
        for (let i = list.length - 1; i >= 0; i--) if (same(list[i].name, String(name))) list.splice(i, 1);
      },
      all() {
        return Object.fromEntries(list.filter((p) => p.enabled).map((p) => [p.name, p.value]));
      },
    });
  };

  const r = out.request;
  const sent = Boolean(input.response);
  const fixed = (what) => {
    if (sent) throw new Error(`${what}: запрос уже отправлен — менять его можно в скрипте до запроса`);
  };
  globalThis.request = Object.freeze({
    get method() {
      return r.method;
    },
    set method(v) {
      fixed("method");
      r.method = String(v).toUpperCase();
    },
    get url() {
      return r.url;
    },
    set url(v) {
      fixed("url");
      r.url = String(v);
    },
    headers: pairs(r.headers, true, sent, "Заголовки"),
    query: pairs(r.query, false, sent, "Параметры"),
    get body() {
      return r.body;
    },
    set body(v) {
      fixed("body");
      if (r.body === null) throw new Error("менять можно только тело JSON или текст");
      r.body = typeof v === "string" ? v : JSON.stringify(v, null, 2);
    },
  });

  if (input.response) {
    const res = input.response;
    globalThis.response = Object.freeze({
      status: res.status,
      statusText: res.reason,
      time: res.time,
      size: res.size,
      headers: pairs(res.headers, true, true, "Заголовки ответа"),
      text() {
        if (res.body === null) throw new Error(res.bodyError);
        return res.body;
      },
      json() {
        try {
          return JSON.parse(this.text());
        } catch (e) {
          throw new Error(`ответ не JSON: ${e.message}`);
        }
      },
    });
  }

  // ---- tests ----
  const deepEqual = (a, b) => {
    if (Object.is(a, b)) return true;
    if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
    if (Array.isArray(a) !== Array.isArray(b)) return false;
    const ka = Object.keys(a);
    const kb = Object.keys(b);
    return ka.length === kb.length && ka.every((k) => own(b, k) && deepEqual(a[k], b[k]));
  };
  const lengthOf = (v) => (v != null && typeof v.length === "number" ? v.length : undefined);
  const matchers = (actual, not) => {
    const check = (pass, wanted) => {
      if (Boolean(pass) === not) throw new Error(`${not ? "не " : ""}ожидалось ${wanted}, пришло ${quoted(actual)}`);
    };
    const number = (n) => {
      if (typeof actual !== "number") throw new Error(`ожидалось число, пришло ${quoted(actual)}`);
      return Number(n);
    };
    return {
      get not() {
        return matchers(actual, !not);
      },
      toBe: (e) => check(Object.is(actual, e), quoted(e)),
      toEqual: (e) => check(deepEqual(actual, e), quoted(e)),
      toBeTruthy: () => check(actual, "истинное значение"),
      toBeFalsy: () => check(!actual, "ложное значение"),
      toBeDefined: () => check(actual !== undefined, "значение"),
      toBeUndefined: () => check(actual === undefined, "undefined"),
      toBeNull: () => check(actual === null, "null"),
      toBeGreaterThan: (n) => check(actual > number(n), `больше ${n}`),
      toBeGreaterThanOrEqual: (n) => check(actual >= number(n), `не меньше ${n}`),
      toBeLessThan: (n) => check(actual < number(n), `меньше ${n}`),
      toBeLessThanOrEqual: (n) => check(actual <= number(n), `не больше ${n}`),
      toContain: (e) =>
        check(
          typeof actual === "string" ? actual.includes(String(e)) : Array.isArray(actual) && actual.some((x) => deepEqual(x, e)),
          `что-то с ${quoted(e)}`,
        ),
      toMatch: (re) => check(typeof actual === "string" && (re instanceof RegExp ? re : new RegExp(re)).test(actual), `совпадение с ${re}`),
      toHaveLength: (n) => check(lengthOf(actual) === n, `длину ${n}`),
      toHaveProperty: (path) => {
        let v = actual;
        for (const key of String(path).split(".")) v = v != null && own(Object(v), key) ? v[key] : undefined;
        check(v !== undefined, `поле ${path}`);
      },
    };
  };
  globalThis.expect = (actual) => matchers(actual, false);
  globalThis.test = (name, fn) => {
    try {
      const result = fn();
      if (result && typeof result.then === "function") throw new Error("асинхронные тесты не поддерживаются");
      out.tests.push({ name: String(name), ok: true, error: null });
    } catch (e) {
      out.tests.push({ name: String(name), ok: false, error: e instanceof Error ? e.message : show(e) });
    }
  };

  // ---- console, crypto, base64 ----
  const log = (prefix) => (...args) => out.logs.push(prefix + args.map(show).join(" "));
  globalThis.console = Object.freeze({ log: log(""), info: log(""), debug: log(""), warn: log("⚠ "), error: log("✗ ") });
  const enc = (e) => (e === undefined ? "hex" : String(e));
  globalThis.crypto = Object.freeze({
    sha1: (text, e) => native.hash("sha1", String(text), enc(e)),
    sha256: (text, e) => native.hash("sha256", String(text), enc(e)),
    sha512: (text, e) => native.hash("sha512", String(text), enc(e)),
    hmac: (alg, key, text, e) => native.hmac(String(alg), String(key), String(text), enc(e)),
    randomUUID: () => native.uuid(),
  });
  globalThis.btoa = (text) => native.b64(String(text), false);
  globalThis.atob = (text) => native.b64(String(text), true);
})();
