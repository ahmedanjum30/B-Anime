// AnimeVillage provider for B-Anime (apiVersion 1): search(), episodes(), sources().
// Talks to the local server (animevillage/server.js), which forwards to animevillage.org:
//   GET /api/search?search=&page=1&perPage=   -> { data: { Page: { media: [...] } } }   (AniList ids)
//   GET /api/episodes/:anilistId?dub=bool    -> { episodes: [{ id: "show-slug?ep=123", number, title }] }
//   GET /api/stream/:showSlug?category=sub|dub&server=hd-1&ep=123
//                                            -> { headers, sources: [{ url, quality, isM3U8 }], subtitles: [{ url, lang, label }] }
//
// Sub/dub: only search() is told which one the user wants, so it is carried along in the ids:
//   search result id:  "158927" (sub) or "158927:dub"
//   episode id:        "sub|spy-x-family-season-2-qlfdj-6399?ep=107908"

const BASE = "https://animevillage.org";
const SERVER = "hd-1";

// Headers the video host expects. The stream API normally returns these too;
// these are the fallback, and anything the API returns wins.
const REFERER = "https://megaplay.buzz/";
const ORIGIN = "https://megaplay.buzz";
const HEADERS = { Referer: REFERER, Origin: ORIGIN };

const LANGS = {
  english: "en", japanese: "ja", arabic: "ar", french: "fr", german: "de", italian: "it", spanish: "es",
  portuguese: "pt", russian: "ru", indonesian: "id", malay: "ms", thai: "th", vietnamese: "vi", turkish: "tr",
  polish: "pl", korean: "ko", chinese: "zh", hindi: "hi", dutch: "nl",
};

// "en" -> "en", "arabic" -> "ar", "portuguese (- portuguese(brazil))" -> "pt"
function langCode(lang) {
  const s = String(lang || "").trim().toLowerCase();
  if (/^[a-z]{2,3}(-[a-z0-9]+)?$/.test(s) && !LANGS[s]) return s.split("-")[0];
  return LANGS[s] || LANGS[s.split(/[\s(-]/)[0]] || "und";
}

async function getJSON(url) {
  const res = await fetch(url, { headers: { Accept: "application/json" } });
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.message || body.error || `HTTP ${res.status} for ${url}`);
  return body;
}

class Provider {
  async search(q) {
    const params = new URLSearchParams({ search: q.query, page: "1", perPage: "15" });
    const json = await getJSON(`${BASE}/api/search?${params}`);
    const media = json?.data?.Page?.media || [];
    const suffix = q.dub ? ":dub" : "";

    return media.map((m) => ({
      id: `${m.id}${suffix}`,
      title: m.title?.english || m.title?.romaji || m.title?.native || String(m.id),
      image: m.coverImage?.large || m.coverImage?.medium || undefined,
      year: m.seasonYear || m.startDate?.year || undefined,
    }));
  }

  async episodes(id) {
    const [showId, flag] = String(id).split(":");
    const category = flag === "dub" ? "dub" : "sub";

    // When AniList finds nothing, search falls back to results with slug ids
    // ("spy-x-family-season-3-hwhtf-8184"). The episodes API wants the AniList id,
    // and /api/anime/<slug> returns it.
    let anilistId = showId;
    if (!/^\d+$/.test(showId)) {
      const info = await getJSON(`${BASE}/api/anime/${encodeURIComponent(showId)}`);
      anilistId = info?.data?.Media?.id;
      if (!anilistId) throw new Error(`Couldn't find the AniList id for ${showId}`);
    }

    const json = await getJSON(`${BASE}/api/episodes/${encodeURIComponent(anilistId)}?dub=${category === "dub"}`);

    return (json.episodes || []).map((ep) => ({
      id: `${category}|${ep.id}`,
      number: Number(ep.number),
      title: ep.title || undefined,
    }));
  }

  // The episode in its own language and in the other one, so the player's Audio menu can switch.
  // Sub and dub share the episode id ("show-slug?ep=123"); only `category` changes. When a show has
  // no dub (or no sub), the API answers with the other language's link instead, so a second
  // language whose link is the same as the first one's is left out.
  async sources(episodeId) {
    const s = String(episodeId);
    const bar = s.indexOf("|");
    const category = bar > 0 ? s.slice(0, bar) : "sub";
    const rawId = bar > 0 ? s.slice(bar + 1) : s;
    const other = category === "dub" ? "sub" : "dub";

    // Same as the site: split "show-slug?ep=123" into the show part and the ep number.
    const [showPart, rest] = rawId.split("?ep=");
    const ep = rest ? rest.split("&")[0] : null;
    const stream = (cat) => {
      const params = new URLSearchParams({ category: cat, server: SERVER });
      if (ep) params.set("ep", ep);
      return getJSON(`${BASE}/api/stream/${encodeURIComponent(showPart)}?${params}`);
    };

    const [mine, theirs] = await Promise.allSettled([stream(category), stream(other)]);
    if (mine.status === "rejected" && theirs.status === "rejected") throw mine.reason;
    const own = mine.status === "fulfilled" ? (mine.value.sources || []).map((x) => x.url) : [];
    const answers = [];
    if (mine.status === "fulfilled") answers.push([category, mine.value]);
    if (theirs.status === "fulfilled" && (theirs.value.sources || []).some((x) => !own.includes(x.url))) {
      answers.push([other, theirs.value]);
    }

    const subtitles = [];
    for (const [, json] of answers) {
      for (const t of json.subtitles || []) {
        if (t && t.url && !subtitles.some((x) => x.url === t.url)) {
          subtitles.push({ url: t.url, lang: langCode(t.lang), label: t.label || t.lang || undefined });
        }
      }
    }

    return {
      headers: { ...answers[0][1].headers, ...HEADERS },
      sources: answers.flatMap(([cat, json]) =>
        (json.sources || []).map((src) => ({
          url: src.url,
          type: src.isM3U8 || /\.m3u8(?:[?#]|$)/i.test(src.url) ? "hls" : "mp4",
          quality: src.quality && src.quality !== "default" ? src.quality : undefined,
          label: `${SERVER.toUpperCase()} (${cat})`,
          audio: cat,
        })),
      ),
      subtitles,
    };
  }
}
