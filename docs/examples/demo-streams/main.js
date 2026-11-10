// Demo streams: a B-Anime extension that serves public test videos for any search.
// It shows the provider shape without scraping anything. See docs/extensions.md.

const VIDEOS = [
  {
    id: "big-buck-bunny",
    title: "Big Buck Bunny (Blender open movie)",
    url: "https://test-streams.mux.dev/x36xhzz/x36xhzz.m3u8",
  },
  {
    id: "sintel",
    title: "Sintel (Blender open movie, with subtitles)",
    url: "https://bitdash-a.akamaihd.net/content/sintel/hls/playlist.m3u8",
  },
  {
    id: "bipbop",
    title: "Apple HLS sample (bipbop)",
    url: "https://devstreaming-cdn.apple.com/videos/streaming/examples/bipbop_16x9/bipbop_16x9_variant.m3u8",
  },
];

class Provider {
  // Every search returns all three videos, so any show "matches" for testing.
  async search(query) {
    console.log("demo search for", query.query);
    return VIDEOS.map((v) => ({ id: v.id, title: v.title }));
  }

  // One "episode" per video; episode 1 of whatever show you're on plays it.
  async episodes(id) {
    return [{ id, number: 1, title: VIDEOS.find((v) => v.id === id)?.title ?? id }];
  }

  async sources(episodeId) {
    const video = VIDEOS.find((v) => v.id === episodeId);
    if (!video) throw new Error(`unknown video ${episodeId}`);
    return {
      sources: [{ url: video.url, type: "hls", quality: "auto" }],
      subtitles: [],
    };
  }
}
