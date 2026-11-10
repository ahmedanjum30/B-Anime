// Compatibility shim: the globals converted providers expect from their original runtime, rebuilt on top
// of the B-Anime sandbox. Runs after prelude.js, before the converted extension's code.
// `__extConfig` (the extension's user config fields) is defined just above this file.
(() => {
  const host = globalThis.__host;
  const unwrap = (json) => {
    const r = JSON.parse(json);
    if (r.error) throw new Error(r.error);
    return r.value;
  };

  // ── bytes helpers ──────────────────────────────────────────────────────────
  const latin1ToBytes = (s) => { const b = new Uint8Array(s.length); for (let i = 0; i < s.length; i++) b[i] = s.charCodeAt(i) & 255; return b; };
  const bytesToLatin1 = (b) => { let s = ""; for (let i = 0; i < b.length; i += 8192) s += String.fromCharCode(...b.subarray(i, i + 8192)); return s; };
  const toB64 = (b) => btoa(bytesToLatin1(b));
  const fromB64 = (s) => latin1ToBytes(atob(String(s)));
  const toHex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
  const fromHex = (s) => { const t = String(s).replace(/\s+/g, ""); const b = new Uint8Array(t.length >> 1); for (let i = 0; i < b.length; i++) b[i] = parseInt(t.substr(i * 2, 2), 16); return b; };
  const utf8 = new TextEncoder();
  const utf8d = new TextDecoder();
  const utf16 = (s, le) => { const b = new Uint8Array(s.length * 2); for (let i = 0; i < s.length; i++) { const c = s.charCodeAt(i); b[i * 2 + (le ? 0 : 1)] = c & 255; b[i * 2 + (le ? 1 : 0)] = c >> 8; } return b; };
  const fromUtf16 = (b, le) => { let s = ""; for (let i = 0; i + 1 < b.length; i += 2) s += String.fromCharCode(le ? b[i] | (b[i + 1] << 8) : (b[i] << 8) | b[i + 1]); return s; };
  const asBytes = (v) => {
    if (v instanceof Uint8Array) return v;
    if (v instanceof ArrayBuffer) return new Uint8Array(v);
    if (v && v.__bytes instanceof Uint8Array) return v.__bytes;
    if (Array.isArray(v)) return Uint8Array.from(v);
    return utf8.encode(String(v ?? ""));
  };

  // ── Buffer (the subset providers use) ──────────────────────────────────────
  const encode = (str, enc = "utf8") => {
    switch (String(enc).toLowerCase()) {
      case "base64": return fromB64(str);
      case "base64url": return fromB64(String(str).replace(/-/g, "+").replace(/_/g, "/"));
      case "hex": return fromHex(str);
      case "latin1": case "binary": case "ascii": return latin1ToBytes(String(str));
      case "utf16le": case "ucs2": case "ucs-2": case "utf-16le": return utf16(String(str), true);
      default: return utf8.encode(String(str));
    }
  };
  const decode = (bytes, enc = "utf8") => {
    switch (String(enc).toLowerCase()) {
      case "base64": return toB64(bytes);
      case "base64url": return toB64(bytes).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
      case "hex": return toHex(bytes);
      case "latin1": case "binary": case "ascii": return bytesToLatin1(bytes);
      case "utf16le": case "ucs2": case "ucs-2": case "utf-16le": return fromUtf16(bytes, true);
      default: return utf8d.decode(bytes);
    }
  };
  class Buffer extends Uint8Array {
    static from(v, enc) {
      const b = typeof v === "string" ? encode(v, enc) : asBytes(v);
      const out = new Buffer(b.length);
      out.set(b);
      return out;
    }
    static alloc(n, fill = 0) { const b = new Buffer(n); if (typeof fill === "number") b.fill(fill); else if (fill != null) { const f = encode(fill); for (let i = 0; i < n; i++) b[i] = f[i % f.length]; } return b; }
    static isBuffer(v) { return v instanceof Buffer; }
    static concat(list) { const n = list.reduce((a, b) => a + b.length, 0); const out = new Buffer(n); let o = 0; for (const b of list) { out.set(b, o); o += b.length; } return out; }
    static byteLength(s, enc) { return encode(s, enc).length; }
    toString(enc = "utf8", start = 0, end = this.length) { return decode(this.subarray(start, end), enc); }
    equals(o) { if (o.length !== this.length) return false; for (let i = 0; i < o.length; i++) if (o[i] !== this[i]) return false; return true; }
    toJSON() { return { type: "Buffer", data: Array.from(this) }; }
  }
  Buffer.poolSize = 8192;
  globalThis.Buffer = Buffer;

  // ── fetch with the original response shape (text()/json() are synchronous) ─
  const webFetch = globalThis.fetch;
  const canonical = (k) => k.split("-").map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join("-");
  class FormData {
    constructor() { this._fields = []; }
    append(k, v) { this._fields.push([String(k), String(v)]); }
    set(k, v) { this._fields = this._fields.filter(([x]) => x !== String(k)); this.append(k, v); }
    get(k) { const e = this._fields.find(([x]) => x === String(k)); return e ? e[1] : null; }
    has(k) { return this._fields.some(([x]) => x === String(k)); }
    delete(k) { this._fields = this._fields.filter(([x]) => x !== String(k)); }
  }
  globalThis.FormData = FormData;

  globalThis.fetch = async (url, options = {}) => {
    const headers = { ...(options.headers || {}) };
    const hasType = () => Object.keys(headers).some((k) => k.toLowerCase() === "content-type");
    let body = options.body;
    if (body instanceof FormData) {
      const boundary = "----banime" + Math.random().toString(16).slice(2);
      body = body._fields.map(([k, v]) => `--${boundary}\r\nContent-Disposition: form-data; name="${k}"\r\n\r\n${v}\r\n`).join("") + `--${boundary}--\r\n`;
      headers["Content-Type"] = `multipart/form-data; boundary=${boundary}`;
    } else if (body instanceof Uint8Array || body instanceof ArrayBuffer) {
      body = decode(asBytes(body), "latin1");
    } else if (body != null && typeof body === "object" && !(body instanceof URLSearchParams)) {
      body = JSON.stringify(body);
      if (!hasType()) headers["Content-Type"] = "application/json";
    }
    const r = await webFetch(String(url), { method: (options.method || "GET").toUpperCase(), headers, body, redirect: options.redirect });
    const text = await r.text();
    const plainHeaders = {};
    const rawHeaders = {};
    for (const [k, v] of r.headers.entries()) { plainHeaders[canonical(k)] = v; rawHeaders[canonical(k)] = [v]; }
    Object.defineProperty(plainHeaders, "get", { value: (name) => r.headers.get(name), enumerable: false });
    const cookies = {};
    for (const c of r.headers.getSetCookie()) {
      const pair = c.split(";")[0];
      const i = pair.indexOf("=");
      if (i > 0) cookies[pair.slice(0, i).trim()] = pair.slice(i + 1).trim();
    }
    let parsed;
    return {
      status: r.status,
      statusText: `${r.status} ${r.statusText}`.trim(),
      method: (options.method || "GET").toUpperCase(),
      rawHeaders,
      ok: r.ok,
      url: r.url,
      headers: plainHeaders,
      cookies,
      redirected: r.redirected,
      contentType: r.headers.get("content-type") || "",
      contentLength: text.length,
      body: text,
      text: () => text,
      json: () => {
        if (parsed === undefined) { try { parsed = JSON.parse(text); } catch { parsed = null; } }
        return parsed;
      },
    };
  };

  // ── LoadDoc / Doc: goquery-style selections ────────────────────────────────
  class DocSelection {
    constructor(doc, ids, prev) { this._doc = doc; this._ids = ids; this._prev = prev; }
    _op(op, arg = "", ids = this._ids) { return unwrap(host.html(this._doc, op, JSON.stringify(ids), String(arg))); }
    _wrap(ids) { return new DocSelection(this._doc, ids, this); }
    _nav(op, selector) { const ids = this._op(op); return this._wrap(selector ? this._op("filter", selector, ids) : ids); }
    _one(i) { return new DocSelection(this._doc, [this._ids[i]], this); }
    _matches(selector, i) { return typeof selector === "function" ? !!selector(i, this._one(i)) : this._op("filter", selector, [this._ids[i]]).length > 0; }
    _walk(step, selector, until) {
      // Repeats `step` from each node; `until` stops (exclusive), `selector` filters.
      const out = [];
      for (const id of this._ids) {
        let cur = [id];
        for (;;) {
          cur = this._op(step, "", cur);
          if (!cur.length) break;
          if (until && this._op("filter", until, cur).length) break;
          if (!out.includes(cur[0])) out.push(cur[0]);
        }
      }
      return this._wrap(selector ? this._op("filter", selector, out) : out);
    }

    length() { return this._ids.length; }
    find(selector) { return this._wrap(this._op("find", selector)); }
    children(selector) { return this._nav("children", selector); }
    contents() { return this._nav("contents"); }
    contentsFiltered(selector) { return this._nav("contents", selector); }
    parent(selector) { return this._nav("parent", selector); }
    parents(selector) { return this._walk("parent", selector); }
    parentsUntil(selector, filter) { return this._walk("parent", filter, selector); }
    closest(selector) {
      const out = [];
      for (const id of this._ids) {
        let cur = [id];
        while (cur.length) {
          if (this._op("filter", selector, cur).length) { if (!out.includes(cur[0])) out.push(cur[0]); break; }
          cur = this._op("parent", "", cur);
        }
      }
      return this._wrap(out);
    }
    next(selector) { return this._nav("next", selector); }
    prev(selector) { return this._nav("prev", selector); }
    nextAll(selector) { return this._walk("next", selector); }
    prevAll(selector) { return this._walk("prev", selector); }
    nextUntil(selector, filter) { return this._walk("next", filter, selector); }
    prevUntil(selector, filter) { return this._walk("prev", filter, selector); }
    siblings(selector) {
      const out = [];
      for (const id of this._ids) {
        for (const c of this._op("children", "", this._op("parent", "", [id]))) if (c !== id && !out.includes(c)) out.push(c);
      }
      return this._wrap(selector ? this._op("filter", selector, out) : out);
    }
    first() { return this._wrap(this._ids.slice(0, 1)); }
    last() { return this._wrap(this._ids.slice(-1)); }
    eq(i) { const n = i < 0 ? this._ids.length + i : i; return this._wrap(n >= 0 && n < this._ids.length ? [this._ids[n]] : []); }
    filter(selector) { return this._wrap(this._ids.filter((_, i) => this._matches(selector, i))); }
    not(selector) { return this._wrap(this._ids.filter((_, i) => !this._matches(selector, i))); }
    is(selector) { return this._ids.some((_, i) => this._matches(selector, i)); }
    has(selector) { return this._wrap(this._ids.filter((id) => this._op("find", selector, [id]).length > 0)); }
    end() { return this._prev || this; }
    each(fn) { this._ids.forEach((_, i) => fn(i, this._one(i))); return this; }
    map(fn) { return this._ids.map((_, i) => fn(i, this._one(i))); }
    text() { return this._op("text"); }
    html() { return this._op("html"); }
    attr(name) { const a = this._op("attrs"); return a && name in a ? a[name] : undefined; }
    attrs() { return this._op("attrs") || {}; }
    data(name) {
      const a = this._op("attrs");
      if (!a) return undefined;
      if (name === undefined) { const d = {}; for (const k of Object.keys(a)) if (k.startsWith("data-")) d[k] = a[k]; return d; }
      return a[`data-${name}`];
    }
  }
  globalThis.LoadDoc = (html) => {
    const doc = unwrap(host.htmlParse(String(html)));
    const root = new DocSelection(doc, [0]);
    return (selector) => (selector instanceof DocSelection ? selector : root.find(String(selector)));
  };
  globalThis.Doc = function Doc(html) {
    const doc = unwrap(host.htmlParse(String(html)));
    return new DocSelection(doc, [0]);
  };

  // ── CryptoJS (AES-CBC and the encoders) ────────────────────────────────────
  const crypto = (op, a, b = "", c = "") => fromB64(unwrap(host.crypto(op, toB64(a), b ? toB64(b) : "", c ? toB64(c) : "")));
  class WordArray {
    constructor(bytes, iv) { this.__bytes = bytes; if (iv) this.iv = iv; }
    toString(encoder) { return encoder && encoder.stringify ? encoder.stringify(this) : toB64(this.__bytes); }
  }
  const encoder = (parse, stringify) => ({ parse: (s) => new WordArray(parse(String(s))), stringify: (w) => stringify(asBytes(w)) });
  const enc = {
    Utf8: encoder((s) => utf8.encode(s), (b) => utf8d.decode(b)),
    Base64: encoder(fromB64, toB64),
    Hex: encoder(fromHex, toHex),
    Latin1: encoder(latin1ToBytes, bytesToLatin1),
    Utf16: encoder((s) => utf16(s, false), (b) => fromUtf16(b, false)),
    Utf16LE: encoder((s) => utf16(s, true), (b) => fromUtf16(b, true)),
  };
  // Keys that aren't 16/24/32 bytes are hashed with SHA-256.
  const adjustKey = (k) => ([16, 24, 32].includes(k.length) ? k : crypto("sha256", k));
  // OpenSSL's EVP_BytesToKey with MD5, as CryptoJS uses for passphrases.
  const evp = (password, salt) => {
    let d = new Uint8Array(0), last = new Uint8Array(0);
    while (d.length < 48) {
      const input = new Uint8Array(last.length + password.length + salt.length);
      input.set(last); input.set(password, last.length); input.set(salt, last.length + password.length);
      last = crypto("md5", input);
      const next = new Uint8Array(d.length + last.length); next.set(d); next.set(last, d.length); d = next;
    }
    return { key: d.slice(0, 32), iv: d.slice(32, 48) };
  };
  const randomBytes = (n) => Uint8Array.from({ length: n }, () => Math.floor(Math.random() * 256));
  globalThis.CryptoJS = {
    enc,
    AES: {
      encrypt(message, key, cfg) {
        const k = adjustKey(asBytes(key));
        const given = cfg && cfg.iv ? asBytes(cfg.iv) : null;
        const iv = given || randomBytes(16);
        const out = crypto("aes-cbc-encrypt", utf8.encode(String(message)), k, iv);
        if (given) return new WordArray(out, iv);
        const withIv = new Uint8Array(16 + out.length); withIv.set(iv); withIv.set(out, 16);
        return new WordArray(withIv, iv);
      },
      decrypt(message, key, cfg) {
        const password = asBytes(key);
        const data = fromB64(message instanceof WordArray ? message.toString() : String(message));
        let k, iv, body;
        if (cfg && cfg.iv) { k = adjustKey(password); iv = asBytes(cfg.iv); body = data; }
        else if (data.length >= 16 && bytesToLatin1(data.subarray(0, 8)) === "Salted__") { ({ key: k, iv } = evp(password, data.subarray(8, 16))); body = data.subarray(16); }
        else { k = adjustKey(password); iv = data.subarray(0, 16); body = data.subarray(16); }
        return new WordArray(crypto("aes-cbc-decrypt", body, k, iv), iv);
      },
    },
  };

  // ── $-helpers ─────────────────────────────────────────────────────────────
  globalThis.__isOffline__ = false;
  globalThis.$isOffline = () => false;
  globalThis.$toString = (v) => {
    if (typeof v === "string") return v;
    if (v instanceof Uint8Array || v instanceof ArrayBuffer) return utf8d.decode(asBytes(v));
    if (v == null) return "";
    if (typeof v === "object") { try { return JSON.stringify(v); } catch { return String(v); } }
    return String(v);
  };
  globalThis.$toBytes = (v) => asBytes(v);
  globalThis.$clone = (v) => (v === undefined ? v : JSON.parse(JSON.stringify(v)));
  globalThis.$replace = (target, value) => {
    if (Array.isArray(target) && Array.isArray(value)) { target.length = 0; target.push(...value); return; }
    if (target && typeof target === "object") { for (const k of Object.keys(target)) delete target[k]; Object.assign(target, value); }
  };
  globalThis.$unmarshalJSON = (data, dst) => $replace(dst, typeof data === "string" ? JSON.parse(data) : $clone(data));
  globalThis.$arrayOf = () => [];
  globalThis.$sleep = () => console.warn("$sleep isn't supported in B-Anime; use await new Promise(r => setTimeout(r, ms))");
  globalThis.$await = () => { throw new Error("$await isn't supported in B-Anime"); };

  const fields = (__extConfig && __extConfig.fields) || [];
  globalThis.$getUserPreference = (key) => {
    const f = fields.find((x) => x.name === key);
    return f && f.default !== undefined && f.default !== "" ? String(f.default) : undefined;
  };

  // Lives for one call: B-Anime starts every call in a fresh sandbox.
  const store = new Map();
  globalThis.$store = {
    set: (k, v) => { store.set(k, v); },
    get: (k) => store.get(k),
    getUnsafe: (k) => store.get(k),
    has: (k) => store.has(k),
    delete: (k) => store.delete(k),
    remove: (k) => store.delete(k),
    getOrSet: (k, fn) => { if (!store.has(k)) store.set(k, fn()); return store.get(k); },
    setIfLessThanLimit: (k, v, max) => { if (store.size >= max && !store.has(k)) return false; store.set(k, v); return true; },
    keys: () => [...store.keys()],
    values: () => [...store.values()],
  };

  const unsupported = (name) => { throw new Error(`${name} isn't available in B-Anime`); };
  globalThis.ChromeDP = new Proxy({}, { get: (_, k) => () => unsupported(`ChromeDP.${String(k)}`) });
  globalThis.$habari = { parse: () => unsupported("$habari.parse") };
  globalThis.AbortController = class AbortController {
    constructor() { this.signal = { aborted: false, reason: undefined, addEventListener() {}, removeEventListener() {} }; }
    abort(reason) { this.signal.aborted = true; this.signal.reason = reason; }
  };
  if (typeof globalThis.structuredClone !== "function") globalThis.structuredClone = (v) => $clone(v);
})();
