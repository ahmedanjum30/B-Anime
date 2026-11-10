// Sandbox setup, run before every extension: console, fetch and the small set of web APIs
// extensions expect (atob/btoa, URL, URLSearchParams, TextEncoder/TextDecoder, setTimeout).
// The raw host functions move into a frozen, non-enumerable `__host` object.
(() => {
  const host = Object.freeze({
    fetch: globalThis.__host_fetch,
    log: globalThis.__host_log,
    htmlParse: globalThis.__host_html_parse,
    html: globalThis.__host_html,
    crypto: globalThis.__host_crypto,
    url: globalThis.__host_url,
    sleep: globalThis.__host_sleep,
  });
  for (const k of ["__host_fetch", "__host_log", "__host_html_parse", "__host_html", "__host_crypto", "__host_url", "__host_sleep"]) delete globalThis[k];
  Object.defineProperty(globalThis, "__host", { value: host, enumerable: false });

  const show = (v) => {
    if (typeof v === "string") return v;
    if (v instanceof Error) return `${v.name}: ${v.message}`;
    try { return JSON.stringify(v); } catch { return String(v); }
  };
  const log = (level) => (...args) => host.log(level, args.map(show).join(" "));
  globalThis.console = { log: log("info"), info: log("info"), debug: log("debug"), warn: log("warn"), error: log("error"), trace: log("debug") };

  globalThis.fetch = async (url, options = {}) => {
    let body = options.body;
    const headers = { ...(options.headers || {}) };
    if (body instanceof URLSearchParams) {
      body = body.toString();
      if (!Object.keys(headers).some((k) => k.toLowerCase() === "content-type")) headers["Content-Type"] = "application/x-www-form-urlencoded";
    } else if (body != null && typeof body !== "string") {
      body = JSON.stringify(body);
    }
    const r = JSON.parse(await host.fetch(String(url), JSON.stringify({
      method: options.method || "GET",
      headers,
      body: body == null ? null : body,
      redirect: options.redirect || "follow",
    })));
    if (r.error) throw new Error(r.error);
    const headerObj = {
      get: (name) => r.headers[String(name).toLowerCase()] ?? null,
      has: (name) => String(name).toLowerCase() in r.headers,
      entries: () => Object.entries(r.headers),
      forEach: (fn) => Object.entries(r.headers).forEach(([k, v]) => fn(v, k)),
      getSetCookie: () => r.setCookies.slice(),
    };
    return {
      ok: r.status >= 200 && r.status < 300,
      status: r.status,
      statusText: r.statusText,
      url: r.url,
      redirected: r.redirected,
      headers: headerObj,
      text: async () => r.body,
      json: async () => JSON.parse(r.body),
    };
  };

  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  globalThis.btoa = (input) => {
    const s = String(input);
    let out = "";
    for (let i = 0; i < s.length; i += 3) {
      const a = s.charCodeAt(i), b = s.charCodeAt(i + 1), c = s.charCodeAt(i + 2);
      if (a > 255 || b > 255 || c > 255) throw new Error("btoa: character out of range");
      const n = (a << 16) | ((b || 0) << 8) | (c || 0);
      out += chars[(n >> 18) & 63] + chars[(n >> 12) & 63] + (i + 1 < s.length ? chars[(n >> 6) & 63] : "=") + (i + 2 < s.length ? chars[n & 63] : "=");
    }
    return out;
  };
  globalThis.atob = (input) => {
    const s = String(input).replace(/[\s=]+/g, "").replace(/-/g, "+").replace(/_/g, "/");
    let out = "", buffer = 0, bits = 0;
    for (const ch of s) {
      const v = chars.indexOf(ch);
      if (v < 0) throw new Error("atob: invalid character");
      buffer = ((buffer << 6) | v) & 0xffffff;
      bits += 6;
      if (bits >= 8) { bits -= 8; out += String.fromCharCode((buffer >> bits) & 255); }
    }
    return out;
  };

  // UTF-8 without relying on engine extras.
  const utf8Encode = (str) => {
    const out = [];
    for (const ch of String(str)) {
      let c = ch.codePointAt(0);
      if (c < 0x80) out.push(c);
      else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
      else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
      else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    }
    return new Uint8Array(out);
  };
  const utf8Decode = (bytes) => {
    const b = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    let out = "";
    for (let i = 0; i < b.length;) {
      const x = b[i++];
      let c;
      if (x < 0x80) c = x;
      else if (x >= 0xf0) { c = ((x & 7) << 18) | ((b[i++] & 63) << 12) | ((b[i++] & 63) << 6) | (b[i++] & 63); }
      else if (x >= 0xe0) { c = ((x & 15) << 12) | ((b[i++] & 63) << 6) | (b[i++] & 63); }
      else if (x >= 0xc0) { c = ((x & 31) << 6) | (b[i++] & 63); }
      else c = 0xfffd;
      out += String.fromCodePoint(c);
    }
    return out;
  };
  globalThis.TextEncoder = class TextEncoder {
    get encoding() { return "utf-8"; }
    encode(s = "") { return utf8Encode(s); }
  };
  globalThis.TextDecoder = class TextDecoder {
    constructor(label = "utf-8") { this.encoding = String(label).toLowerCase(); }
    decode(bytes) {
      if (bytes == null) return "";
      const b = bytes instanceof ArrayBuffer ? new Uint8Array(bytes) : bytes;
      if (this.encoding === "latin1" || this.encoding === "iso-8859-1") return String.fromCharCode(...b);
      return utf8Decode(b);
    }
  };

  class URLSearchParams {
    constructor(init) {
      this._list = [];
      if (init == null) return;
      if (typeof init === "string") {
        for (const part of init.replace(/^\?/, "").split("&")) {
          if (!part) continue;
          const i = part.indexOf("=");
          const dec = (s) => decodeURIComponent(s.replace(/\+/g, " "));
          this._list.push(i < 0 ? [dec(part), ""] : [dec(part.slice(0, i)), dec(part.slice(i + 1))]);
        }
      } else if (Array.isArray(init) || typeof init[Symbol.iterator] === "function") {
        for (const [k, v] of init) this._list.push([String(k), String(v)]);
      } else {
        for (const k of Object.keys(init)) this._list.push([k, String(init[k])]);
      }
    }
    _changed() { if (this._url) this._url._setSearch(this.toString()); }
    append(k, v) { this._list.push([String(k), String(v)]); this._changed(); }
    delete(k) { this._list = this._list.filter(([x]) => x !== String(k)); this._changed(); }
    get(k) { const e = this._list.find(([x]) => x === String(k)); return e ? e[1] : null; }
    getAll(k) { return this._list.filter(([x]) => x === String(k)).map(([, v]) => v); }
    has(k) { return this._list.some(([x]) => x === String(k)); }
    set(k, v) {
      const i = this._list.findIndex(([x]) => x === String(k));
      if (i < 0) this._list.push([String(k), String(v)]);
      else { this._list[i][1] = String(v); this._list = this._list.filter(([x], j) => j <= i || x !== String(k)); }
      this._changed();
    }
    sort() { this._list.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)); this._changed(); }
    forEach(fn) { for (const [k, v] of this._list) fn(v, k, this); }
    keys() { return this._list.map(([k]) => k)[Symbol.iterator](); }
    values() { return this._list.map(([, v]) => v)[Symbol.iterator](); }
    entries() { return this._list.map((e) => [...e])[Symbol.iterator](); }
    [Symbol.iterator]() { return this.entries(); }
    get size() { return this._list.length; }
    toString() {
      const enc = (s) => encodeURIComponent(s).replace(/%20/g, "+").replace(/[!'()~]/g, (c) => "%" + c.charCodeAt(0).toString(16).toUpperCase());
      return this._list.map(([k, v]) => `${enc(k)}=${enc(v)}`).join("&");
    }
  }
  globalThis.URLSearchParams = URLSearchParams;

  class URL {
    constructor(input, base) {
      this._load(String(input), base === undefined ? "" : String(base instanceof URL ? base.href : base));
    }
    _load(input, base = "") {
      const r = JSON.parse(host.url(input, base));
      if (r.error) throw new TypeError(r.error);
      this._p = r.value;
      this._params = new URLSearchParams(this._p.search);
      this._params._url = this;
    }
    _rebuild(parts) {
      const p = { ...this._p, ...parts };
      const auth = p.username ? `${p.username}${p.password ? ":" + p.password : ""}@` : "";
      const search = p.search && !p.search.startsWith("?") ? "?" + p.search : p.search;
      const hash = p.hash && !p.hash.startsWith("#") ? "#" + p.hash : p.hash;
      const path = p.pathname.startsWith("/") ? p.pathname : "/" + p.pathname;
      this._load(`${p.protocol}//${auth}${p.host}${path}${search}${hash}`);
    }
    // searchParams edits write back here; keep the same params object the caller holds.
    _setSearch(s) { const params = this._params; this._rebuild({ search: s ? "?" + s : "" }); this._params = params; }
    get href() { return this._p.href; }
    set href(v) { this._load(String(v)); }
    get protocol() { return this._p.protocol; }
    set protocol(v) { this._rebuild({ protocol: String(v).replace(/:?$/, ":") }); }
    get username() { return this._p.username; }
    get password() { return this._p.password; }
    get host() { return this._p.host; }
    set host(v) { this._rebuild({ host: String(v) }); }
    get hostname() { return this._p.hostname; }
    set hostname(v) { this._rebuild({ host: this._p.port ? `${v}:${this._p.port}` : String(v) }); }
    get port() { return this._p.port; }
    set port(v) { this._rebuild({ host: v === "" ? this._p.hostname : `${this._p.hostname}:${v}` }); }
    get pathname() { return this._p.pathname; }
    set pathname(v) { this._rebuild({ pathname: String(v) }); }
    get search() { return this._p.search; }
    set search(v) { this._rebuild({ search: String(v) }); }
    get hash() { return this._p.hash; }
    set hash(v) { this._rebuild({ hash: String(v) }); }
    get origin() { return this._p.origin; }
    get searchParams() { return this._params; }
    toString() { return this.href; }
    toJSON() { return this.href; }
    static canParse(input, base) { try { new URL(input, base); return true; } catch { return false; } }
  }
  globalThis.URL = URL;

  let timerId = 0;
  const cancelled = new Set();
  globalThis.setTimeout = (fn, ms = 0, ...args) => {
    const id = ++timerId;
    host.sleep(Math.max(0, Number(ms) || 0)).then(() => {
      if (!cancelled.delete(id) && typeof fn === "function") fn(...args);
    });
    return id;
  };
  globalThis.clearTimeout = (id) => { cancelled.add(id); };
  globalThis.queueMicrotask = (fn) => { Promise.resolve().then(fn); };
})();
