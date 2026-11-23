class Provider {
  constructor() {
    this.base = "https://ani.pm"; // local server
    this.site = "https://ani.pm";        // only used for display links
    this.embedBase = "https://embed.settlar.io"; 
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
      episodeServers: ["Settlar", "Beta"],
      supportsDub: true,
    };
  }

  // referer/origin default to the local server; the embed call passes the player's own.
  async request(url, { referer = `${this.base}/`, origin = this.base, accept = "application/json" } = {}) {
    const tab = await this.getTab();
    const res = await tab(url, {
      mode: "cors",
      referer,
      origin,
      headers: { Accept: accept },
    });
    const text = await res.text();
    return { res, text };
  }

  async getJSON(url, opts) {
    const { res, text } = await this.request(url, opts);
    let json = {};
    try { json = JSON.parse(text); } catch { /* not JSON */ }
    if (!res.ok) throw new Error(`${url} -> HTTP ${res.status}: ${json.error || json.message || text.slice(0, 200)}`);
    return json;
  }

  // Ids carry everything later steps need, separated by "/":
  //   show    = "{settlar|anilist}/{key}/{sub|dub}/{anilistId}"
  //   episode = "{settlar|anilist}/{key}/{number}/{sub|dub}/{anilistId}"
  // key = ani.pm id for settlar shows, AniList id for anilist-only shows. anilistId is used by the beta server.
  parseEpisodeId(id) {
    const [src, key, number, lang = "sub", anilistId] = String(id).split("/");
    return { src, key, number, lang, anilistId: anilistId || (src === "anilist" ? key : "") };
  }

  // Step 1: search.
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
        id: `${isAni ? "anilist" : "settlar"}/${key}/${lang}/${m.anilistId ?? ""}`,
        title: m.title,
        url: `${this.site}/${isAni ? "ani" : "anime"}/${m.routeId}`,
        subOrDub: m.dubCount > 0 && m.subCount > 0 ? "both" : m.dubCount > 0 ? "dub" : "sub",
      });
    }
    if (!results.length) throw new Error("No anime found");
    return results;
  }

  // Step 2: series + episodes.
  async findEpisodes(id) {
    const [src, key, lang = "sub", showAnilistId] = String(id).split("/");
    const path = src === "anilist" ? "ani" : "series";
    const json = await this.getJSON(`${this.base}/api/anime/${path}/${encodeURIComponent(key)}`);
    const data = json.data ?? json;
    const anilistId = showAnilistId || data.anilistId || (src === "anilist" ? key : "");
    const episodes = [];
    for (const ep of data.episodes || []) {
      const number = Number(ep.number);
      if (!Number.isFinite(number)) continue;
      // Skip episodes the series says aren't available in this language (when it says).
      if (lang === "dub" && ep.dub === false) continue;
      if (lang === "sub" && ep.sub === false) continue;
      episodes.push({
        id: `${src}/${key}/${number}/${lang}/${anilistId ?? ""}`,
        title: ep.title || `Episode ${number}`,
        number,
        url: `${this.site}/${path === "ani" ? "ani" : "anime"}/${data.routeId ?? key}?ep=${encodeURIComponent(ep.routeId || String(number))}`,
      });
    }
    episodes.sort((a, b) => a.number - b.number);
    return episodes;
  }

  // server "Settlar" or "Beta". Anything else: Settlar first, Beta if Settlar fails.
  async findEpisodeServer(episode, server) {
    if (server === "Settlar") return this.settlarServer(episode);
    if (server === "Beta") return this.betaServer(episode);
    try {
      return await this.settlarServer(episode);
    } catch (settlarErr) {
      try {
        return await this.betaServer(episode);
      } catch (betaErr) {
        throw new Error(`Settlar: ${settlarErr.message} | Beta: ${betaErr.message}`);
      }
    }
  }

  // Beta server (step 2B): /api/anime/beta-server/{anilistId} -> origin + path, then a predictable master link:
  //   {origin}/hls/{anilistId}-{slug}/E{0001}/master.m3u8
  // Sub and dub are audio tracks inside the same master; subtitles are HLS tracks inside it too.
  async betaServer(episode) {
    const { number, anilistId } = this.parseEpisodeId(episode.id);
    if (!anilistId) throw new Error("No AniList id for this show, so the beta server can't be asked");

    const info = await this.getJSON(`${this.base}/api/anime/beta-server/${encodeURIComponent(anilistId)}`);
    if (!info.available) throw new Error(`Not on the beta server (AniList ${anilistId})`);
    if (Array.isArray(info.episodes) && !info.episodes.map(Number).includes(Number(number))) {
      throw new Error(`Episode ${number} isn't on the beta server`);
    }

    // path "/watch/{slug}-{anilistId}" -> folder "{anilistId}-{slug}"
    const origin = String(info.origin || "https://beta.cno.to").replace(/\/+$/, "");
    const slug = String(info.path || "").split("/").pop().replace(new RegExp(`-${anilistId}$`), "");
    if (!slug) throw new Error(`Beta server gave no path for AniList ${anilistId}`);
    const e4 = `E${String(number).padStart(4, "0")}`;
    const master = `${origin}/hls/${anilistId}-${slug}/${e4}/master.m3u8`;

    // Make sure the master exists ("cold" shows may not be built yet).
    const headers = { referer: `${origin}${info.path}/${number}`, origin };
    const { res, text } = await this.request(master, { ...headers, accept: "*/*" });
    if (!res.ok || !text.startsWith("#EXTM3U")) {
      throw new Error(`Beta master not ready (HTTP ${res.status}${info.cold ? ", cold: true" : ""}): ${master}`);
    }

    return {
      server: "Beta",
      headers: {
        Referer: `${origin}/`,
        Origin: origin,
      },
      videoSources: [
        {
          url: master,
          quality: "auto",
          type: "m3u8",
          subtitles: [], // inside the master playlist as HLS subtitle tracks
        },
      ],
    };
  }

  // Steps 3A -> 3B -> 4: playback-bootstrap -> settlar/session (player link) -> embed session (m3u8 + subtitles)
  async settlarServer(episode) {
    const { src, key, number, lang } = this.parseEpisodeId(episode.id);

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
      audio: channel, // the language actually served (bootstrap may fall back from dub to sub)
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
            srclang: s.srclang,
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
  // Asks at once: Settlar in the episode's language, Beta, and Settlar in the other language (when the
  // show has it). Returns every one that worked, in that order.
  // Settlar sources carry `audio` ("sub"/"dub") so the app's Audio menu can switch between them. Beta has
  // no `audio`: both languages are tracks inside its master, and the player lists those itself.
  // headers = the first working server's; each source also carries its own server's headers.
  async sources(episodeId) {
    const ep = this.parseEpisodeId(episodeId);
    const other = ep.lang === "dub" ? "sub" : "dub";
    const jobs = [
      { server: "Settlar", id: episodeId, lang: ep.lang },
      { server: "Beta", id: episodeId },
      { server: "Settlar", id: [ep.src, ep.key, ep.number, other, ep.anilistId].join("/"), lang: other },
    ];
    const results = await Promise.allSettled(jobs.map((j) => this.findEpisodeServer({ id: j.id }, j.server)));
    const ok = [];
    const errors = [];
    results.forEach((r, i) => {
      const job = jobs[i];
      const name = job.lang ? `${job.server} (${job.lang})` : job.server;
      if (r.status !== "fulfilled") return errors.push(`${name}: ${r.reason?.message ?? r.reason}`);
      // Settlar serves the other language when this one is missing; that one is asked for separately.
      if (job.lang && r.value.audio !== job.lang) return errors.push(`${name}: not available (got ${r.value.audio})`);
      ok.push(r.value);
    });
    if (!ok.length) throw new Error(errors.join(" | "));
    return {
      headers: ok[0].headers,
      sources: ok.flatMap((result) => result.videoSources.map((s) => ({
        url: s.url,
        type: s.type,
        quality: s.quality,
        label: result.audio ? `${result.server} (${result.audio})` : result.server,
        audio: result.audio,
        headers: result.headers,
        subtitles: s.subtitles,
      }))),
    };
  }
}
