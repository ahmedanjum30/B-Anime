// Same steps as server/example.html, 1-11:
//   1-5   site: search -> watch page -> episodes -> server buttons -> player link
//   6-7   player page -> getSources (subtitles, skip times, "enc")
//   8     AES-256-CBC-decrypt "enc" -> { file: ".../master.m3u8" }
//   9     sign the link with an HMAC-SHA256 CDN token (~90 s life)
//   10    return the .m3u8 (type "m3u8") with the Referer the CDN wants
//   11    HD-1 (?s=tcdn): each piece is a 1x1 PNG + the video bytes on an image CDN (see note in findEpisodeServer)
// sources() resolves every server (sub and dub) this way; a server that fails is left out.

// ===== Domains =====
const SITE = "https://anikototv.bz";        // search, watch pages, /ajax/ calls
const MEGAPLAY = "https://megaplay.buzz";   // player for HD-1, Vidstream-2, Vidstream-1 (/videojs/)
const VIDTUBE = "https://vidtube.site";     // player for VidPlay
// Player hosts whose getSources we read. The server list decides which one a button uses.
const PLAYER_HOSTS = [MEGAPLAY, VIDTUBE];
// ===================

// ===== Stream headers =====
// The m3u8 CDN (Cloudflare) answers 403 "you have been blocked" without this Referer. The segments don't need it.
const REFERER = `${MEGAPLAY}/`;
// ==========================

// ===== Player script constants (e1-player.min.js) =====
// Step 8: key padded with zero bytes to 32 (AES-256), IV padded to 16.
const SOURCE_ENC_KEY = "i?LMTAx0Q6,:}50U";
const SOURCE_ENC_IV = "W0;27ToaUpl_P%'c";
// Step 9: token = b64url("{expiry}|{show id}/{episode id}") + "." + b64url(HMAC-SHA256(that, secret))
const CDN_TOKEN_SECRET = "MpCdnT0k3n!9f2K#xQ7vL5mR8wN1pY4s";
const CDN_TOKEN_TTL_SEC = 90;
// ======================================================

// ===== Crypto in plain JS =====
// B-Anime runs extensions in QuickJS, which has no crypto.subtle, so AES and HMAC are done here.
// Works the same in browsers and Node.

// AES-CBC decrypt (128/192/256-bit key by its length), PKCS#7 padding removed.
const aesCbcDecrypt = (() => {
  const SBOX = new Uint8Array(256);
  const INV = new Uint8Array(256);
  const xt = (a) => ((a << 1) ^ (a & 0x80 ? 0x1b : 0)) & 0xff;
  const mul = (a, b) => {
    let r = 0;
    for (; b; b >>= 1, a = xt(a)) if (b & 1) r ^= a;
    return r;
  };
  for (let i = 0; i < 256; i++) {
    // inverse in GF(2^8) (0 maps to 0), then the affine transform
    let inv = 0;
    if (i) for (let j = 1; j < 256; j++) if (mul(i, j) === 1) { inv = j; break; }
    let s = inv;
    for (let k = 1; k <= 4; k++) s ^= ((inv << k) | (inv >> (8 - k))) & 0xff;
    SBOX[i] = s ^ 0x63;
    INV[s ^ 0x63] = i;
  }

  function expandKey(key) {
    const nk = key.length / 4;
    const rounds = nk + 6;
    const w = new Uint8Array(16 * (rounds + 1));
    w.set(key);
    let rcon = 1;
    for (let i = key.length; i < w.length; i += 4) {
      let t = [w[i - 4], w[i - 3], w[i - 2], w[i - 1]];
      const word = i / 4;
      if (word % nk === 0) {
        t = [SBOX[t[1]] ^ rcon, SBOX[t[2]], SBOX[t[3]], SBOX[t[0]]];
        rcon = xt(rcon);
      } else if (nk > 6 && word % nk === 4) {
        t = t.map((b) => SBOX[b]);
      }
      for (let j = 0; j < 4; j++) w[i + j] = w[i - 4 * nk + j] ^ t[j];
    }
    return { w, rounds };
  }

  function decryptBlock(block, { w, rounds }) {
    const s = Uint8Array.from(block);
    const addKey = (r) => { for (let i = 0; i < 16; i++) s[i] ^= w[r * 16 + i]; };
    const invShiftSub = () => {
      const t = Uint8Array.from(s);
      for (let c = 0; c < 4; c++) for (let r = 0; r < 4; r++) s[r + 4 * c] = INV[t[r + 4 * ((c - r + 4) % 4)]];
    };
    addKey(rounds);
    for (let round = rounds - 1; round >= 0; round--) {
      invShiftSub();
      addKey(round);
      if (round === 0) break;
      for (let c = 0; c < 16; c += 4) {
        const [a0, a1, a2, a3] = [s[c], s[c + 1], s[c + 2], s[c + 3]];
        s[c] = mul(a0, 14) ^ mul(a1, 11) ^ mul(a2, 13) ^ mul(a3, 9);
        s[c + 1] = mul(a0, 9) ^ mul(a1, 14) ^ mul(a2, 11) ^ mul(a3, 13);
        s[c + 2] = mul(a0, 13) ^ mul(a1, 9) ^ mul(a2, 14) ^ mul(a3, 11);
        s[c + 3] = mul(a0, 11) ^ mul(a1, 13) ^ mul(a2, 9) ^ mul(a3, 14);
      }
    }
    return s;
  }

  return (data, key, iv) => {
    if (data.length % 16 || !data.length) throw new Error("AES: data is not whole blocks");
    const k = expandKey(key);
    const out = new Uint8Array(data.length);
    let prev = iv;
    for (let i = 0; i < data.length; i += 16) {
      const block = data.subarray(i, i + 16);
      const plain = decryptBlock(block, k);
      for (let j = 0; j < 16; j++) out[i + j] = plain[j] ^ prev[j];
      prev = block;
    }
    const pad = out[out.length - 1];
    if (pad < 1 || pad > 16) throw new Error("AES: bad padding (wrong key or IV)");
    return out.subarray(0, out.length - pad);
  };
})();

