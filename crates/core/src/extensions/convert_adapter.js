// Presents a converted provider (`__InnerProvider`, captured from its code) as a
// B-Anime provider. `__extKind` is "anime" (onlinestream-provider) or "manga".
const __languageCodes = {
  english: "en", japanese: "ja", spanish: "es", "spanish (latin america)": "es", "latin spanish": "es", portuguese: "pt",
  "portuguese (brazil)": "pt", "brazilian portuguese": "pt", french: "fr", german: "de", italian: "it", russian: "ru",
  arabic: "ar", indonesian: "id", malay: "ms", thai: "th", vietnamese: "vi", turkish: "tr", polish: "pl", korean: "ko",
  chinese: "zh", "chinese (simplified)": "zh", "chinese (traditional)": "zh", hindi: "hi", dutch: "nl", ukrainian: "uk",
  romanian: "ro", czech: "cs", hungarian: "hu", swedish: "sv", finnish: "fi", greek: "el", hebrew: "he", persian: "fa",
};

const __languageCode = (name) => {
  const raw = String(name || "").trim();
  const lower = raw.toLowerCase();
  if (/^[a-z]{2,3}(-[a-z0-9]+)?$/i.test(raw)) return lower.split("-")[0];
  if (__languageCodes[lower]) return __languageCodes[lower];
  const first = lower.split(/[\s(-]/)[0];
  return __languageCodes[first] || "und";
};

class Provider {
  constructor() {
    if (typeof __InnerProvider !== "function") throw new Error("the extension doesn't define a Provider class");
    this.inner = new __InnerProvider();
  }

  async settings() {
    try { return (await this.inner.getSettings()) || {}; } catch { return {}; }
  }

  async search(q) {
    const media = q.media || {};
    const year = q.year ?? media.year;
    if (__extKind === "manga") {
      const list = (await this.inner.search({ query: q.query, year })) || [];
      return list.map((r) => ({ id: String(r.id), title: r.title, image: r.image || undefined, year: r.year || undefined }));
    }
    const titles = media.titles || [];
    const opts = {
      query: q.query,
      dub: !!q.dub,
      year,
      media: {
        id: media.id || 0,
        idMal: media.idMal || undefined,
        romajiTitle: titles[0] || q.query,
        englishTitle: titles[1] || undefined,
        synonyms: titles.slice(2),
        episodeCount: media.episodes || undefined,
        format: media.format || undefined,
        status: media.status || undefined,
        isAdult: false,
        startDate: year ? { year } : undefined,
      },
    };
    const supportsDub = !!(await this.settings()).supportsDub;
    // A source without dubs has nothing to offer a dub search; answering with its subbed
    // results would get them matched (and played) as the dub.
    if (q.dub && !supportsDub) return [];
    let list = (await this.inner.search(opts)) || [];
    // Sources with dubs label each result; keep only the version asked for. Unlabelled results
    // are kept for sub searches only.
    if (supportsDub) {
      const want = q.dub ? "dub" : "sub";
      list = list.filter((r) => r.subOrDub === want || r.subOrDub === "both" || (!r.subOrDub && !q.dub));
    }
    return list.map((r) => ({ id: String(r.id), title: r.title, image: r.image || undefined, year: r.year || undefined }));
  }

  async episodes(id) {
    const list = (await this.inner.findEpisodes(id)) || [];
    // The original API hands the whole episode back when asking for its video, so it rides in the id.
    return list.map((e) => ({
      id: JSON.stringify({ id: e.id, number: e.number, url: e.url || "", title: e.title || undefined }),
      number: Number(e.number),
      title: e.title || undefined,
    }));
  }

  async sources(episodeId) {
    let episode;
    try { episode = JSON.parse(episodeId); } catch { episode = { id: episodeId, number: 0, url: "" }; }
    const settings = await this.settings();
    const servers = settings.episodeServers && settings.episodeServers.length ? settings.episodeServers : ["default"];
    const errors = [];
    for (const server of servers) {
      try {
        const res = await this.inner.findEpisodeServer(episode, server);
        const videos = (res && res.videoSources) || [];
        if (!videos.length) { errors.push(`${server}: no video`); continue; }
        const subtitles = [];
        const seen = new Set();
        for (const v of videos) {
          for (const s of v.subtitles || []) {
            if (!s || !s.url || seen.has(s.url)) continue;
            seen.add(s.url);
            const entry = { url: s.url, lang: __languageCode(s.language), label: s.language || undefined };
            if (s.isDefault) subtitles.unshift(entry); else subtitles.push(entry);
          }
        }
        return {
          headers: res.headers || {},
          sources: videos.map((v) => ({
            url: v.url,
            type: v.type === "m3u8" ? "hls" : v.type === "mp4" ? "mp4" : undefined,
            label: [servers.length > 1 ? res.server || server : "", v.quality, v.label].filter(Boolean).join(" · ") || undefined,
          })),
          subtitles,
        };
      } catch (e) {
        errors.push(`${server}: ${e && e.message ? e.message : e}`);
        console.warn(`server ${server} failed:`, e && e.message ? e.message : String(e));
      }
    }
    throw new Error(`no server had this episode (${errors.join("; ")})`);
  }

  async chapters(id) {
    const list = (await this.inner.findChapters(id)) || [];
    return list.map((c, i) => {
      const n = parseFloat(String(c.chapter).replace(/[^0-9.]/g, ""));
      return {
        id: String(c.id),
        number: Number.isFinite(n) ? n : (c.index ?? i) + 1,
        title: c.title || undefined,
      };
    });
  }

  async pages(chapterId) {
    const list = ((await this.inner.findChapterPages(chapterId)) || []).slice().sort((a, b) => (a.index ?? 0) - (b.index ?? 0));
    return { headers: (list[0] && list[0].headers) || {}, pages: list.map((p) => ({ url: p.url })) };
  }
}

// B-Anime checks a provider's methods at install; only show the ones this kind uses.
if (__extKind === "anime") { delete Provider.prototype.chapters; delete Provider.prototype.pages; }
else { delete Provider.prototype.episodes; delete Provider.prototype.sources; }
