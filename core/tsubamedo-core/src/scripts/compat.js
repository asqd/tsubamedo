// Postman's `pm` (and its older `postman`, `tests[…]`) and Insomnia's `insomnia`, over our API,
// so scripts imported from those tools keep working. Only what such scripts use most.
"use strict";
(() => {
  // The prelude's language helper, taken off the global object before any user script runs.
  const t = globalThis.__t;
  delete globalThis.__t;
  const quoted = (v) => {
    if (typeof v === "string") return JSON.stringify(v);
    try {
      return JSON.stringify(v) ?? String(v);
    } catch {
      return String(v);
    }
  };
  const fail = (what) => {
    throw new Error(t(`${what} в TsubameDo нет: у скриптов нет сети и файлов`, `${what} is not available in TsubameDo: scripts have no network or files`));
  };

  // ---- variables: every Postman scope is our one set ----
  const scope = Object.freeze({
    get: (n) => vars.get(n),
    set: (n, v) => vars.set(n, v),
    unset: (n) => vars.unset(n),
    has: (n) => vars.has(n),
    replaceIn: (t) => vars.render(t),
    toObject: () => vars.all(),
    clear() {},
  });

  // ---- chai's expect, as far as test scripts go ----
  const own = (o, k) => o != null && Object.prototype.hasOwnProperty.call(Object(o), k);
  const deepEqual = (a, b) => {
    if (Object.is(a, b)) return true;
    if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
    if (Array.isArray(a) !== Array.isArray(b)) return false;
    const ka = Object.keys(a);
    const kb = Object.keys(b);
    return ka.length === kb.length && ka.every((k) => own(b, k) && deepEqual(a[k], b[k]));
  };
  const typeOf = (v) => (v === null ? "null" : Array.isArray(v) ? "array" : v instanceof RegExp ? "regexp" : typeof v);
  class Assertion {
    constructor(value, message) {
      this.value = value;
      this.message = message;
      this.negate = false;
      this.isDeep = false;
    }
    check(pass, wanted) {
      if (Boolean(pass) === this.negate) {
        const prefix = this.message ? `${this.message}: ` : "";
        const not = this.negate;
        throw new Error(prefix + t(`${not ? "не " : ""}ожидалось ${wanted}, пришло ${quoted(this.value)}`, `expected ${not ? "not " : ""}${wanted}, got ${quoted(this.value)}`));
      }
      return this;
    }
    get not() {
      this.negate = !this.negate;
      return this;
    }
    get deep() {
      this.isDeep = true;
      return this;
    }
    get ok() {
      return this.check(this.value, t("истинное значение", "a truthy value"));
    }
    get true() {
      return this.check(this.value === true, "true");
    }
    get false() {
      return this.check(this.value === false, "false");
    }
    get null() {
      return this.check(this.value === null, "null");
    }
    get undefined() {
      return this.check(this.value === undefined, "undefined");
    }
    get NaN() {
      return this.check(Number.isNaN(this.value), "NaN");
    }
    get exist() {
      return this.check(this.value != null, t("значение", "a value"));
    }
    get exists() {
      return this.exist;
    }
    get empty() {
      const v = this.value;
      const size = typeof v === "string" || Array.isArray(v) ? v.length : v && typeof v === "object" ? Object.keys(v).length : NaN;
      return this.check(size === 0, t("пустое значение", "an empty value"));
    }
    a(type) {
      return this.check(typeOf(this.value) === String(type).toLowerCase(), t(`тип ${type}`, `type ${type}`));
    }
    an(type) {
      return this.a(type);
    }
    equal(v) {
      return this.check(this.isDeep ? deepEqual(this.value, v) : Object.is(this.value, v), quoted(v));
    }
    equals(v) {
      return this.equal(v);
    }
    eq(v) {
      return this.equal(v);
    }
    eql(v) {
      return this.check(deepEqual(this.value, v), quoted(v));
    }
    include(v) {
      const a = this.value;
      let pass;
      if (typeof a === "string") pass = a.includes(String(v));
      else if (Array.isArray(a)) pass = a.some((x) => (this.isDeep ? deepEqual(x, v) : Object.is(x, v)));
      else if (a && typeof a === "object" && v && typeof v === "object") pass = Object.keys(v).every((k) => own(a, k) && deepEqual(a[k], v[k]));
      else pass = false;
      return this.check(pass, t(`что-то с ${quoted(v)}`, `something with ${quoted(v)}`));
    }
    includes(v) {
      return this.include(v);
    }
    contain(v) {
      return this.include(v);
    }
    contains(v) {
      return this.include(v);
    }
    above(n) {
      return this.check(this.value > n, t(`больше ${n}`, `more than ${n}`));
    }
    gt(n) {
      return this.above(n);
    }
    greaterThan(n) {
      return this.above(n);
    }
    below(n) {
      return this.check(this.value < n, t(`меньше ${n}`, `less than ${n}`));
    }
    lt(n) {
      return this.below(n);
    }
    lessThan(n) {
      return this.below(n);
    }
    least(n) {
      return this.check(this.value >= n, t(`не меньше ${n}`, `at least ${n}`));
    }
    gte(n) {
      return this.least(n);
    }
    most(n) {
      return this.check(this.value <= n, t(`не больше ${n}`, `at most ${n}`));
    }
    lte(n) {
      return this.most(n);
    }
    within(a, b) {
      return this.check(this.value >= a && this.value <= b, t(`от ${a} до ${b}`, `from ${a} to ${b}`));
    }
    lengthOf(n) {
      return this.check(this.value != null && this.value.length === n, t(`длину ${n}`, `length ${n}`));
    }
    length(n) {
      return this.lengthOf(n);
    }
    property(name, ...value) {
      const v = this.value;
      let found = own(v, name);
      let got = found ? v[name] : undefined;
      if (!found && String(name).includes(".")) {
        got = String(name)
          .split(".")
          .reduce((o, k) => (own(o, k) ? o[k] : undefined), v);
        found = got !== undefined;
      }
      if (value.length === 0) this.check(found, t(`поле ${name}`, `property ${name}`));
      else this.check(found && deepEqual(got, value[0]), t(`поле ${name} = ${quoted(value[0])}`, `property ${name} = ${quoted(value[0])}`));
      // As in chai, what follows is about the field: `property("user").that.has.property("id")`.
      return this.negate ? this : new Assertion(got, this.message);
    }
    keys(...list) {
      const wanted = list.length === 1 && Array.isArray(list[0]) ? list[0] : list;
      return this.check(this.value && wanted.every((k) => own(this.value, k)), t(`поля ${wanted.join(", ")}`, `properties ${wanted.join(", ")}`));
    }
    members(list) {
      const a = this.value;
      return this.check(Array.isArray(a) && list.every((x) => a.some((y) => deepEqual(x, y))), t(`элементы ${quoted(list)}`, `members ${quoted(list)}`));
    }
    match(re) {
      return this.check(typeof this.value === "string" && re.test(this.value), t(`совпадение с ${re}`, `a match for ${re}`));
    }
    string(s) {
      return this.check(typeof this.value === "string" && this.value.includes(s), t(`строку с ${quoted(s)}`, `a string with ${quoted(s)}`));
    }
    oneOf(list) {
      return this.check(list.some((x) => deepEqual(x, this.value)), t(`одно из ${quoted(list)}`, `one of ${quoted(list)}`));
    }
  }
  // Words that only make the sentence read: `to.be.an`, `that.has`…
  for (const word of ["to", "be", "been", "is", "that", "which", "and", "has", "have", "with", "at", "of", "same", "does", "still", "also", "all", "any", "nested", "own"]) {
    Object.defineProperty(Assertion.prototype, word, {
      get() {
        return this;
      },
    });
  }
  const chai = (value, message) => new Assertion(value, message);
  chai.fail = (message) => {
    throw new Error(message || t("провал", "failed"));
  };

  // ---- request ----
  const listOf = (pairs) =>
    Object.freeze({
      get: (n) => pairs.get(n),
      has: (n) => pairs.has(n),
      add: (h) => pairs.set(h.key, h.value),
      upsert: (h) => pairs.set(h.key, h.value),
      remove: (n) => pairs.remove(typeof n === "string" ? n : n.key),
      toObject: () => pairs.all(),
      each: (fn) => Object.entries(pairs.all()).forEach(([key, value]) => fn({ key, value })),
    });
  const url = Object.freeze({
    toString: () => request.url,
    query: listOf(request.query),
    getQueryString: () =>
      Object.entries(request.query.all())
        .map(([k, v]) => `${k}=${v}`)
        .join("&"),
  });
  const pmRequest = Object.freeze({
    get method() {
      return request.method;
    },
    set method(v) {
      request.method = v;
    },
    get url() {
      return url;
    },
    set url(v) {
      request.url = String(v);
    },
    headers: listOf(request.headers),
    get body() {
      const raw = request.body;
      return raw == null ? undefined : { mode: "raw", raw, toString: () => raw };
    },
    set body(v) {
      request.body = v && typeof v === "object" && "raw" in v ? v.raw : v;
    },
    addHeader: (h) => request.headers.set(h.key, h.value),
    upsertHeader: (h) => request.headers.set(h.key, h.value),
    removeHeader: (n) => request.headers.remove(typeof n === "string" ? n : n.key),
    addQueryParams: (list) => (Array.isArray(list) ? list : [list]).forEach((p) => request.query.set(p.key, p.value)),
    removeQueryParams: (list) => (Array.isArray(list) ? list : [list]).forEach((p) => request.query.remove(typeof p === "string" ? p : p.key)),
  });

  // ---- response ----
  const res = globalThis.response;
  let pmResponse;
  if (res) {
    const to = (negate) => {
      const check = (pass, wanted) => {
        if (Boolean(pass) === negate) {
          throw new Error(t(`${negate ? "не " : ""}ожидался ${wanted}, пришёл статус ${res.status}`, `expected ${negate ? "not " : ""}${wanted}, got status ${res.status}`));
        }
      };
      const status = (s) => check(typeof s === "number" ? res.status === s : res.statusText === s, t(`статус ${quoted(s)}`, `status ${quoted(s)}`));
      const header = (name, value) =>
        check(value === undefined ? res.headers.has(name) : res.headers.get(name) === String(value), t(`заголовок ${name}`, `header ${name}`) + (value === undefined ? "" : ` = ${quoted(value)}`));
      const isJson = () => {
        try {
          res.json();
          return true;
        } catch {
          return false;
        }
      };
      const be = {
        get ok() {
          return check(res.status === 200, t("статус 200", "status 200"));
        },
        get success() {
          return check(res.status >= 200 && res.status < 300, t("успешный статус", "a success status"));
        },
        get error() {
          return check(res.status >= 400, t("статус ошибки", "an error status"));
        },
        get clientError() {
          return check(res.status >= 400 && res.status < 500, t("статус 4xx", "status 4xx"));
        },
        get serverError() {
          return check(res.status >= 500, t("статус 5xx", "status 5xx"));
        },
        get notFound() {
          return check(res.status === 404, t("статус 404", "status 404"));
        },
        get json() {
          return check(isJson(), t("ответ JSON", "a JSON response"));
        },
      };
      return { have: { status, header, jsonBody: () => check(isJson(), t("ответ JSON", "a JSON response")) }, be, get not() { return to(!negate); } };
    };
    pmResponse = Object.freeze({
      code: res.status,
      status: res.statusText,
      responseTime: res.time,
      responseSize: res.size,
      headers: listOf(res.headers),
      json: () => res.json(),
      text: () => res.text(),
      cookies: res.cookies,
      to: to(false),
    });
  }

  const pmTest = (name, fn) => test(name, fn);
  pmTest.skip = () => {};
  const pm = Object.freeze({
    environment: scope,
    collectionVariables: scope,
    globals: scope,
    variables: scope,
    baseEnvironment: scope,
    iterationData: Object.freeze({ get: () => undefined, has: () => false, toObject: () => ({}) }),
    request: pmRequest,
    response: pmResponse,
    test: pmTest,
    expect: chai,
    info: Object.freeze({ eventName: res ? "test" : "prerequest", iteration: 0, iterationCount: 1 }),
    sendRequest: () => fail("pm.sendRequest"),
    cookies: Object.freeze({ ...globalThis.cookies, jar: () => fail("pm.cookies.jar()") }),
    visualizer: Object.freeze({ set() {} }),
    execution: Object.freeze({ setNextRequest() {}, skipRequest: () => fail("pm.execution.skipRequest") }),
    require: (name) => modules(name),
  });
  globalThis.pm = pm;
  globalThis.insomnia = pm;

  // ---- older Postman scripts ----
  globalThis.postman = Object.freeze({
    setEnvironmentVariable: (n, v) => vars.set(n, v),
    getEnvironmentVariable: (n) => vars.get(n),
    clearEnvironmentVariable: (n) => vars.unset(n),
    setGlobalVariable: (n, v) => vars.set(n, v),
    getGlobalVariable: (n) => vars.get(n),
    clearGlobalVariable: (n) => vars.unset(n),
    getResponseHeader: (n) => (res ? res.headers.get(n) : undefined),
    setNextRequest() {},
  });
  globalThis.tests = new Proxy(
    {},
    {
      set(_, name, value) {
        test(String(name), () => {
          if (!value) throw new Error(t("условие ложно", "the condition is false"));
        });
        return true;
      },
    },
  );
  if (res) {
    Object.defineProperty(globalThis, "responseBody", { get: () => res.text() });
    globalThis.responseCode = Object.freeze({ code: res.status, name: res.statusText });
    globalThis.responseTime = res.time;
    globalThis.responseHeaders = res.headers.all();
  }

  // ---- crypto-js, the library such scripts sign requests with ----
  const ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const base64 = (hex) => {
    const bytes = (hex.match(/../g) || []).map((h) => parseInt(h, 16));
    let out = "";
    for (let i = 0; i < bytes.length; i += 3) {
      const n = (bytes[i] << 16) | ((bytes[i + 1] ?? 0) << 8) | (bytes[i + 2] ?? 0);
      const left = bytes.length - i;
      out += ALPHABET[(n >> 18) & 63] + ALPHABET[(n >> 12) & 63] + (left > 1 ? ALPHABET[(n >> 6) & 63] : "=") + (left > 2 ? ALPHABET[n & 63] : "=");
    }
    return out;
  };
  const words = (hex) => Object.freeze({ __hex: hex, toString: (encoding) => (encoding === CryptoJS.enc.Base64 ? base64(hex) : hex) });
  const textOf = (v) => (v && typeof v === "object" && "__text" in v ? v.__text : String(v));
  const hash = (alg) => (message) => words(crypto[alg](textOf(message)));
  const hmac = (alg) => (message, key) => words(crypto.hmac(alg, textOf(key), textOf(message)));
  const CryptoJS = Object.freeze({
    SHA1: hash("sha1"),
    SHA256: hash("sha256"),
    SHA512: hash("sha512"),
    HmacSHA1: hmac("sha1"),
    HmacSHA256: hmac("sha256"),
    HmacSHA512: hmac("sha512"),
    MD5: () => fail("MD5"),
    enc: Object.freeze({
      Hex: Object.freeze({ stringify: (w) => w.toString() }),
      Base64: Object.freeze({
        stringify: (w) => ("__hex" in w ? w.toString(CryptoJS.enc.Base64) : btoa(textOf(w))),
        parse: (s) => Object.freeze({ __text: atob(s), toString: () => atob(s) }),
      }),
      Utf8: Object.freeze({ parse: (s) => Object.freeze({ __text: String(s), toString: () => String(s) }), stringify: (w) => textOf(w) }),
    }),
  });
  globalThis.CryptoJS = CryptoJS;
  const modules = (name) => {
    if (name === "crypto-js") return CryptoJS;
    if (name === "chai") return { expect: chai };
    throw new Error(t(`модуля ${name} в TsubameDo нет`, `there is no module ${name} in TsubameDo`));
  };
  globalThis.require = modules;
})();