function sha256(data) {
  const K = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
  ];
  const H = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
  const len = data.length;
  const total = Math.ceil((len + 9) / 64) * 64;
  const m = new Uint8Array(total);
  m.set(data);
  m[len] = 0x80;
  const dv = new DataView(m.buffer);
  dv.setUint32(total - 8, Math.floor(len / 0x20000000));
  dv.setUint32(total - 4, (len * 8) >>> 0);
  const W = new Uint32Array(64);
  const rotr = (x, n) => (x >>> n) | (x << (32 - n));
  for (let off = 0; off < total; off += 64) {
    for (let i = 0; i < 16; i++) W[i] = dv.getUint32(off + i * 4);
    for (let i = 16; i < 64; i++) {
      const s0 = rotr(W[i - 15], 7) ^ rotr(W[i - 15], 18) ^ (W[i - 15] >>> 3);
      const s1 = rotr(W[i - 2], 17) ^ rotr(W[i - 2], 19) ^ (W[i - 2] >>> 10);
      W[i] = (W[i - 16] + s0 + W[i - 7] + s1) >>> 0;
    }
    let [a, b, c, d, e, f, g, h] = H;
    for (let i = 0; i < 64; i++) {
      const t1 = (h + (rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)) + ((e & f) ^ (~e & g)) + K[i] + W[i]) >>> 0;
      const t2 = ((rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)) + ((a & b) ^ (a & c) ^ (b & c))) >>> 0;
      [h, g, f, e, d, c, b, a] = [g, f, e, (d + t1) >>> 0, c, b, a, (t1 + t2) >>> 0];
    }
    [a, b, c, d, e, f, g, h].forEach((v, i) => { H[i] = (H[i] + v) >>> 0; });
  }
  const out = new Uint8Array(32);
  H.forEach((v, i) => new DataView(out.buffer).setUint32(i * 4, v));
  return out;
}

function hmacSha256(key, message) {
  let k = key.length > 64 ? sha256(key) : key;
  const block = new Uint8Array(64);
  block.set(k);
  const inner = new Uint8Array(64 + message.length);
  const outer = new Uint8Array(64 + 32);
  for (let i = 0; i < 64; i++) { inner[i] = block[i] ^ 0x36; outer[i] = block[i] ^ 0x5c; }
  inner.set(message, 64);
  outer.set(sha256(inner), 64);
  return sha256(outer);
}
// ==============================

