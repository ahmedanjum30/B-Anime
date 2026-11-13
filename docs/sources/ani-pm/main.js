class Provider {
  constructor() {
    this.base = "https://ani.pm"; // local server
    this.site = "https://ani.pm";        // only used for display links
    this.embedBase = "https://embed.settlar.io";                 // embed session server; "" = use the host from the player link
    // mozillaFetch tab (sets Firefox User-Agent and Accept-Language itself).
    // Created on the first request, because main.js is loaded as a script, so a top-level import isn't allowed.
    this.tab = null;
  }

  async getTab() {
    if (!this.tab) {
      const { createMozillaFetch } = await import("mozillafetch");
      this.tab = createMozillaFetch({ session: true });
    }
    return this.tab;
  }

  getSettings() {
    return {
      episodeServers: ["Settlar"],
      supportsDub: true,
    };
  }

  // referer/origin default to the local server; the embed call passes the player's own.
  async getJSON(url, { referer = `${this.base}/`, origin = this.base } = {}) {
    const tab = await this.getTab();
    const res = await tab(url, {
      mode: "cors",
      referer,
      origin,
      headers: { Accept: "application/json" },
    });
    const text = await res.text();
    let json = {};
    try { json = JSON.parse(text); } catch { /* not JSON */ }
    if (!res.ok) throw new Error(`${url} -> HTTP ${res.status}: ${json.error || json.message || text.slice(0, 200)}`);
    return json;
  }

  // Step 1: search. Result id = "{settlar|anilist}/{key}/{sub|dub}"
  //   settlar shows are keyed by the ani.pm id, anilist-only shows by the AniList id.
  async search(query) {
    const q = typeof query === "string" ? query : query.query;
    const lang = query && query.dub ? "dub" : "sub";
    const json = await this.getJSON(`${this.base}/api/anime/search?${new URLSearchParams({ q })}`);
    const results = [];
    for (const m of json.items || []) {
      const isAni = m.source === "anilist";
      const key = isAni ? m.anilistId : m.id;
      if (key == null) continue;
      if (lang === "dub" && m.dubCount === 0) continue;
      results.push({
        id: `${isAni ? "anilist" : "settlar"}/${key}/${lang}`,
        title: m.title,
        url: `${this.site}/${isAni ? "ani" : "anime"}/${m.routeId}`,
        subOrDub: m.dubCount > 0 && m.subCount > 0 ? "both" : m.dubCount > 0 ? "dub" : "sub",
      });
    }
    if (!results.length) throw new Error("No anime found");
    return results;
  }

  // Step 2: series + episodes. Episode id = "{source}/{key}/{number}/{sub|dub}"
  async findEpisodes(id) {
    const [src, key, lang = "sub"] = String(id).split("/");
    const path = src === "anilist" ? "ani" : "series";
    const json = await this.getJSON(`${this.base}/api/anime/${path}/${encodeURIComponent(key)}`);
    const data = json.data ?? json;
    const episodes = [];
    for (const ep of data.episodes || []) {
      const number = Number(ep.number);
      if (!Number.isFinite(number)) continue;
      // Skip episodes the series says aren't available in this language (when it says).
      if (lang === "dub" && ep.dub === false) continue;
      if (lang === "sub" && ep.sub === false) continue;
      episodes.push({
        id: `${src}/${key}/${number}/${lang}`,
        title: ep.title || `Episode ${number}`,
        number,
        url: `${this.site}/${path === "ani" ? "ani" : "anime"}/${data.routeId ?? key}?ep=${encodeURIComponent(ep.routeId || String(number))}`,
      });
    }
    episodes.sort((a, b) => a.number - b.number);
    return episodes;
  }

  // Steps 3A -> 3B -> 4: playback-bootstrap -> settlar/session (player link) -> embed session (m3u8 + subtitles)
  async findEpisodeServer(episode, _server) {
    const [src, key, number, lang = "sub"] = String(episode.id).split("/");

    // A) bootstrap: gives a one-time settlarSelection token
    const qa = new URLSearchParams({ ep: number, lang, core: "0" });
    const boot = await this.getJSON(`${this.base}/api/anime/playback-bootstrap/${src}/${encodeURIComponent(key)}?${qa}`);
    if (!boot.settlarSelection) throw new Error(`No settlarSelection for episode ${number} (${lang})`);

    // B) session: swaps the token for the player link embed.settlar.io/embed/v1?t=...
    const channel = boot.effectiveLanguage || lang;
    const qb = new URLSearchParams({
      selection: boot.settlarSelection, provider: "anipm", ep: String(boot.episode ?? number), channel, telemetry: "0",
    });
    const sess = await this.getJSON(`${this.base}/api/anime/settlar/session?${qb}`);
    if (!sess.embedUrl) throw new Error("settlar/session returned no embedUrl");

    // C) what the player does with t=: /api/embed/session?t=... -> { source (master .m3u8), subtitles }
    // The local server doesn't forward this one, so it's called directly.
    const embed = new URL(sess.embedUrl);
    const t = embed.searchParams.get("t");
    const embedBase = (this.embedBase || embed.origin).replace(/\/+$/, "");
    let stream;
    try {
      stream = await this.getJSON(`${embedBase}/api/embed/session?t=${encodeURIComponent(t)}`, {
        referer: "https://embed.settlar.io",
        origin: "https://embed.settlar.io",
      });
    } catch (err) {
      throw new Error(`Embed session blocked (${err.message}). Player link: ${sess.embedUrl}`);
    }
    if (!stream.source) throw new Error(`Embed session has no source. Player link: ${sess.embedUrl}`);

    return {
      server: "Settlar",
      headers: {
        Referer: `${embed.origin}/`,
        Origin: embed.origin,
      },
      videoSources: [
        {
          url: stream.source,
          quality: "auto",
          type: stream.kind === "hls" || !stream.kind ? "m3u8" : stream.kind,
          subtitles: (stream.subtitles || []).map((s, i) => ({
            id: String(i),
            url: s.url,
            language: s.label || s.srclang,
            isDefault: !!s.default,
          })),
        },
      ],
    };
  }

  // Names your tool looks for. They only call the methods above — no behavior change.
  async episodes(id) {
    return this.findEpisodes(id);
  }

  // The tool passes the episode id string and expects { headers, sources: [...] }.
  async sources(episodeId) {
    const result = await this.findEpisodeServer({ id: episodeId }, "Settlar");
    return {
      headers: result.headers,
      sources: result.videoSources.map((s) => ({
        url: s.url,
        type: s.type,
        quality: s.quality,
        label: result.server,
        subtitles: s.subtitles,
      })),
    };
  }
}
