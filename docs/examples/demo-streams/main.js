// Demo streams: a B-Anime extension that serves public test videos for any search.
// It shows the provider shape without scraping anything. See docs/extensions.md.

const VIDEOS = [
  {
    title: "Big Buck Bunny (Blender open movie)",
    url: "https://test-streams.mux.dev/x36xhzz/x36xhzz.m3u8",
  },
  {
    title: "Sintel (Blender open movie, with subtitles)",
    url: "https://bitdash-a.akamaihd.net/content/sintel/hls/playlist.m3u8",
    subtitles: [{ url: "https://bitdash-a.akamaihd.net/content/sintel/subtitles/subtitles_en.vtt", lang: "en", label: "English" }],
  },
  {
    title: "Apple HLS sample (bipbop)",
    url: "https://devstreaming-cdn.apple.com/videos/streaming/examples/bipbop_16x9/bipbop_16x9_variant.m3u8",
  },
];

class Provider {
  // Any search returns one "show"; match it to any AniList entry to try the player.
  async search(query) {
    console.log("demo search for", query.query);
    return [{ id: "demo", title: "Demo streams (3 test videos)" }];
  }

  // Three "episodes", one per test video.
  async episodes(id) {
    return VIDEOS.map((v, i) => ({ id: String(i), number: i + 1, title: v.title }));
  }

  async sources(episodeId) {
    const video = VIDEOS[Number(episodeId)];
    if (!video) throw new Error(`unknown episode ${episodeId}`);
    return {
      sources: [{ url: video.url, type: "hls", quality: "auto" }],
      subtitles: video.subtitles ?? [],
    };
  }
}