class Provider {
  constructor() {
    this.base = SITE;
    this.ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:155.0) Gecko/20100101 Firefox/155.0";
    this.lang = "en-US,en;q=0.9";
    this.defaultServer = "HD-1";
  }

  getSettings() {
    return {
      episodeServers: ["HD-1", "Vidstream-2", "Vidstream-1", "VidPlay"],
      supportsDub: true,
    };
  }

  async request(url, { referer = `${this.base}/`, ajax = false } = {}) {
    const res = await fetch(url, {
      headers: {
        "User-Agent": this.ua,
        "Accept-Language": this.lang,
        Accept: ajax ? "application/json, text/javascript, */*; q=0.01" : "text/html,application/xhtml+xml,*/*;q=0.8",
        Referer: referer,
        // The site's own script sends this on every /ajax/ call; without it: {"status":500,"result":"Bad request"}.
        ...(ajax && { "X-Requested-With": "XMLHttpRequest" }),
      },
    });
    const text = await res.text();
    if (!res.ok) throw new Error(`${url} -> HTTP ${res.status}: ${text.slice(0, 200)}`);
    return text;
  }

  async ajax(url, referer) {
    const text = await this.request(url, { referer, ajax: true });
    let json;
    try { json = JSON.parse(text); } catch { throw new Error(`${url} -> not JSON: ${text.slice(0, 200)}`); }
    if (json.status && json.status !== 200) throw new Error(`${url} -> ${json.status}: ${JSON.stringify(json.result).slice(0, 200)}`);
    return json;
  }

  // result is either an HTML string or { html }
  html(json) {
    return typeof json.result === "string" ? json.result : json.result?.html || "";
  }

  decode(s) {
    return String(s ?? "")
      .replace(/<[^>]+>/g, "")
      .replace(/&#(\d+);/g, (_, n) => String.fromCharCode(Number(n)))
      .replace(/&#x([0-9a-f]+);/gi, (_, n) => String.fromCharCode(parseInt(n, 16)))
      .replace(/&quot;/g, '"').replace(/&#039;|&apos;/g, "'").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&")
      .trim();
  }

  // ----- Crypto helpers (the plain-JS functions above; no crypto.subtle needed) -----

  padBytes(text, size) {
    const bytes = new TextEncoder().encode(String(text));
    const out = new Uint8Array(size); // zero-filled
    out.set(bytes.subarray(0, Math.min(size, bytes.length)));
    return out;
  }

  b64UrlDecode(s) {
    let t = String(s).replace(/-/g, "+").replace(/_/g, "/");
    if (t.length % 4) t += "====".slice(t.length % 4);
    return Uint8Array.from(atob(t), (c) => c.charCodeAt(0));
  }

  b64UrlEncode(buf) {
    const bytes = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
    let s = "";
    for (const b of bytes) s += String.fromCharCode(b);
    return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  }

  // Step 8: "enc" -> { file: ".../master.m3u8", ... }
  async decryptSources(enc) {
    const plain = aesCbcDecrypt(this.b64UrlDecode(enc), this.padBytes(SOURCE_ENC_KEY, 32), this.padBytes(SOURCE_ENC_IV, 16));
    const data = JSON.parse(new TextDecoder().decode(plain));
    if (!data || typeof data.file !== "string") throw new Error("Invalid enc payload");
    return data;
  }

  // Step 9: add ?token= signed over the two 32-character ids in the path. Same order as the player:
  // first t={sub|dub}, then the token (skipped if the link already has one or has no ids).
  async signStreamUrl(url, lang) {
    let out = url;
    if (/\/master\.m3u8(?:\?|$)/i.test(out) && !/[?&]t=/.test(out)) out += `${out.includes("?") ? "&" : "?"}t=${encodeURIComponent(lang)}`;
    if (/[?&]token=/.test(out)) return out;
    const ids = out.match(/\/([a-f0-9]{32})\/([a-f0-9]{32})\//i);
    if (!ids) return out;
    const message = `${Math.floor(Date.now() / 1000) + CDN_TOKEN_TTL_SEC}|${ids[1].toLowerCase()}/${ids[2].toLowerCase()}`;
    const enc = new TextEncoder();
    const sig = hmacSha256(enc.encode(CDN_TOKEN_SECRET), enc.encode(message));
    const token = `${this.b64UrlEncode(enc.encode(message))}.${this.b64UrlEncode(sig)}`;
    return `${out}${out.includes("?") ? "&" : "?"}token=${encodeURIComponent(token)}`;
  }

  // Step 1: search. Result id = "{slug}/{sub|dub}"
  async search(query) {
    const q = typeof query === "string" ? query : query.query;
    const lang = query && query.dub ? "dub" : "sub";
    const json = await this.ajax(`${this.base}/ajax/anime/search?${new URLSearchParams({ keyword: q })}`);
    const results = [];
    for (const m of this.html(json).matchAll(/<a\b[^>]*class="item"[^>]*href="([^"]*\/watch\/([^"/?#]+))"[^>]*>([\s\S]*?)<\/a>/gi)) {
      const title = this.decode(m[3].match(/class="name[^"]*"[^>]*>([\s\S]*?)<\/div>/i)?.[1]);
      results.push({
        id: `${m[2]}/${lang}`,
        title: title || m[2],
        url: `${this.base}/watch/${m[2]}`,
        subOrDub: lang,
      });
    }
    if (!results.length) throw new Error("No anime found");
    return results;
  }

  // Step 2: watch page -> anime id. Step 3: episode list. Episode id = "{sub|dub}:{data-ids}"
  async findEpisodes(id) {
    const [slug, lang = "sub"] = String(id).split("/");
    const watch = `${this.base}/watch/${slug}`;
    const page = await this.request(watch);
    const animeId = page.match(/id="watch-main"[^>]*data-id="([^"]+)"/)?.[1] || page.match(/data-id="([^"]+)"[^>]*id="watch-main"/)?.[1];
    if (!animeId) throw new Error(`No anime id on the watch page for "${slug}"`);

    const list = this.html(await this.ajax(`${this.base}/ajax/episode/list/${encodeURIComponent(animeId)}`, watch));
    const episodes = [];
    for (const m of list.matchAll(/<li\b([^>]*)>\s*<a\b([^>]*)>/gi)) {
      const a = m[2];
      const attr = (name) => a.match(new RegExp(`\\b${name}="([^"]*)"`))?.[1];
      const ids = attr("data-ids");
      const number = Number.parseFloat(attr("data-num"));
      if (!ids || !Number.isFinite(number)) continue;
      if (attr(`data-${lang}`) === "0") continue; // not available in this language
      episodes.push({
        id: `${lang}:${ids}`,
        title: this.decode(m[1].match(/\btitle="([^"]*)"/)?.[1]) || `Episode ${number}`,
        number,
        url: `${watch}/ep-${attr("data-slug") || number}`,
      });
    }
    episodes.sort((a, b) => a.number - b.number);
    return episodes;
  }

  // Episode id "{sub|dub}:{data-ids}" -> { lang, ids }
  parseEpisodeId(episode) {
    const raw = typeof episode === "string" ? episode : episode.id;
    const colon = raw.indexOf(":");
    return { lang: colon > 0 ? raw.slice(0, colon) : "sub", ids: colon > 0 ? raw.slice(colon + 1) : raw };
  }

  // Step 4: every button in every language block -> [{ lang, name, link }], in the site's order (HD-1 first).
  // The count isn't fixed; it's whatever <li> the server returned for this episode (4 sub + 4 dub for Naruto).
  async listServers(ids) {
    const html = this.html(await this.ajax(`${this.base}/ajax/server/list?servers=${encodeURIComponent(ids)}`));
    const servers = [];
    for (const block of html.matchAll(/data-type="([a-z]+)"([\s\S]*?)(?=data-type="|$)/gi)) {
      for (const b of block[2].matchAll(/<li\b[^>]*data-link-id="([^"]+)"[^>]*>([\s\S]*?)<\/li>/gi)) {
        servers.push({ lang: block[1], name: this.decode(b[2]), link: b[1] });
      }
    }
    return servers;
  }

  // Steps 5-9 for one server button -> { stream, subtitles, skip }. Throws if there's no playable .m3u8.
  async resolveServer(button) {
    // Step 5: the button's player link (+ skip times the site knows)
    const res = await this.ajax(`${this.base}/ajax/server?get=${encodeURIComponent(button.link)}`);
    const player = res.result?.url;
    if (!player) throw new Error(`Server ${button.name} returned no player link`);
    const p = new URL(player);
    if (!PLAYER_HOSTS.includes(p.origin)) throw new Error(`Unknown player host ${p.origin}`);

    // Step 6: player page -> file id
    const page = await this.request(player, { referer: `${this.base}/` });
    const dataId = page.match(/data-id="([^"]+)"/)?.[1];
    if (!dataId) throw new Error(`No file id on the ${button.name} player page`);

    // Step 7: getSources (HD-1 links carry ?s=tcdn; it's passed on). vidtube has no /videojs/ path.
    const extra = p.searchParams.get("s") ? `&s=${encodeURIComponent(p.searchParams.get("s"))}` : "";
    const query = `getSources?id=${encodeURIComponent(dataId)}${extra}`;
    let src;
    try { src = await this.ajax(`${p.origin}/videojs/stream/${query}`, player); }
    catch { src = await this.ajax(`${p.origin}/stream/${query}`, player); }

    // Steps 8-9: decrypt "enc" (older players send plain "sources") and sign the link.
    const file = src.enc ? (await this.decryptSources(src.enc)).file : src.sources?.[0]?.file || src.sources?.file;
    if (!file) throw new Error(`Server ${button.name} returned no video link`);
    const stream = await this.signStreamUrl(file, button.lang);

    const subtitles = (src.tracks || [])
      .filter((t) => t.file && t.kind !== "thumbnails")
      .map((t) => ({ url: t.file, lang: t.label || "Unknown", label: t.label || "Unknown", isDefault: !!t.default }));
    const skip = src.intro || src.outro
      ? { intro: [src.intro?.start ?? 0, src.intro?.end ?? 0], outro: [src.outro?.start ?? 0, src.outro?.end ?? 0] }
      : res.result?.skip_data || null;
    return { stream, subtitles, skip };
  }

  // One server by name, in the episode's language (falls back to the first button).
  // Step 10: the .m3u8 + Referer. Step 11 (HD-1 / ?s=tcdn): each piece is a 70-byte PNG, a cut-short
  // 182-byte packet, then the video; B-Anime's proxy drops the first part, other players must do the same.
  async findEpisodeServer(episode, server) {
    const { lang, ids } = this.parseEpisodeId(episode);
    const name = (server || this.defaultServer).toLowerCase();
    const buttons = (await this.listServers(ids)).filter((s) => s.lang === lang);
    const button = buttons.find((b) => b.name.toLowerCase() === name) || buttons[0];
    if (!button) throw new Error(`No ${lang} servers for this episode`);
    const { stream, subtitles, skip } = await this.resolveServer(button);
    return {
      server: button.name,
      headers: { Referer: REFERER },
      videoSources: [{ url: stream, quality: "auto", type: "m3u8", subtitles, skip }],
    };
  }

  // Names your tool looks for. They only call the methods above.
  async episodes(id) {
    return this.findEpisodes(id);
  }

  // What B-Anime calls: every server from both language blocks, each resolved to an .m3u8 (all at once,
  // in parallel). A server that fails is left out; the app can't play the player page anyway.
  // → { headers, sources: [{ url, type: "m3u8", label, audio: "sub"|"dub" }], subtitles: [{ url, lang, label }] }
  async sources(episodeId) {
    const { lang: wantLang, ids } = this.parseEpisodeId(episodeId);
    const servers = await this.listServers(ids);
    if (!servers.length) throw new Error("No servers for this episode");

    const resolved = await Promise.all(servers.map((s) => this.resolveServer(s).then((r) => ({ ...s, ...r }), () => null)));
    const ok = resolved.filter(Boolean);
    if (!ok.length) throw new Error("No server returned a playable video");
    // The episode's own language first; each block keeps the site's order (HD-1 first).
    ok.sort((a, b) => (a.lang === wantLang ? 0 : 1) - (b.lang === wantLang ? 0 : 1));

    const headers = { Referer: REFERER };
    const subtitles = [];
    for (const s of ok) for (const t of s.subtitles) if (!subtitles.some((x) => x.url === t.url)) subtitles.push(t);
    return {
      headers,
      sources: ok.map((s) => ({
        url: s.stream,
        type: "m3u8",
        quality: "auto",
        label: `${s.name} (${s.lang})`,
        audio: s.lang,
        headers,
        skip: s.skip,
      })),
      subtitles,
    };
  }
}
