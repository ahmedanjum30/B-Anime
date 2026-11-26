import { createSignal, onCleanup } from "solid-js";
import type Hls from "hls.js";

/** What the player plays: the preview reads the same video. */
export interface PreviewSource {
  url: string;
  kind: "hls" | "mp4" | "file";
}

/** A frame for the timeline's hover tip: where it is (s) and the picture. */
export interface PreviewFrame {
  t: number;
  src: string;
}

/** How long the pointer rests on a spot before its frame is fetched: a library file reads its
 *  disk at once; a stream loads a piece of video, so a sweep across the bar loads nothing. */
const REST_FILE = 120;
const REST_STREAM = 300;
/** Frames within this many seconds of the pointer stand for it (a piece of a stream covers a few). */
const NEAR = 4;
/** The preview's video goes this long after the pointer leaves the bar: no memory held meanwhile. */
const RELEASE_AFTER = 2000;
/** Frames kept, so going back over a spot shows it at once. */
const KEEP = 40;
const WIDTH = 192;

/**
 * Previews on the timeline: the frame at the spot the pointer rests on. A small video of its
 * own, made only while the pointer is on the bar and gone two seconds after it leaves (nothing
 * plays in the background). A library file is read from disk; a stream loads only the piece at
 * that spot, at its lowest quality, and stops there.
 */
export function createSeekPreview(source: () => PreviewSource | null, enabled: () => boolean) {
  const [frame, setFrame] = createSignal<PreviewFrame | null>(null);
  const kept = new Map<number, PreviewFrame>();
  let video: HTMLVideoElement | null = null;
  let hls: Hls | null = null;
  let loadedUrl = "";
  let want: number | null = null;
  let restTimer: number | undefined;
  let releaseTimer: number | undefined;
  const canvas = document.createElement("canvas");

  const release = () => {
    clearTimeout(restTimer);
    hls?.destroy();
    hls = null;
    if (video) {
      video.removeAttribute("src");
      video.load();
    }
    video = null;
    loadedUrl = "";
  };
  onCleanup(() => {
    release();
    clearTimeout(releaseTimer);
  });

  const keep = (f: PreviewFrame) => {
    kept.set(Math.round(f.t), f);
    if (kept.size > KEEP) kept.delete(kept.keys().next().value!);
  };
  const nearest = (t: number) => {
    let best: PreviewFrame | null = null;
    for (const f of kept.values()) if (Math.abs(f.t - t) <= NEAR && (!best || Math.abs(f.t - t) < Math.abs(best.t - t))) best = f;
    return best;
  };

  /** The frame the video shows now, into the kept ones. */
  const grab = () => {
    const v = video;
    if (!v || want === null || !v.videoWidth || Math.abs(v.currentTime - want) > NEAR) return;
    canvas.width = WIDTH;
    canvas.height = Math.round((WIDTH * v.videoHeight) / v.videoWidth);
    const g = canvas.getContext("2d");
    if (!g) return;
    try {
      g.drawImage(v, 0, 0, canvas.width, canvas.height);
      const f = { t: v.currentTime, src: canvas.toDataURL("image/jpeg", 0.72) };
      keep(f);
      if (want !== null && Math.abs(want - f.t) <= NEAR) setFrame(f);
    } catch {
      /* a video the page can't read back: no preview */
    }
    // A stream stops loading once it has the frame (no buffering ahead in the background).
    hls?.stopLoad();
  };

  const open = async (src: PreviewSource) => {
    release();
    const v = document.createElement("video");
    v.muted = true;
    v.preload = "auto";
    v.crossOrigin = "anonymous";
    v.playsInline = true;
    v.addEventListener("seeked", grab);
    v.addEventListener("loadeddata", grab);
    video = v;
    loadedUrl = src.url;
    const onWebKit = /Macintosh/.test(navigator.userAgent) && !/Chrome|Chromium|Edg\//.test(navigator.userAgent);
    if (src.kind === "hls" && !onWebKit) {
      const HlsLib = (await import("hls.js")).default;
      if (video !== v || !HlsLib.isSupported()) return;
      const h = new HlsLib({
        autoStartLoad: false,
        startLevel: 0,
        capLevelToPlayerSize: false,
        maxBufferLength: 1,
        maxMaxBufferLength: 2,
        backBufferLength: 0,
        enableWorker: true,
      });
      hls = h;
      // The lowest quality: a preview is 192 pixels wide.
      h.on(HlsLib.Events.MANIFEST_PARSED, () => {
        h.currentLevel = 0;
        if (want !== null) h.startLoad(want);
      });
      h.loadSource(src.url);
      h.attachMedia(v);
    } else {
      v.src = src.url;
    }
  };

  const fetchAt = async (t: number) => {
    const src = source();
    if (!src) return;
    if (!video || loadedUrl !== src.url) await open(src);
    const v = video;
    if (!v) return;
    if (hls) hls.startLoad(t);
    v.currentTime = t;
  };

  return {
    /** The frame to show for the pointer's spot (or none yet). */
    frame,
    /** The pointer is over `t` on the timeline. */
    hover(t: number) {
      if (!enabled() || !matchMedia("(hover: hover)").matches) return;
      clearTimeout(releaseTimer);
      want = t;
      const near = nearest(t);
      setFrame(near);
      if (near) return;
      clearTimeout(restTimer);
      const src = source();
      restTimer = window.setTimeout(() => void fetchAt(t), src?.kind === "file" ? REST_FILE : REST_STREAM);
    },
    /** The pointer left the timeline: the preview's video goes shortly after. */
    leave() {
      want = null;
      clearTimeout(restTimer);
      setFrame(null);
      clearTimeout(releaseTimer);
      releaseTimer = window.setTimeout(release, RELEASE_AFTER);
    },
    /** Another episode or source: what's kept is another video's. */
    reset() {
      kept.clear();
      setFrame(null);
      release();
    },
  };
}
