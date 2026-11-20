// Anime4i provider. Calls anime4i.com and Dailymotion directly. Same steps as server/example.html, 1-5.
class Provider {
  constructor() {
    this.base = "https://anime4i.com";            // the site: search, show pages, episode pages
    this.dmBase = "https://www.dailymotion.com";  // Dailymotion video details (metadata)
    this.dmPlayer = "https://geo.dailymotion.com"; // where the embedded player lives (referer/origin for its calls)
    // Sent with every request.
    this.ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:155.0) Gecko/20100101 Firefox/155.0";
    this.lang = "en-US,en;q=0.9";
  }

  getSettings() {
    return {
      episodeServers: ["Dailymotion"],
      supportsDub: false, // every episode is "English Subtitles"
    };
  }

  async request(url, { referer = `${this.base}/`, origin = this.base, accept = "*/*" } = {}) {
    const res = await fetch(url, {
      headers: {
        "User-Agent": this.ua,
        "Accept-Language": this.lang,
        Accept: accept,
        Referer: referer,
        Origin: origin,
      },
    });
    const text = await res.text();
    if (!res.ok) throw new Error(`${url} -> HTTP ${res.status}: ${text.slice(0, 200)}`);
    return text;
  }

  async getHTML(url, opts = {}) {
    return this.request(url, { ...opts, accept: "text/html,application/xhtml+xml,*/*;q=0.8" });
  }

  async getJSON(url, opts = {}) {
    const text = await this.request(url, { ...opts, accept: "application/json" });
    try { return JSON.parse(text); } catch { throw new Error(`${url} -> not JSON: ${text.slice(0, 200)}`); }
  }

  decode(s) {
    return String(s ?? "")
      .replace(/&#(\d+);/g, (_, n) => String.fromCharCode(Number(n)))
      .replace(/&#x([0-9a-f]+);/gi, (_, n) => String.fromCharCode(parseInt(n, 16)))
      .replace(/&quot;/g, '"').replace(/&#039;|&apos;/g, "'").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&")
      .trim();
  }

  // Last path segment of a link: https://anime4i.com/anime/swallowed-star/ -> swallowed-star
  slugOf(url) {
    return String(url).replace(/[?#].*$/, "").replace(/\/+$/, "").split("/").pop();
  }

  // Step 1: search. WordPress JSON search; only "anime" results (show pages). Result id = show slug.
  async search(query) {
    const q = typeof query === "string" ? query : query.query;
    const json = await this.getJSON(`${this.base}/wp-json/wp/v2/search?${new URLSearchParams({ search: q, per_page: "20" })}`);
    const results = [];
    for (const r of Array.isArray(json) ? json : []) {
      if (r.subtype !== "anime" && !/\/anime\//.test(r.url || "")) continue;
      const slug = this.slugOf(r.url);
      if (!slug) continue;
      results.push({
        id: slug,
        title: this.decode(r.title),
        url: `${this.base}/anime/${slug}`,
        subOrDub: "sub",
      });
    }
    if (!results.length) throw new Error("No anime found");
    return results;
  }

  // Step 2: show page -> episode list (<div class="eplister">). Episode id = episode page slug.
  async findEpisodes(id) {
    const html = await this.getHTML(`${this.base}/anime/${encodeURIComponent(id)}/`);
    const start = html.indexOf('class="eplister"');
    if (start < 0) throw new Error(`No episode list on the show page for "${id}"`);
    const end = html.indexOf("</ul>", start);
    const list = html.slice(start, end < 0 ? undefined : end);

    const episodes = [];
    const seen = new Set();
    const re = /<a\b[^>]*\bhref="([^"]+)"[^>]*>([\s\S]*?)<\/a>/gi;
    let m;
    while ((m = re.exec(list)) !== null) {
      const slug = this.slugOf(m[1]);
      if (!slug || seen.has(slug)) continue;
      const numText = m[2].match(/<div[^>]*class="epl-num"[^>]*>([\s\S]*?)<\/div>/i)?.[1] ?? "";
      const titleText = m[2].match(/<div[^>]*class="epl-title"[^>]*>([\s\S]*?)<\/div>/i)?.[1] ?? "";
      const number = Number.parseFloat(this.decode(numText.replace(/<[^>]+>/g, "")));
      if (!Number.isFinite(number)) continue;
      seen.add(slug);
      episodes.push({
        id: slug,
        title: this.decode(titleText.replace(/<[^>]+>/g, "")) || `Episode ${number}`,
        number,
        url: `${this.base}/${slug}`,
      });
    }
    episodes.sort((a, b) => a.number - b.number);
    return episodes;
  }

  // Step 3: episode page -> Dailymotion private id (from the player iframe or the Base64 server list).
  // Step 4: Dailymotion metadata -> qualities.auto[0].url = master .m3u8 (step 5 is the player's job).
  async findEpisodeServer(episode, _server) {
    const slug = typeof episode === "string" ? episode : episode.id;
    const html = await this.getHTML(`${this.base}/${encodeURIComponent(slug)}/`);

    // Every embed on the page: the iframes, plus each <select class="mirror"> option (Base64 of an <iframe>).
    const embeds = [];
    for (const m of html.matchAll(/<iframe\b[^>]*\bsrc="([^"]+)"/gi)) embeds.push(m[1]);
    const mirror = html.match(/<select\b[^>]*class="mirror"[^>]*>([\s\S]*?)<\/select>/i)?.[1] ?? "";
    for (const m of mirror.matchAll(/<option\b[^>]*value="([A-Za-z0-9+/=]+)"/gi)) {
      try {
        const iframe = atob(m[1]);
        const src = iframe.match(/\bsrc="([^"]+)"/i)?.[1];
        if (src) embeds.push(src);
      } catch { /* not Base64 */ }
    }

    const dm = embeds.map((u) => this.decode(u)).find((u) => /dailymotion\.com\/(player|embed)/i.test(u));
    const privateId = dm && (dm.match(/[?&]video=([A-Za-z0-9]+)/)?.[1] || dm.match(/\/video\/([A-Za-z0-9]+)/)?.[1]);
    if (!privateId) {
      throw new Error(`No Dailymotion video on the episode page. Embeds found: ${embeds.join(" , ") || "(none)"}`);
    }

    // Asked the way the embedded player asks it: from geo.dailymotion.com.
    const meta = await this.getJSON(`${this.dmBase}/player/metadata/video/${encodeURIComponent(privateId)}`, {
      referer: `${this.dmPlayer}/`,
      origin: this.dmPlayer,
    });
    if (meta.error) throw new Error(`Dailymotion: ${meta.error.title || meta.error.message || JSON.stringify(meta.error)}`);
    const auto = meta.qualities?.auto || [];
    const hls = auto.find((q) => /mpegurl/i.test(q.type || "")) || auto[0];
    if (!hls?.url) throw new Error(`Dailymotion metadata has no qualities.auto link (video ${privateId})`);

    return {
      server: "Dailymotion",
      // Headers for the player loading the m3u8 (tested with curl over HTTP/1.1):
      //   Accept-Language is required (403 without it); Origin is needed with a browser User-Agent;
      //   Referer must NOT be sent (403 whenever it's there).
      headers: {
        "User-Agent": "VLC/3.0.21 LibVLC/3.0.21",
        "Icy-MetaData": "1",
        "Accept-Language": "en_US",
      },
      videoSources: [
        {
          url: hls.url,
          quality: "auto",
          type: "m3u8",
          subtitles: [], // episodes are "English Subtitles"; no separate subtitle files are listed
        },
      ],
    };
  }

  // Names your tool looks for. They only call the methods above.
  async episodes(id) {
    return this.findEpisodes(id);
  }

  // The tool passes the episode id string and expects { headers, sources: [...] }.
  async sources(episodeId) {
    const result = await this.findEpisodeServer({ id: episodeId }, "Dailymotion");
    return {
      headers: result.headers,
      sources: result.videoSources.map((s) => ({
        url: s.url,
        type: s.type,
        quality: s.quality,
        label: result.server,
        headers: result.headers,
        subtitles: s.subtitles,
      })),
    };
  }
}
