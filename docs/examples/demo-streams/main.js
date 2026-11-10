// Demo streams: a B-Anime extension that serves public test videos for any search.
// It shows the provider shape without scraping anything. See docs/extensions.md.

const BASE = "https://raw.githubusercontent.com/ahmedanjum30/B-Anime/main/docs/examples/demo-streams/";

const VIDEOS = [
  {
    title: "Big Buck Bunny (Blender open movie, external subtitles)",
    url: "https://test-streams.mux.dev/x36xhzz/x36xhzz.m3u8",
    subtitles: [{ url: BASE + "bbb-en.vtt", lang: "en", label: "English (demo)" }],
  },
  {
    title: "Apple advanced HLS example (subtitles in the stream)",
    url: "https://devstreaming-cdn.apple.com/videos/streaming/examples/img_bipbop_adv_example_fmp4/master.m3u8",
  },
  {
    title: "Apple HLS sample (bipbop)",
    url: "https://devstreaming-cdn.apple.com/videos/streaming/examples/bipbop_16x9/bipbop_16x9_variant.m3u8",
  },
  {
    title: "Tears of Steel (Blender open movie, English and Italian audio)",
    url: "https://demo.unified-streaming.com/k8s/features/stable/video/tears-of-steel/tears-of-steel-multi-lang.ism/.m3u8",
  },
  {
    title: "Angel One (Shaka demo, five audio languages)",
    url: "https://storage.googleapis.com/shaka-demo-assets/angel-one-hls/hls.m3u8",
  },
  {
    title: "Dual-audio file (MP4 with Japanese and English tracks)",
    url: BASE + "dual-audio.mp4",
    type: "mp4",
  },
];

class Provider {
  // Any search returns one "show"; match it to any AniList entry to try the player.
  async search(query) {
    console.log("demo search for", query.query);
    return [{ id: "demo", title: `Demo streams (${VIDEOS.length} test videos)` }];
  }

  // One "episode" per test video.
  async episodes(id) {
    return VIDEOS.map((v, i) => ({ id: String(i), number: i + 1, title: v.title }));
  }

  async sources(episodeId) {
    const video = VIDEOS[Number(episodeId)];
    if (!video) throw new Error(`unknown episode ${episodeId}`);
    return {
      sources: [{ url: video.url, type: video.type ?? "hls", quality: "auto" }],
      subtitles: video.subtitles ?? [],
    };
  }
}
