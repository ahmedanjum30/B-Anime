import { For, Show, createEffect, createMemo, createResource, createSignal, on, onCleanup, onMount, untrack } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { playerSlot, targetFromPath, type PlayerTarget } from "../lib/playerSlot";
import { createSeekPreview } from "../lib/seekPreview";
import type Hls from "hls.js";

import {
  ArrowLeft, AudioLines, Captions, ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight, ExternalLink, ListVideo, Maximize, Search, Minimize, Pause, PictureInPicture, Play, Settings2,
  SkipBack, SkipForward, Users, Volume2, VolumeX, X,
} from "../lib/icons";
import {
  errorText, externalPlayers, getMedia, googleSync, listExtensions, uiLog, getPlayerPrefs, getPosition, getSkipTimes, getStream, inTauri, localEpisodes,
  localStream, onAndroid, onWebKit, openExternalPlayer, pickSubtitleFile, saveEntry, savePosition,
} from "../lib/api";
import type { PictureTrack, Stream } from "../lib/types";
import { applyRules } from "../lib/entry";
import { fmtClock } from "../lib/format";
import { leaveTo } from "../lib/nav";
import { audioId, recallAudio, rememberAudio, rememberDub } from "../lib/dub";
import { createSource, recallServer, rememberServer, rememberShowSource, serverName, recallPlayFrom, rememberPlayFrom, recallShowSource, useRoomMatch } from "../lib/source";
import { sameServerIn, startSource } from "../lib/server-pick";
import { homeStore } from "../lib/store";
import { syncPlayer, watchStore } from "../lib/watch";
import Together from "./Together";
import type { SkipTime } from "../lib/types";

/** Fraction of the episode that counts as watched (and no earlier than its ending; see watchedAt). */
const WATCHED_AT = 0.85;
/** And at least this share of it actually played (not jumped or dragged over). */
const PLAYED_AT = 0.5;
const AUTO_NEXT_SECONDS = 8;
/** A stream that hasn't delivered any video by then gets an error instead of an endless spinner. */
const STALL_SECONDS = 30;
/** At most this many sign / song / note lines at the top at once. */
const TOP_CUES_MAX = 3;
/** Dialogue lines shown at the bottom at once (two people talking over each other). */
const BOTTOM_CUES_MAX = 2;
/** This many lines sharing exact start and end times are sign text, not dialogue. */
const SIGN_GROUP_MIN = 3;
const FORMAT_NAME: Record<string, string> = {
  TV: "TV", TV_SHORT: "TV short", MOVIE: "Movie", SPECIAL: "Special", OVA: "OVA", ONA: "ONA", MUSIC: "Music",
};
const SKIP_LABEL: Record<string, string> = {
  op: "Skip intro",
  "mixed-op": "Skip intro",
  ed: "Skip outro",
  "mixed-ed": "Skip outro",
  recap: "Skip recap",
};
/** What a marked section of the timeline is, for its hover label. */
const SECTION_NAME: Record<string, string> = {
  op: "Opening",
  "mixed-op": "Opening",
  ed: "Ending",
  "mixed-ed": "Ending",
  recap: "Recap",
};

const stored = (key: string, fallback: number) => {
  try {
    const v = Number(localStorage.getItem(key));
    return Number.isFinite(v) && localStorage.getItem(key) !== null ? v : fallback;
  } catch {
    return fallback;
  }
};
const store = (key: string, v: number) => {
  try {
    localStorage.setItem(key, String(v));
  } catch {
    /* not essential */
  }
};

export default function Player(props: { behind: boolean }) {
  const navigate = useNavigate();
  // Everything the player says is about the episode: on the video.
  const notify = homeStore.notifyPlayback;

  // What to play: the /watch page's episode (kept while the player closes).
  const target = createMemo<PlayerTarget>((prev) => playerSlot.target() ?? prev, playerSlot.target()!);
  const mediaId = () => target().id;
  const episode = () => target().ep;
  const extParam = () => target().ext;

  /** Playing a file from the library instead of an extension's stream. */
  const local = () => target().local;
  /** The English-dub version of the source. */
  const dubParam = () => target().dub;
  /** Another episode or source: on the player's page its address changes; behind the app only
   *  what it plays. */
  const go = (path: string) => {
    if (playerSlot.onPage()) return navigate(path, { replace: true });
    const t = targetFromPath(path);
    if (t) playerSlot.setTarget(t);
  };

  const [page] = createResource(mediaId, getMedia);
  const media = () => page()?.media;
  const [prefs] = createResource(getPlayerPrefs);
  // In a room, this device plays the episode from where it picked on the Together page: its
  // library, or a source. Not picked: the library when it has the episode. The room's links
  // don't say (each device has its own files), so the source waits for the library's answer.
  const [installed] = createResource(() => (watchStore.room() ? true : null), () => listExtensions().catch(() => []));
  /** Not picked: the host's source when this device has it (the same copy as the host's). */
  const hostPick = () => {
    const h = watchStore.hostSourceFor(mediaId());
    return h && installed()?.some((e) => e.id === h.id && e.enabled) ? h.id : null;
  };
  /** Library (true) or source (false): the show's choice, else in a room the host's source by
   *  default; null for the library when it has the episode. */
  const roomChoice = () => {
    const from = recallPlayFrom(mediaId());
    if (from) return from === "library";
    return hostPick() ? false : null;
  };
  const [libraryEps] = createResource(
    () => (!local() && watchStore.room() && roomChoice() !== false ? mediaId() : null),
    (id) => localEpisodes(id).catch(() => [] as number[]),
  );
  const fromLibrary = () =>
    !local() && !!watchStore.room() && roomChoice() !== false && !!libraryEps()?.includes(episode());
  const checkingLibrary = () => !local() && !!watchStore.room() && roomChoice() !== false && libraryEps.loading;
  const why = () =>
    `pick=${recallPlayFrom(mediaId())} ext=${recallShowSource(mediaId())} choice=${roomChoice()} hostPick=${hostPick()} local=${local()} ` +
    `libraryEps=${libraryEps.loading ? "loading" : JSON.stringify(libraryEps()?.length ?? null)} target=${target().path}`;
  createEffect(() => {
    if (fromLibrary()) {
      uiLog(`player → library copy: ${why()}`);
      go(`/watch/${mediaId()}/${episode()}?local=1`);
    }
    // Picked a source (or the host's by default) while the library copy plays: over to it.
    else if (local() && watchStore.room() && roomChoice() === false) {
      uiLog(`player → source: ${why()}`);
      go(`/watch/${mediaId()}/${episode()}${dubParam() ? "?dub=1" : ""}`);
    }
  });
  /** The source picked under Playing from, in a room (the player follows it exactly). */
  const pickedExt = () => (watchStore.room() && recallPlayFrom(mediaId()) === "source" ? recallShowSource(mediaId()) : null);
  const source = createSource(
    () => (local() || checkingLibrary() || fromLibrary() ? undefined : media()),
    () => extParam() ?? pickedExt() ?? hostPick() ?? undefined,
    dubParam,
  );
  // A guest on the host's source plays the show the host matched there (see useRoomMatch).
  useRoomMatch(source, mediaId);
  // Picked after the player had chosen one (or it chose a remembered one): over to the pick.
  createEffect(() => {
    const want = pickedExt();
    if (want && source.ext() && source.ext() !== want && source.usable().some((e) => e.id === want)) source.setExt(want);
  });
  // A held start waits for everyone to have the host's copy of the episode (the same length):
  // a different cut doesn't line up. Said on the held screen, with the way to the same copy.
  const hostSrc = () => watchStore.nowPlaying()?.source ?? null;
  const me = () => watchStore.members().find((m) => m.id === watchStore.room()?.me);
  const myGap = () => {
    const m = me();
    return m ? watchStore.lengthGap(m) : null;
  };
  const heldMismatch = () => {
    const gap = myGap();
    if (gap !== null) {
      const h = hostSrc();
      return `Your copy of this episode is ${watchStore.gapText(gap)} than the host's, so it wouldn't stay in step.${h ? "" : " Play it from the same source as the host."}`;
    }
    const others = watchStore.members().filter((m) => watchStore.lengthGap(m) !== null);
    if (!others.length) return null;
    return `Waiting for ${others.map((m) => m.name).join(", ")} to play the same copy as the host (theirs is ${watchStore.gapText(watchStore.lengthGap(others[0])!)})…`;
  };
  /** Over to the host's source (the same copy as the host's), from the held screen. */
  const toHostSource = (id: string) => {
    if (source.usable().some((e) => e.id === id)) source.setExt(id);
    rememberPlayFrom(mediaId(), "source");
  };
  /** In a room, the host's match on this source hasn't been taken on yet (see useRoomMatch). */
  const roomMatchPending = () => {
    const h = watchStore.hostSourceFor(mediaId());
    if (!h?.showId || source.ext() !== h.id) return false;
    const m = source.match();
    return !(m?.status === "matched" && m.sourceId === h.showId);
  };
  const item = () => source.byNumber().get(episode());
  const [localEps] = createResource(() => (local() ? mediaId() : null), localEpisodes);
  // Which stream to load, as a string: it changes only when the stream should (the copy, the
  // source, the episode). An object made afresh on every update of what it reads (the room's
  // state, every 2 s) counted as a new stream each time: it reloaded from 0:00 every 2 s.
  const streamKey = createMemo<string | null>((prev) => {
    let next: string | null;
    if (local()) next = `L\u0000${mediaId()}\u0000${episode()}`;
    // In a room, not before the show is the one the host matched on this source: loading this
    // device's own match first (another season, say) and then the host's started it twice. Only
    // for the first load: a stream already playing stays (that check flipping dropped it, and the
    // player loaded the same stream again every few seconds).
    else if (roomMatchPending() && !prev) next = null;
    else if (!item() || !source.ext()) next = prev?.startsWith("S") ? prev : null;
    else next = `S\u0000${source.ext()}\u0000${item()!.id}`;
    if (next !== prev) {
      const show = (k: string | null | undefined) => (k ? k.split("\u0000").join(" ") : "none");
      uiLog(`player: stream ${show(prev)} → ${show(next)} (local=${local()} pending=${roomMatchPending()})`);
    }
    return next;
  }, null);
  const [stream] = createResource<Stream, string>(streamKey, (k) => {
    const [kind, a, b] = k.split("\u0000");
    return kind === "L" ? localStream(Number(a), Number(b)) : getStream(a, b);
  });
  const [players] = createResource(externalPlayers);

  /** Episodes to switch to from the player: the library's files, or what the source lists. */
  const episodeChoices = createMemo(() => {
    if (local()) return (localEps() ?? []).map((number) => ({ number, title: undefined as string | undefined }));
    return [...source.byNumber().entries()].sort((a, b) => a[0] - b[0]).map(([number, i]) => ({ number, title: i.title ?? undefined }));
  });
  /** The episodes panel's search: digits match episode numbers that start with them (4 → 4, 40–49…). */
  const [epQuery, setEpQuery] = createSignal("");
  const shownChoices = createMemo(() => {
    const q = epQuery().trim();
    if (!q) return episodeChoices();
    if (/^\d+$/.test(q)) return episodeChoices().filter((e) => String(e.number).startsWith(q));
    const text = q.toLowerCase();
    return episodeChoices().filter((e) => e.title?.toLowerCase().includes(text));
  });
  /** Sub or Dub for the next episode: the language playing now when the source names it (it may
   *  have been switched in the Audio menu), else what this episode was opened with. */
  const nextDub = () => {
    const playing = stream()?.sources[sourceIndex()]?.audio;
    return playing === "en" ? true : playing === "ja" ? false : dubParam();
  };
  const has = (ep: number) => (local() ? !!localEps()?.includes(ep) : source.byNumber().has(ep));
  const hasNext = () => has(episode() + 1);
  const hasPrev = () => has(episode() - 1);
  const goTo = (ep: number) => {
    if (ep === episode()) return;
    stopForSwitch();
    go(
      local()
        ? `/watch/${mediaId()}/${ep}?local=1`
        : `/watch/${mediaId()}/${ep}?ext=${encodeURIComponent(source.ext() ?? "")}${nextDub() ? "&dub=1" : ""}`,
    );
  };
  /**
   * Switching episodes answers at once: the current one's place is saved, it stops, and the
   * spinner shows while the next one's video is looked up (which can take a few seconds).
   * Otherwise the old episode kept playing until then and the click looked missed.
   */
  const stopForSwitch = () => {
    persist();
    destroy();
    video.pause();
    video.removeAttribute("src");
    video.load();
    // Zeroed before the pause event's own save runs, so it can't file this position under the
    // next episode.
    setDuration(0);
    setTime(0);
    setBuffered(0);
    setPlaying(false);
    setVideoError(null);
    setWaiting(true);
    setTextCues({ top: [], bottom: [] });
    setPictures((p) => ({ ...p, images: [] }));
  };
  const back = () => leaveTo(navigate, `/show/${mediaId()}`);

  const failure = () => {
    if (page.error) return `Couldn't load the show: ${errorText(page.error)}`;
    if (local()) return stream.error ? errorText(stream.error) : null;
    if (source.exts() && !source.usable().length) return "No streaming extension is installed or turned on.";
    if (source.match.error) return errorText(source.match.error);
    if (source.match()?.status === "unmatched") return "This show isn't matched to a source yet. Pick one on the show page.";
    if (source.items.error) return errorText(source.items.error);
    if (source.items() && !item()) return `Episode ${episode()} isn't on this source.`;
    if (stream.error) return errorText(stream.error);
    return null;
  };

  // ── Video element and HLS
  let video!: HTMLVideoElement;
  let shell!: HTMLDivElement;
  let hls: Hls | null = null;
  /** A pending reload after a network error (see the stream effect). */
  let netRetry: number | undefined;
  const [sourceIndex, setSourceIndex] = createSignal(0);
  const [levels, setLevels] = createSignal<{ height: number; bitrate: number }[]>([]);
  const [level, setLevel] = createSignal(-1); // -1 = auto
  const [playing, setPlaying] = createSignal(false);
  const [time, setTime] = createSignal(0);
  const [duration, setDuration] = createSignal(0);
  const [buffered, setBuffered] = createSignal(0);
  const [volume, setVolume] = createSignal(stored("banime.volume", 1));
  const [muted, setMuted] = createSignal(false);
  const [waiting, setWaiting] = createSignal(true);
  // Watch together: in step with the room this device is in (nothing happens outside a room).
  const together = syncPlayer({
    video: () => video,
    mediaId,
    episode,
    title: () => media()?.title.userPreferred ?? "",
    dub: () => nextDub(),
    waiting,
    // The extension this episode plays from (offered to guests who can't play it); not a
    // library file, and not an extension installed from this computer.
    source: () => {
      if (local()) return null;
      const e = source.usable().find((x) => x.id === source.ext());
      // With the show it's matched to there: guests on the same source play that one too.
      const m = source.match();
      const show = m?.status === "matched" ? { showId: m.sourceId, showTitle: m.title } : {};
      return e && /^https:\/\//.test(e.sourceUrl) ? { id: e.id, name: e.name, url: e.sourceUrl, ...show } : null;
    },
  });
  /** Started per stream; cleared once the video has data (see the stream effect). */
  let stallTimer: number | undefined;
  onCleanup(() => clearTimeout(stallTimer));
  const [videoError, setVideoError] = createSignal<string | null>(null);
  const [tracks, setTracks] = createSignal<TextTrack[]>([]);
  const [activeTrack, setActiveTrack] = createSignal(-1);
  const [menu, setMenu] = createSignal<null | "subs" | "audio" | "quality" | "external" | "episodes" | "together">(null);
  // The episodes panel opens with an empty search each time.
  createEffect(on(menu, (m) => m !== "episodes" && setEpQuery("")));
  /** Subtitle files the user loaded for this episode. */
  const [extraSubs, setExtraSubs] = createSignal<{ url: string; label: string }[]>([]);
  const [subSize, setSubSizeSignal] = createSignal<"s" | "m" | "l">(
    (["s", "m", "l"] as const)[Math.max(0, Math.min(2, stored("banime.subsize", 1)))],
  );
  const setSubSize = (s: "s" | "m" | "l") => {
    setSubSizeSignal(s);
    store("banime.subsize", ["s", "m", "l"].indexOf(s));
  };
  /** How far above the bottom edge subtitles sit: low, middle (default) or high. */
  const [subPos, setSubPosSignal] = createSignal<"low" | "mid" | "high">(
    (["low", "mid", "high"] as const)[Math.max(0, Math.min(2, stored("banime.subpos", 1)))],
  );
  const setSubPos = (p: "low" | "mid" | "high") => {
    setSubPosSignal(p);
    store("banime.subpos", ["low", "mid", "high"].indexOf(p));
  };
  /** The subtitles menu's page: the main one, or the language list opened from it. */
  const [subPage, setSubPage] = createSignal<"main" | "lang">("main");
  // Each time the menu opens, it starts on its main page.
  createEffect(on(menu, (m) => m !== "subs" && setSubPage("main")));
  /** Subtitle timing correction in seconds (positive shows them later). */
  const [subOffset, setSubOffset] = createSignal(0);
  /** Alternate audio tracks inside an HLS stream (e.g. Japanese and English). */
  const [audioTracks, setAudioTracks] = createSignal<{ name: string; lang: string }[]>([]);
  const [audioTrack, setAudioTrack] = createSignal(-1);
  const [fullscreen, setFullscreen] = createSignal(false);
  const [skips, setSkips] = createSignal<SkipTime[]>([]);
  const [countdown, setCountdown] = createSignal<number | null>(null);
  /** Pointer over the timeline: where (0..1) and which time it points at. */
  const [hover, setHover] = createSignal<{ x: number; t: number } | null>(null);
  /** Dragging the timeline: where it's held (s). The video moves there only on release, so
   *  dragging across it doesn't load every spot on the way. */
  const [scrub, setScrub] = createSignal<number | null>(null);
  /** The time the bar and the clock show: where it's dragged to, else where it plays. */
  const shownTime = () => scrub() ?? time();
  /** The timeline is held (pressed, maybe dragged): it stays held wherever the pointer goes,
   *  until it's released, even outside the bar or the player. */
  let holding = false;
  /** The video goes where the bar was let go. */
  const commitScrub = () => {
    const t = scrub();
    setScrub(null);
    if (t !== null && together.mayMoveTimeline()) video.currentTime = t;
    // A finger lifted: its time tip goes (a mouse keeps it while it's over the bar).
    if (!matchMedia("(hover: hover)").matches) setHover(null);
  };
  onMount(() => {
    const release = () => {
      if (!holding) return;
      holding = false;
      commitScrub();
      wake();
    };
    window.addEventListener("pointerup", release);
    window.addEventListener("pointercancel", release);
    onCleanup(() => {
      window.removeEventListener("pointerup", release);
      window.removeEventListener("pointercancel", release);
    });
  });
  const pct = (t: number) => (duration() ? Math.min(100, Math.max(0, (t / duration()) * 100)) : 0);
  const sectionAt = (t: number) => skips().find((s) => t >= s.start && t < s.end);
  const onSeekHover = (e: PointerEvent) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    setHover({ x, t: x * duration() });
    if (duration()) preview.hover(x * duration());
  };

  const destroy = () => {
    clearTimeout(netRetry);
    hls?.destroy();
    hls = null;
  };
  onCleanup(destroy);

  /** Does a track's language match a preference code ("ja", "en", …)? */
  const langMatches = (lang: string, label: string, code: string) => {
    const names: Record<string, RegExp> = { ja: /^(ja|jp|jpn)\b|japanese/i, en: /^(en|eng)\b|english/i };
    const re = names[code] ?? new RegExp(`^${code}\\b`, "i");
    return re.test(lang) || re.test(label);
  };

  // Watching a streamed episode makes its Sub or Dub, and its source, this show's choice next
  // time (Home, show page, player).
  createEffect(
    on(stream, (s) => {
      if (!s || local()) return;
      rememberDub(mediaId(), dubParam());
      const ext = source.ext();
      if (ext) rememberShowSource(mediaId(), ext);
    }),
  );

  // Which server (stream) to start on: in the Sub/Dub this episode was opened with (?dub=1), the
  // server last picked for this show. See startSource for the details.
  createEffect(
    on(stream, (s) => {
      const all = (s?.sources ?? []).map((x, i) => ({ ...x, i }));
      const start = startSource(all, {
        picked: local() ? null : dubParam() ? "en" : "ja",
        preferred: prefs()?.audio,
        remembered: recallServer(mediaId()),
      });
      setSourceIndex(start?.i ?? 0);
      setExtraSubs([]);
      setSubOffset(0);
    }),
  );
  /** Picks a server by hand (Source / Audio menus) and remembers it for this show. */
  const chooseServer = (i: number) => {
    setSourceIndex(i);
    const src = stream()?.sources[i];
    if (src && !local()) rememberServer(mediaId(), serverName(src));
  };

  /** The stream last loaded, so a server switch (same stream, new index) is told from a new episode. */
  let loadedStream: ReturnType<typeof stream>;
  let loadedIdx = -1;
  /**
   * Mac: the stream's qualities, each its own playlist. WebKit plays the master playlist itself
   * and offers no choice, so a fixed quality plays that quality's playlist instead.
   */
  let nativeMaster = "";
  let nativeLevelUrls: string[] = [];
  const loadNativeLevels = async (master: string, s: ReturnType<typeof stream>, idx: number) => {
    try {
      const text = await (await fetch(master)).text();
      if (s !== stream() || idx !== sourceIndex()) return;
      const found = playlistLevels(text, master);
      if (!found) return;
      nativeMaster = master;
      nativeLevelUrls = found.map((l) => l.url);
      setLevels(found.map(({ height, bitrate }) => ({ height, bitrate })));
    } catch {
      // No list of qualities: Auto only.
    }
  };
  /** Bumped to load the same stream again from here (the Mac, back from a long while hidden). */
  const [reattach, setReattach] = createSignal(0);
  /** Whether the video was paused when it was reloaded, so it stays paused. */
  let reattachPaused = false;
  // Mac: after a long while behind other windows, WebKit can drop the video's picture (black,
  // sound and controls fine). Reload only the video, at the same second, when B-Anime is back.
  if (onWebKit) {
    let hiddenSince = 0;
    const onVisibility = () => {
      if (document.hidden) return void (hiddenSince = Date.now());
      const away = hiddenSince ? Date.now() - hiddenSince : 0;
      hiddenSince = 0;
      if (away < 60_000 || !video || !stream() || videoError() || video.currentTime <= 1) return;
      reattachPaused = video.paused;
      setReattach((n) => n + 1);
    };
    document.addEventListener("visibilitychange", onVisibility);
    onCleanup(() => document.removeEventListener("visibilitychange", onVisibility));
  }
  createEffect(
    on([stream, sourceIndex, reattach], async ([s, idx]) => {
      // Switching servers mid-episode continues from here, not from the saved position.
      const carry = s && s === loadedStream && video && video.currentTime > 1 ? video.currentTime : null;
      // The same stream and server loaded again (the Mac, back from the background): it keeps
      // its quality.
      const reload = !!s && s === loadedStream && idx === loadedIdx;
      const keepLevel = reload ? untrack(level) : -1;
      loadedStream = s;
      loadedIdx = idx;
      switchedAt = carry;
      destroy();
      setVideoError(null);
      if (!reload) {
        setLevels([]);
        setLevel(-1);
        nativeLevelUrls = [];
      }
      setAudioTracks([]);
      setAudioTrack(-1);
      setWaiting(true);
      clearTimeout(stallTimer);
      if (!s || !video) return;
      const src = s.sources[Math.min(idx, s.sources.length - 1)];
      // Watchdog: whatever goes wrong (a source that never answers, a stream nothing reports an
      // error for), the player ends with a message instead of loading forever.
      stallTimer = window.setTimeout(() => {
        if (s !== stream() || idx !== sourceIndex() || videoError() || video.readyState >= 2) return;
        destroy();
        setVideoError(
          `The video didn't start after ${STALL_SECONDS} seconds: the source may be down or blocking it.` +
            (s.sources.length > idx + 1 ? " Try another source." : ""),
        );
      }, STALL_SECONDS * 1000);
      // HLS goes through hls.js whenever Media Source Extensions exist: newer WebView2 builds
      // claim native HLS support but play proxied streams unreliably, and hls.js gives us levels.
      // Not on the Mac: WebKit plays HLS itself (as Safari does), and some streams that hls.js
      // repackages for it played with sound and no picture there (anamorphic 1440x1080 H.264).
      const HlsLib = src.kind === "hls" && !onWebKit ? (await import("hls.js")).default : null;
      if (src.kind === "hls" && !HlsLib?.isSupported() && !video.canPlayType("application/vnd.apple.mpegurl")) {
        return setVideoError("This system can't play HLS streams.");
      }
      // Where to start: the saved position, looked up before loading so streams begin there.
      // Seeking only once the video's details arrived raced hls.js's own start (at 0:00), and
      // auto-skip then jumped over an opening at the start instead of resuming.
      const startAt = carry ?? (await resumePoint());
      if (s !== stream() || idx !== sourceIndex()) return;
      if (HlsLib && HlsLib.isSupported()) {
        const instance = new HlsLib({
          enableWorker: true,
          // Phones have less room for buffered video; a full buffer stalled playback after a
          // quality change.
          backBufferLength: onAndroid ? 30 : 90,
          maxBufferLength: 60,
          startPosition: startAt ?? -1,
        });
        hls = instance;
        instance.on(HlsLib.Events.MANIFEST_PARSED, (_e, data) => {
          setLevels(data.levels.map((l) => ({ height: l.height, bitrate: l.bitrate })));
        });
        instance.on(HlsLib.Events.AUDIO_TRACKS_UPDATED, (_e, data) => {
          const list = data.audioTracks.map((t) => ({ name: t.name, lang: t.lang ?? "" }));
          setAudioTracks(list);
          // Switch to this show's audio, else the preferred language, when the stream carries it.
          const i = preferredAudio(list);
          if (i >= 0 && i !== instance.audioTrack) instance.audioTrack = i;
          setAudioTrack(instance.audioTrack);
        });
        instance.on(HlsLib.Events.AUDIO_TRACK_SWITCHED, (_e, data) => setAudioTrack(data.id));
        // Recovery attempts since the video last made progress. A slow connection may hiccup many
        // times in an episode; it gives up only when several tries in a row get nowhere. (One
        // count for the whole episode stopped playback after the third hiccup.)
        let mediaTries = 0;
        let netTries = 0;
        instance.on(HlsLib.Events.FRAG_BUFFERED, () => {
          mediaTries = 0;
          netTries = 0;
        });
        instance.on(HlsLib.Events.ERROR, (_e, data) => {
          if (!data.fatal) return;
          // The playlist itself didn't load (refused, missing, not a playlist). startLoad() can't
          // resume a stream whose playlist never arrived: it did nothing, no further error came,
          // and the spinner ran forever. Say what the server answered instead.
          const playlistFailed = [
            HlsLib.ErrorDetails.MANIFEST_LOAD_ERROR,
            HlsLib.ErrorDetails.MANIFEST_LOAD_TIMEOUT,
            HlsLib.ErrorDetails.MANIFEST_PARSING_ERROR,
          ].includes(data.details);
          const another = s.sources.length > idx + 1 ? " Try another source." : "";
          if (playlistFailed) {
            const status = data.response?.code;
            const why =
              status === 403 || status === 401
                ? `the source refused the video (HTTP ${status})`
                : status === 404 || status === 410
                  ? `the source no longer has this video (HTTP ${status})`
                  : status
                    ? `the source answered HTTP ${status}`
                    : "the video's playlist couldn't be loaded";
            return setVideoError(`Playback failed: ${why}.${another}`);
          }
          if (data.type === HlsLib.ErrorTypes.MEDIA_ERROR && mediaTries++ < 2) return instance.recoverMediaError();
          if (data.type === HlsLib.ErrorTypes.NETWORK_ERROR && netTries < 5) {
            // Waits longer each time (2, 4, 6… seconds), so a dropped connection has time to return.
            netTries++;
            clearTimeout(netRetry);
            netRetry = window.setTimeout(() => hls === instance && instance.startLoad(), netTries * 2000);
            return;
          }
          setVideoError(`Playback failed (${data.details}).${another}`);
        });
        instance.loadSource(src.url);
        instance.attachMedia(video);
      } else {
        video.src = (keepLevel >= 0 && nativeLevelUrls[keepLevel]) || src.url;
        if (src.kind === "hls" && !reload) void loadNativeLevels(src.url, s, idx);
      }
    }),
  );

  // ── Picture subtitles (PGS in library MKVs)
  // Their <track> carries empty cues (id = event index) for timing; the pictures for the active
  // cues are drawn in a layer sized like the video's picture.
  const pictureUrl = new WeakMap<TextTrack, string>();
  const pictureData = new Map<string, Promise<PictureTrack>>();
  const [aspect, setAspect] = createSignal(16 / 9);
  const [pictures, setPictures] = createSignal<{ width: number; height: number; images: PictureTrack["events"][number]["images"] }>(
    { width: 1920, height: 1080, images: [] },
  );
  const updatePictures = async () => {
    const t = tracks().find((x) => x.mode === "showing");
    const url = t && pictureUrl.get(t);
    if (!t || !url) return setPictures((p) => (p.images.length ? { ...p, images: [] } : p));
    let data = pictureData.get(url);
    if (!data) {
      data = fetch(url).then((r) => (r.ok ? (r.json() as Promise<PictureTrack>) : Promise.reject(new Error(`HTTP ${r.status}`))));
      data.catch(() => pictureData.delete(url));
      pictureData.set(url, data);
    }
    try {
      const track = await data;
      if (t.mode !== "showing") return;
      const images = Array.from(t.activeCues ?? []).flatMap((c) => track.events[Number(c.id)]?.images ?? []);
      setPictures({ width: track.width, height: track.height, images });
    } catch (e) {
      notify(`Couldn't load the subtitles: ${errorText(e)}`);
    }
  };
  createEffect(on(stream, () => setPictures((p) => ({ ...p, images: [] }))));

  // ── Text subtitles
  // Drawn by us rather than the browser (its own cues are hidden in app.css): the browser only
  // repositions a cue when the next one starts, so subtitles couldn't follow the controls or a
  // fullscreen switch. Ours sit in two stacks: top-placed cues (signs, `line:` near the top) stay
  // at the top, the rest stack at the bottom and glide up with the controls (CSS transform).
  const [textCues, setTextCues] = createSignal<{ top: VTTCue[]; bottom: VTTCue[] }>({ top: [], bottom: [] });
  const isTopCue = (c: VTTCue) =>
    c.line !== "auto" && (c.snapToLines ? c.line >= 0 : c.line < 50);
  function updateTextCues() {
    const t = tracks().find((x) => x.mode === "showing");
    const active = t && !pictureUrl.has(t) ? Array.from(t.activeCues ?? []) : [];
    // Exact duplicates showing together (some sources repeat a line, or add an outline copy)
    // are shown once.
    const seen = new Set<string>();
    const cues = active
      .filter((c): c is VTTCue => c instanceof VTTCue)
      .filter((c) => {
        const key = `${isTopCue(c) ? "t" : "b"}|${c.text.trim()}`;
        if (seen.has(key)) return false;
        seen.add(key);
        return true;
      });
    // Stable order: the line that started first sits at the bottom of its stack and keeps its
    // place; newer lines go above it instead of reshuffling everything.
    const byStart = (a: VTTCue, b: VTTCue) => b.startTime - a.startTime || a.endTime - b.endTime;
    let bottom = cues.filter((c) => !isTopCue(c));
    const signs = cues.filter(isTopCue);
    // Plain subtitles: three or more lines with exactly the same start and end are a block of
    // sign text (a slide or notice translated line by line); dialogue doesn't come in groups
    // like that. They go to the top.
    const timing = (c: VTTCue) => `${c.startTime}|${c.endTime}`;
    const groups = new Map<string, number>();
    for (const c of bottom) groups.set(timing(c), (groups.get(timing(c)) ?? 0) + 1);
    if ([...groups.values()].some((n) => n >= SIGN_GROUP_MIN)) {
      signs.push(...bottom.filter((c) => groups.get(timing(c))! >= SIGN_GROUP_MIN));
      bottom = bottom.filter((c) => groups.get(timing(c))! < SIGN_GROUP_MIN);
    }
    // Plain subtitles (no styles or positions) can't say what's a sign. When more lines are up
    // at once than dialogue ever needs, the longest-lasting ones are signs (a sign's translation
    // stays up for the whole shot; a spoken line lasts a few seconds): they go to the top.
    if (bottom.length > BOTTOM_CUES_MAX) {
      const length = (c: VTTCue) => c.endTime - c.startTime;
      const ranked = [...bottom].sort((a, b) => length(a) - length(b));
      const keep = new Set(ranked.slice(0, BOTTOM_CUES_MAX));
      signs.push(...bottom.filter((c) => !keep.has(c)));
      bottom = bottom.filter((c) => keep.has(c));
    }
    bottom.sort(byStart);
    // Signs, songs and notes (everything that isn't dialogue) at the top, newest first, at most
    // a few so they never cover the picture.
    const top = signs.sort((a, b) => b.startTime - a.startTime).slice(0, TOP_CUES_MAX);
    const same = (a: VTTCue[], b: VTTCue[]) => a.length === b.length && a.every((c, i) => c === b[i]);
    const prev = textCues();
    if (!same(prev.top, top) || !same(prev.bottom, bottom)) setTextCues({ top, bottom });
  }
  createEffect(on(stream, () => setTextCues({ top: [], bottom: [] })));
  /** One cue's text with its WebVTT markup (italics, bold, ruby…) as safe DOM nodes. */
  const CueText = (p: { cue: VTTCue }) => (
    <span class="sub-cue">
      <span class="sub-text" ref={(el) => el.append(p.cue.getCueAsHTML())} />
    </span>
  );

  // External subtitles are <track>s; HLS ones arrive as TextTracks too. Keep one list.
  let autoPicked = false;
  const refreshTracks = () => {
    const list = Array.from(video.textTracks).filter((t) => t.kind === "subtitles" || t.kind === "captions");
    setTracks(list);
    for (const t of list) {
      t.oncuechange = () => {
        applyOffset();
        updateTextCues();
        if (pictureUrl.has(t)) void updatePictures();
      };
    }
    applyOffset();
    const showing = list.findIndex((t) => t.mode === "showing");
    if (showing >= 0) setActiveTrack(showing);
    else if (!autoPicked && list.length) {
      // The preferred subtitle language (Settings → Playback), or in a Watch together room the
      // one its host chose; "off" shows none. Full subtitles beat forced / signs-only tracks in
      // the same language.
      const room = watchStore.room() ? watchStore.nowPlaying() : null;
      const roomSubs = room && room.mediaId === mediaId() && room.episode === episode() ? room.subs : null;
      const want = roomSubs ?? prefs()?.subtitles ?? "en";
      const matching = want === "off" ? [] : list.filter((t) => langMatches(t.language, t.label, want));
      const best = matching.find((t) => !/forced|signs|songs/i.test(t.label)) ?? matching[0];
      chooseTrack(best ? list.indexOf(best) : -1, false);
      autoPicked = !!best || want === "off";
    }
  };
  /** A track's menu name; tracks sharing a name are numbered so they can be told apart. */
  const trackName = (t: TextTrack, i: number) => {
    const base = t.label || t.language || `Track ${i + 1}`;
    const same = tracks().filter((x) => (x.label || x.language) === (t.label || t.language));
    return same.length > 1 ? `${base} (${same.indexOf(t) + 1})` : base;
  };
  /**
   * The chosen subtitle file is still downloading. Browsers fetch a <track> only once it's
   * switched on, so this covers the moment between choosing a language and its lines arriving.
   * (Subtitles inside an HLS stream have no <track> element and never show it.)
   */
  const [subLoading, setSubLoading] = createSignal(false);
  /** The list of subtitle languages hasn't arrived yet (the episode's stream is still loading). */
  const subListLoading = () => stream.loading || (!!stream() && duration() === 0 && tracks().length === 0);
  const checkSubLoading = () => {
    const t = tracks()[activeTrack()];
    const el = t && Array.from(video.querySelectorAll("track")).find((e) => e.track === t);
    // readyState: 0 not started, 1 loading, 2 loaded, 3 failed.
    setSubLoading(!!el && el.readyState < 2);
  };
  const chooseTrack = (i: number, close = true) => {
    tracks().forEach((t, j) => (t.mode = j === i ? "showing" : "disabled"));
    setActiveTrack(i);
    checkSubLoading();
    updateTextCues();
    void updatePictures();
    if (close) setMenu(null);
  };

  // Timing correction: every cue keeps its original times, shifted by the offset.
  const original = new WeakMap<TextTrackCue, [number, number]>();
  function applyOffset() {
    const offset = subOffset();
    for (const t of tracks()) {
      for (const cue of Array.from(t.cues ?? [])) {
        let o = original.get(cue);
        if (!o) original.set(cue, (o = [cue.startTime, cue.endTime]));
        const start = Math.max(0, o[0] + offset);
        if (cue.startTime !== start) {
          cue.startTime = start;
          cue.endTime = Math.max(start, o[1] + offset);
        }
      }
    }
  }
  createEffect(on(subOffset, applyOffset, { defer: true }));
  const nudgeSubs = (d: number) => {
    setSubOffset(Math.round((subOffset() + d) * 10) / 10);
    notify(`Subtitle timing ${subOffset() >= 0 ? "+" : ""}${subOffset().toFixed(1)} s`);
  };

  const loadSubtitleFile = async () => {
    setMenu(null);
    try {
      const picked = await pickSubtitleFile();
      if (!picked) return;
      setExtraSubs([...extraSubs(), picked]);
      // Show it as soon as the browser has parsed it.
      const label = picked.label;
      const pick = () => {
        const i = tracks().findIndex((t) => t.label === label);
        if (i >= 0) chooseTrack(i, false);
      };
      setTimeout(pick, 300);
      setTimeout(pick, 1200);
    } catch (e) {
      notify(`Couldn't load the subtitles: ${errorText(e)}`);
    }
  };

  // Audio tracks inside a file (dual-audio MKV/MP4). WebView2 exposes them with the
  // AudioVideoTracks feature, which the app turns on (tauri.conf.json).
  type NativeAudioTrack = { enabled: boolean; language: string; label: string };
  const nativeAudio = () => (video as unknown as { audioTracks?: ArrayLike<NativeAudioTrack> }).audioTracks;
  const readNativeAudio = () => {
    const list = nativeAudio();
    if (hls || !list || list.length < 2) return;
    const all = Array.from(list);
    const tracks = all.map((t) => ({ name: t.label, lang: t.language }));
    setAudioTracks(tracks);
    const i = preferredAudio(tracks);
    if (i >= 0) all.forEach((t, j) => (t.enabled = j === i));
    setAudioTrack(all.findIndex((t) => t.enabled));
  };
  /** The track this show was last listened in, else the one in the preferred language (-1: none). */
  const preferredAudio = (list: { name: string; lang: string }[]) => {
    const saved = recallAudio(mediaId());
    const mine = saved ? list.findIndex((t) => audioId(t) === saved) : -1;
    if (mine >= 0) return mine;
    const want = prefs()?.audio;
    return want ? list.findIndex((t) => langMatches(t.lang, t.name, want)) : -1;
  };

  const chooseAudio = (i: number) => {
    if (hls) hls.audioTrack = i;
    else {
      const list = nativeAudio();
      if (list) Array.from(list).forEach((t, j) => (t.enabled = j === i));
    }
    setAudioTrack(i);
    const t = audioTracks()[i];
    if (t) rememberAudio(mediaId(), audioId(t));
    setMenu(null);
  };
  /** A track's menu name: its own name unless that's generic ("stream_5"), else its language. */
  const audioLabel = (t: { name: string; lang: string }, i: number) => {
    const named = (x: { name: string; lang: string }) => {
      if (x.name && !/^(stream|track|audio)?[\s_-]*\d*$/i.test(x.name)) return x.name;
      if (x.lang && x.lang !== "und") {
        try {
          return new Intl.DisplayNames(["en"], { type: "language" }).of(x.lang) ?? x.lang;
        } catch {
          return x.lang;
        }
      }
      return "";
    };
    const base = named(t) || `Track ${i + 1}`;
    const same = audioTracks().filter((x) => named(x) === named(t));
    return same.length > 1 ? `${base} (${same.indexOf(t) + 1})` : base;
  };
  /** Sources that differ by audio language, for the Audio menu when the stream has no tracks. */
  /** Sources that name their audio language. */
  const withAudio = () => (stream()?.sources ?? []).map((s, i) => ({ ...s, i })).filter((s) => s.audio);
  /** One Audio menu entry per language (a source with several servers per language listed each). */
  const audioSources = () => withAudio().filter((s, k, all) => all.findIndex((x) => x.audio === s.audio) === k);
  /** Switches the audio language and keeps the server: the source at the same place in the new
   *  language's list as the current one is in its own (HD-1 sub → HD-1 dub). */
  const chooseAudioSource = (audio: string | null) => {
    const all = withAudio();
    const current = all.find((s) => s.i === sourceIndex());
    const pick = current ? sameServerIn(all, current, audio) : all.find((s) => s.audio === audio);
    if (!pick) return;
    chooseServer(pick.i);
    // Also this show's Sub/Dub from now on (show page, next episode).
    if (!local() && (audio === "ja" || audio === "en")) rememberDub(mediaId(), audio === "en");
  };
  const currentAudio = () => stream()?.sources[sourceIndex()]?.audio ?? null;
  const audioName = (code: string | null) => (code === "ja" ? "Japanese" : code === "en" ? "English" : code ?? "Unknown");

  // ── Resume, position saving, watched-at-85%, skip times
  let resumed = false;
  /** The saved position this episode resumed from (also keeps auto-skip off sections before it). */
  let resumeAt: number | null = null;
  /** The resume still has to be confirmed (and announced) once the video's details are in. */
  let resumePending = false;
  /** Where a server switch left off; the new server's video starts there. */
  let switchedAt: number | null = null;
  let marked = false;
  /** Seconds of this episode actually played (normal playback; jumps and drags add nothing).
   *  A resumed episode starts with what was played before the saved spot. */
  let played = 0;
  let lastTick: number | null = null;
  let lastSave = 0;
  /** The saved position to start this episode at, looked up once per episode. */
  const resumePoint = async () => {
    if (resumed || !inTauri) return null;
    resumed = true;
    // In a Watch together room nobody resumes their own spot: everyone starts where the room is.
    // A stream starts there itself (hls.js begins at this position: a jump made before it had
    // loaded was undone, back to 0:00); the room's updates keep it in step after that.
    if (watchStore.room()) {
      resumeAt = null;
      resumePending = false;
      const p = watchStore.nowPlaying();
      const at = watchStore.started() && p && p.mediaId === mediaId() && p.episode === episode() && !p.held ? watchStore.roomPosition() : 0;
      return at > 1 ? at : null;
    }
    const pos = await getPosition(mediaId(), episode()).catch(() => null);
    resumeAt = pos && pos > 30 ? pos : null;
    resumePending = resumeAt !== null;
    return resumeAt;
  };
  createEffect(on(() => `${mediaId()}:${episode()}`, () => {
    resumed = false;
    resumeAt = null;
    resumePending = false;
    marked = false;
    played = 0;
    lastTick = null;
    setSkips([]);
    skipsFor = "";
    setCountdown(null);
    setActiveTrack(-1);
  }));

  const persist = () => {
    if (!inTauri || !duration()) return Promise.resolve();
    return savePosition(mediaId(), episode(), video.currentTime, duration()).catch(() => {});
  };

  /** The episode whose skip times are in (or on their way), so a reload of the same video (a
   * quality or server switch, back from the background) doesn't ask AniSkip again. */
  let skipsFor = "";
  const onMeta = async () => {
    setDuration(video.duration || 0);
    video.volume = volume();
    if (switchedAt !== null) {
      // HLS already starts there; plain files seek now.
      const pos = switchedAt;
      switchedAt = null;
      if (Math.abs(video.currentTime - pos) > 2) video.currentTime = pos;
    } else if (resumePending && resumeAt !== null) {
      resumePending = false;
      const pos = resumeAt;
      if (pos < video.duration * 0.9) {
        // Streams already start there; plain files (and anything that drifted) seek now.
        if (Math.abs(video.currentTime - pos) > 2) video.currentTime = pos;
        notify(`Resumed at ${fmtClock(pos)}.`);
      } else {
        // Saved near the very end: start over instead.
        resumeAt = null;
        video.currentTime = 0;
      }
    }
    // Play first: the skip times come from AniSkip, and waiting for them left an episode paused
    // at the start (the next one after auto-next) whenever AniSkip was slow.
    if (reattachPaused) reattachPaused = false;
    else void video.play().catch(() => {});
    const mal = media()?.idMal;
    if (!mal || !inTauri) return;
    const ep = episode();
    const key = `${mal}:${ep}`;
    if (skipsFor === key) return;
    skipsFor = key;
    // A stream's length can still be unknown here (Infinity, with hls.js on Android): the lookup
    // can't send that, so it failed and the episode had no skip bars or buttons. It waits for the
    // real length (a few seconds at most), and asks without one if it never comes.
    const length = await knownDuration(10_000);
    if (ep !== episode()) return;
    setSkips(
      await getSkipTimes(mal, ep, length).catch(() => {
        // Ask again on the next load.
        if (skipsFor === key) skipsFor = "";
        return [];
      }),
    );
  };
  /** The video's length once it's known (a finite number), or 0 after `within` ms. */
  const knownDuration = (within: number) =>
    new Promise<number>((resolve) => {
      const known = () => Number.isFinite(video.duration) && video.duration > 0;
      if (known()) return resolve(video.duration);
      const done = () => {
        video.removeEventListener("durationchange", check);
        clearTimeout(timer);
        resolve(known() ? video.duration : 0);
      };
      const check = () => known() && done();
      const timer = window.setTimeout(done, within);
      video.addEventListener("durationchange", check);
    });

  /**
   * When the episode counts as watched (seconds): 85% of the way through, and when the episode's
   * ending is known, not before the ending starts either (both must be reached), so stopping
   * just short of the credits doesn't count. Auto-skipping the ending passes its start too.
   */
  const watchedAt = () => {
    const share = duration() * WATCHED_AT;
    const ending = skips().find((s) => s.kind === "ed" || s.kind === "mixed-ed");
    return ending ? Math.max(share, ending.start) : share;
  };

  const onTime = () => {
    const t = video.currentTime;
    setTime(t);
    // Playback moves a fraction of a second per update; a jump moves more, and isn't watching.
    if (lastTick !== null && !video.paused) {
      const step = t - lastTick;
      if (step > 0 && step < 2) played += step;
    }
    lastTick = t;
    const b = video.buffered;
    for (let i = 0; i < b.length; i++) if (b.start(i) <= t && t <= b.end(i)) setBuffered(b.end(i));
    if (performance.now() - lastSave > 10_000) {
      lastSave = performance.now();
      persist();
    }
    // Reached the end region, and watched most of it: dragging to the end doesn't count.
    if (!marked && duration() > 0 && t >= watchedAt() && played + (resumeAt ?? 0) >= duration() * PLAYED_AT) {
      marked = true;
      void markWatched();
    }
  };

  /** Episodes marked watched in this session (the loaded list entry doesn't refresh). */
  const [markedUpTo, setMarkedUpTo] = createSignal(0);
  const watchedUpTo = () => Math.max(page()?.listEntry?.progress ?? 0, markedUpTo());

  const markWatched = async () => {
    const entry = page()?.listEntry ?? null;
    if (!inTauri || (entry && entry.progress >= episode())) return;
    // Not signed in to AniList: there's no list to update (watching works the same; the
    // resume position is still kept on this device).
    if (!homeStore.account()?.viewer) return;
    const total = media()?.episodes ?? null;
    try {
      await saveEntry(mediaId(), applyRules(entry, { progress: episode() }, total));
      setMarkedUpTo((n) => Math.max(n, episode()));
      notify(`Episode ${episode()} marked watched on AniList.`);
    } catch (e) {
      notify(`Couldn't update AniList: ${errorText(e)}`);
    }
  };

  /** Skipping moves the timeline: in a running room only the host does it (everyone follows). */
  const canSkip = () => !(watchStore.room() && watchStore.started() && !watchStore.room()!.host);
  const activeSkip = () => (canSkip() ? skips().find((s) => time() >= s.start && time() < s.end - 1) : undefined);
  const skip = () => {
    const s = activeSkip();
    if (s) video.currentTime = s.end;
  };

  // ── Auto-skip: jump over each opening / ending / recap once as playback reaches it.
  // On unless the viewer turned it off.
  // ── Previews on the timeline: the frame where the pointer rests (PC and Mac).
  const [previews, setPreviewsSignal] = createSignal(stored("banime.seekpreview", 1) === 1);
  const setPreviews = (on: boolean) => {
    setPreviewsSignal(on);
    store("banime.seekpreview", on ? 1 : 0);
    if (!on) preview.reset();
  };
  const preview = createSeekPreview(() => {
    const s = stream()?.sources[Math.min(sourceIndex(), (stream()?.sources.length ?? 1) - 1)];
    return s ? { url: s.url, kind: s.kind } : null;
  }, previews);
  // Another episode or server: the kept frames are another video's.
  createEffect(on([stream, sourceIndex], () => preview.reset(), { defer: true }));

  const [autoSkip, setAutoSkipSignal] = createSignal(stored("banime.autoskip", 1) === 1);
  const setAutoSkip = (on: boolean) => {
    setAutoSkipSignal(on);
    store("banime.autoskip", on ? 1 : 0);
  };
  /** Sections already skipped (or seeked back into) this episode; never skipped twice. */
  let autoSkipped = new Set<number>();
  createEffect(on(skips, () => (autoSkipped = new Set())));
  createEffect(() => {
    const s = activeSkip();
    if (!s || !autoSkip() || !playing() || autoSkipped.has(s.start)) return;
    autoSkipped.add(s.start);
    // A section before the resume point was passed last time; never jump because of it.
    if (resumeAt !== null && s.start < resumeAt) return;
    video.currentTime = s.end;
    notify(`Skipped the ${(SECTION_NAME[s.kind] ?? "section").toLowerCase()}.`);
  });

  // ── Auto-next
  let tick: number | undefined;
  const onEnded = () => {
    persist();
    if (!hasNext()) return;
    setCountdown(AUTO_NEXT_SECONDS);
    tick = window.setInterval(() => {
      const n = (countdown() ?? 1) - 1;
      if (n <= 0) {
        clearInterval(tick);
        setCountdown(null);
        goTo(episode() + 1);
      } else setCountdown(n);
    }, 1000);
  };
  const cancelNext = () => {
    clearInterval(tick);
    setCountdown(null);
  };
  onCleanup(() => {
    clearInterval(tick);
    // Closing the player: your place goes to your other devices now (Google, when signed in),
    // not 20 s later, when a phone may already have put the app in the background.
    void persist().then(() => googleSync().catch(() => {}));
  });

  // ── Controls
  const toggle = () => (video.paused ? void video.play() : video.pause());
  const seekBy = (d: number) => {
    if (!together.mayMoveTimeline()) return;
    video.currentTime = Math.max(0, Math.min(duration(), video.currentTime + d));
  };

  // ── Taps on the picture (touch screens)
  // A tap shows the controls, or hides them if they're up: it never pauses, so the controls can
  // be used without stopping the episode (pausing is the big button in the middle). A quick
  // second tap on the right side jumps 10 s ahead, on the left 10 s back; more taps keep adding.
  // A tap on a side waits a moment for a second one; the middle answers at once.
  const TAP_SECONDS = 10;
  const DOUBLE_TAP_MS = 300;
  const [tapSeek, setTapSeek] = createSignal<{ side: "l" | "r"; total: number } | null>(null);
  let tapChain: { side: "l" | "r"; at: number } | null = null;
  let tapTimer: number | undefined;
  let tapHide: number | undefined;
  /** When the last touch ended: the click the browser makes from it is ignored (handled here). */
  let lastTouch = 0;
  onCleanup(() => {
    clearTimeout(tapTimer);
    clearTimeout(tapHide);
  });
  const onPictureTap = (e: PointerEvent) => {
    if (e.pointerType !== "touch") return;
    const now = performance.now();
    lastTouch = now;
    const box = video.getBoundingClientRect();
    const x = (e.clientX - box.left) / box.width;
    const side = x < 0.4 ? "l" : x > 0.6 ? "r" : null;
    if (side && tapChain?.side === side && now - tapChain.at < DOUBLE_TAP_MS * 1.5) {
      clearTimeout(tapTimer);
      tapChain = { side, at: now };
      const d = side === "r" ? TAP_SECONDS : -TAP_SECONDS;
      seekBy(d);
      const shown = tapSeek();
      setTapSeek({ side, total: (shown?.side === side ? shown.total : 0) + d });
      clearTimeout(tapHide);
      tapHide = window.setTimeout(() => setTapSeek(null), 700);
      return;
    }
    tapChain = side ? { side, at: now } : null;
    clearTimeout(tapTimer);
    const wereHidden = hiddenAtDown;
    tapTimer = window.setTimeout(() => {
      if (menu()) return setMenu(null);
      // Hidden: the pointer-down already brought them up. Showing: put them away (while playing).
      if (!wereHidden && playing()) {
        clearTimeout(idleTimer);
        setIdle(true);
      }
    }, side ? DOUBLE_TAP_MS : 0);
  };
  /** Whether the controls were hidden when the current touch began (it shows them at once). */
  let hiddenAtDown = false;
  /** A click the browser made from a touch: already handled by onPictureTap. */
  const fromTouch = () => performance.now() - lastTouch < 600;
  const setVol = (v: number) => {
    const clamped = Math.max(0, Math.min(1, v));
    video.volume = clamped;
    video.muted = clamped === 0;
    setVolume(clamped);
    setMuted(clamped === 0);
    store("banime.volume", clamped);
  };
  const toggleMute = () => {
    video.muted = !video.muted;
    setMuted(video.muted);
  };
  const chooseLevel = (i: number) => {
    if (hls) hls.currentLevel = i;
    else if (video && nativeMaster && (i === -1 || nativeLevelUrls[i])) {
      // Mac: play that quality's playlist (or the master, for Auto) from here.
      switchedAt = video.currentTime > 1 ? video.currentTime : null;
      reattachPaused = video.paused;
      setWaiting(true);
      video.src = i === -1 ? nativeMaster : nativeLevelUrls[i];
    }
    setLevel(i);
    setMenu(null);
  };
  /** Android: what the app's activity does for the player (MainActivity.kt). */
  const androidBars = () =>
    (window as unknown as {
      BAnimeAndroid?: { setImmersive(on: boolean): void; enterPip(w: number, h: number): boolean; setKeepScreenOn?(on: boolean): void };
    }).BAnimeAndroid;
  /** The phone's screen stays on while the episode plays (Android's WebView doesn't do it). */
  const keepScreenOn = (on: boolean) => androidBars()?.setKeepScreenOn?.(on);
  onCleanup(() => keepScreenOn(false));
  /** Android picture in picture: the whole app is the small window, so only the video shows. */
  const [pip, setPip] = createSignal(false);
  if (onAndroid) {
    const onPip = (e: Event) => {
      const state = (e as CustomEvent<string>).detail;
      setPip(state === "on");
      if (state === "on") setMenu(null);
      // The small window was dismissed: stop, as closing a player does.
      if (state === "closed") video.pause();
    };
    window.addEventListener("banime-pip", onPip);
    onCleanup(() => window.removeEventListener("banime-pip", onPip));
  }
  const toggleFullscreen = async () => {
    const next = !fullscreen();
    setFullscreen(next);
    if (onAndroid) {
      androidBars()?.setImmersive(next);
    } else if (inTauri) {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().setFullscreen(next).catch(() => {});
    } else if (next) void shell.requestFullscreen?.().catch(() => {});
    else if (document.fullscreenElement) void document.exitFullscreen();
  };
  onCleanup(() => {
    if (fullscreen() && onAndroid) androidBars()?.setImmersive(false);
    else if (fullscreen() && inTauri) void import("@tauri-apps/api/window").then(({ getCurrentWindow }) => getCurrentWindow().setFullscreen(false));
  });
  const togglePip = () => {
    // Android's WebView has no picture in picture of its own; the app's activity does it.
    if (onAndroid) return void androidBars()?.enterPip(video.videoWidth, video.videoHeight);
    if (document.pictureInPictureElement) void document.exitPictureInPicture();
    else void video.requestPictureInPicture?.().catch(() => {});
  };
  // Behind the app (a room's episode while browsing): the PC pops it out into a small window
  // that stays on top; back on its page, it comes back in. The phone keeps playing it unseen
  // (its picture in picture would shrink the whole app), with its sound.
  let poppedOut = false;
  createEffect(
    on(
      () => props.behind,
      (behind) => {
        if (behind) {
          if (fullscreen()) void toggleFullscreen();
          setMenu(null);
          if (!onAndroid && !document.pictureInPictureElement && video.readyState >= 1) {
            void video.requestPictureInPicture?.().then(() => (poppedOut = true)).catch(() => {});
          }
        } else if (poppedOut) {
          poppedOut = false;
          if (document.pictureInPictureElement === video) void document.exitPictureInPicture().catch(() => {});
        }
      },
      { defer: true },
    ),
  );
  onCleanup(() => {
    if (document.pictureInPictureElement === video) void document.exitPictureInPicture().catch(() => {});
  });
  // The small window's "back to tab" (or closing it) while the player is out of sight: the
  // player comes back to its page and the app to the front, still playing where the room is.
  let disposed = false;
  onCleanup(() => (disposed = true));
  const onLeavePip = () => {
    playerSlot.setInPip(false);
    // Out of sight (popped out by leaving the page, or by hand before leaving it).
    if (!props.behind || disposed) return;
    poppedOut = false;
    // The room's state, not the video's: the close button has paused it already (or will).
    const p = watchStore.nowPlaying();
    const roomPlaying =
      !!watchStore.room() && watchStore.started() && !!p?.playing && p.mediaId === mediaId() && p.episode === episode();
    navigate(target().path);
    if (inTauri && !onAndroid) {
      void import("@tauri-apps/api/window").then(async ({ getCurrentWindow }) => {
        const w = getCurrentWindow();
        await w.unminimize().catch(() => {});
        await w.show().catch(() => {});
        await w.setFocus().catch(() => {});
      });
    }
    // Closing the small window pauses its video; that wasn't meant for the room.
    if (roomPlaying) window.setTimeout(() => video.paused && void video.play().catch(() => {}), 100);
  };
  const onEnterPip = () => playerSlot.setInPip(true);
  onMount(() => {
    video.addEventListener("leavepictureinpicture", onLeavePip);
    video.addEventListener("enterpictureinpicture", onEnterPip);
    // The room bar's "Pop out", for when leaving the player couldn't (no click to allow it).
    playerSlot.setPopOut(() => {
      if (document.pictureInPictureElement || video.readyState < 1) return;
      void video.requestPictureInPicture?.().then(() => (poppedOut = true)).catch(() => {});
    });
    onCleanup(() => {
      video.removeEventListener("leavepictureinpicture", onLeavePip);
      video.removeEventListener("enterpictureinpicture", onEnterPip);
      playerSlot.setPopOut(null);
      playerSlot.setInPip(false);
    });
  });
  const openExternal = async (player: string) => {
    const s = stream();
    if (!s) return;
    const src = s.sources[Math.min(sourceIndex(), s.sources.length - 1)];
    const sub = activeTrack() >= 0 ? s.subtitles.find((x) => x.label === tracks()[activeTrack()]?.label)?.url : undefined;
    video.pause();
    setMenu(null);
    try {
      await openExternalPlayer(player, src.url, `${media()?.title.userPreferred ?? ""} · Episode ${episode()}`, sub, {
        mediaId: mediaId(),
        episode: episode(),
      });
      notify(
        player === "mpv"
          ? "Opened in MPV. Progress syncs to AniList when you're 85% through."
          : "Opened in VLC. Progress isn't tracked there.",
      );
    } catch (e) {
      notify(errorText(e));
    }
  };

  // Controls hide after a moment of stillness while playing.
  const [idle, setIdle] = createSignal(false);
  let idleTimer: number | undefined;
  /** A mouse is over the top bar or the controls: they stay up while it's there. */
  let overBars = false;
  const wake = () => {
    setIdle(false);
    clearTimeout(idleTimer);
    idleTimer = window.setTimeout(() => playing() && !menu() && !overBars && !holding && setIdle(true), 2600);
  };
  const barsEnter = (e: PointerEvent) => {
    if (e.pointerType !== "mouse") return;
    overBars = true;
    wake();
  };
  const barsLeave = (e: PointerEvent) => {
    if (e.pointerType !== "mouse" || holding) return;
    overBars = false;
    wake(); // the usual few seconds from here
  };
  onCleanup(() => clearTimeout(idleTimer));
  /**
   * Mouse movement while the controls are hidden: only a deliberate move brings them back
   * (40 px within 0.6 s), not a nudge or jitter (a mouse under a blanket). Clicks and keys still
   * show them at once, and while they show any movement keeps them up.
   */
  let moveFrom: { x: number; y: number; t: number } | null = null;
  const onPointerMove = (e: PointerEvent) => {
    if (!(idle() && playing() && !menu())) return wake();
    const now = performance.now();
    if (!moveFrom || now - moveFrom.t > 600) {
      moveFrom = { x: e.clientX, y: e.clientY, t: now };
      return;
    }
    if (Math.hypot(e.clientX - moveFrom.x, e.clientY - moveFrom.y) >= 40) {
      moveFrom = null;
      wake();
    }
  };


  const onKey = (e: KeyboardEvent) => {
    // Behind the app, the keys belong to the page that's open.
    if (props.behind) return;
    // Text fields and dropdowns keep their keys. Sliders don't: after clicking the timeline it
    // keeps focus, and its own arrow-key step (0.1 s) made arrows seem to do nothing.
    if ((e.target as HTMLElement)?.closest("input:not([type=range]), select, textarea")) return;
    const k = e.key.toLowerCase();
    const handled = (() => {
      switch (k) {
        case " ": case "k": toggle(); return true;
        case "arrowleft": seekBy(-5); return true;
        case "arrowright": seekBy(5); return true;
        case "j": seekBy(-10); return true;
        case "l": seekBy(10); return true;
        case "arrowup": setVol(volume() + 0.05); return true;
        case "arrowdown": setVol(volume() - 0.05); return true;
        case "m": toggleMute(); return true;
        case "f": void toggleFullscreen(); return true;
        case "e": setMenu(menu() === "episodes" ? null : "episodes"); return true;
        case "s": skip(); return true;
        case "c": chooseTrack(tracks().length ? (activeTrack() + 2) % (tracks().length + 1) - 1 : -1); return true;
        case "z": nudgeSubs(-0.5); return true;
        case "x": nudgeSubs(0.5); return true;
        case "n": if (hasNext()) goTo(episode() + 1); return true;
        case "p": if (hasPrev()) goTo(episode() - 1); return true;
        case "escape":
          if (menu()) setMenu(null);
          else if (fullscreen()) void toggleFullscreen();
          else back();
          return true;
      }
      if (/^[0-9]$/.test(k) && duration()) {
        if (together.mayMoveTimeline()) video.currentTime = (Number(k) / 10) * duration();
        return true;
      }
      return false;
    })();
    if (handled) {
      e.preventDefault();
      wake();
    }
  };
  onMount(() => {
    together.attach(video);
    window.addEventListener("keydown", onKey);
    video.textTracks.addEventListener("addtrack", refreshTracks);
    video.textTracks.addEventListener("removetrack", refreshTracks);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  const title = createMemo(() => media()?.title.userPreferred ?? "");
  const epTitle = () => item()?.title;
  const qualityLabel = (h: number) => (h ? `${h}p` : "Source");

  return (
    <div
      ref={shell}
      class="player"
      classList={{ "is-fullscreen": fullscreen(), "is-idle": idle() && playing() && !menu(), "is-pip": pip(), "is-behind": props.behind }}
      aria-hidden={props.behind ? "true" : undefined}
      data-subsize={subSize()}
      data-subpos={subPos()}
      onPointerMove={onPointerMove}
      onPointerDown={() => {
        hiddenAtDown = idle() && playing() && !menu();
        wake();
      }}
    >
      <video
        ref={video}
        class="player-video"
        playsinline
        crossorigin="anonymous"
        onLoadedMetadata={() => {
          if (video.videoWidth && video.videoHeight) setAspect(video.videoWidth / video.videoHeight);
          readNativeAudio();
          void onMeta();
        }}
        onTimeUpdate={onTime}
        onDurationChange={() => setDuration(video.duration || 0)}
        onPlay={() => { setPlaying(true); wake(); keepScreenOn(true); }}
        onPause={() => { setPlaying(false); setIdle(false); persist(); keepScreenOn(false); }}
        onWaiting={() => setWaiting(true)}
        // A jump while paused: Android may not say "can play" again until playing, so the spinner
        // (and the room's "loading") stayed. Ready once the picture there is.
        onSeeked={() => video.readyState >= 2 && video.paused && setWaiting(false)}
        onPlaying={() => { setWaiting(false); clearTimeout(stallTimer); }}
        onCanPlay={() => { setWaiting(false); clearTimeout(stallTimer); }}
        onEnded={onEnded}
        onError={() =>
          !hls &&
          setVideoError(
            local()
              ? "The built-in player can't play this file's format (common with MKV)."
              : "This video couldn't be played.",
          )
        }
        // With a menu or the episodes panel open, a click on the picture just closes it.
        onClick={() => !fromTouch() && (menu() ? setMenu(null) : toggle())}
        onDblClick={() => !fromTouch() && void toggleFullscreen()}
        onPointerUp={onPictureTap}
      >
        <For each={stream()?.subtitles ?? []}>
          {(s) => (
            <track
              ref={(el) => s.images && pictureUrl.set(el.track, s.url)}
              kind="subtitles"
              src={s.images ? `${s.url}?as=cues` : s.url}
              srclang={s.lang}
              label={s.label}
              onLoad={() => { refreshTracks(); checkSubLoading(); }}
              onError={checkSubLoading}
            />
          )}
        </For>
        <For each={extraSubs()}>
          {(s) => (
            <track
              kind="subtitles"
              src={s.url}
              label={s.label}
              onLoad={() => { refreshTracks(); checkSubLoading(); }}
              onError={checkSubLoading}
            />
          )}
        </For>
      </video>

      {/* Double-tap feedback: how far the taps jumped, on the side that was tapped. */}
      <Show when={tapSeek()}>
        {(t) => (
          <div class="tap-seek" data-side={t().side} aria-hidden="true">
            <Show when={t().side === "r"} fallback={<ChevronsLeft size={22} stroke-width={2} />}>
              <ChevronsRight size={22} stroke-width={2} />
            </Show>
            <span class="num">{t().total > 0 ? "+" : "−"}{Math.abs(t().total)} s</span>
          </div>
        )}
      </Show>

      <Show when={textCues().top.length || textCues().bottom.length}>
        <div class="sub-layer" aria-live="off">
          <div class="sub-stack sub-top">
            <For each={textCues().top}>{(c) => <CueText cue={c} />}</For>
          </div>
          <div class="sub-stack sub-bottom">
            <For each={textCues().bottom}>{(c) => <CueText cue={c} />}</For>
          </div>
        </div>
      </Show>

      <Show when={pictures().images.length}>
        <div class="pgs-layer" aria-hidden="true">
          <div class="pgs-screen" style={{ "aspect-ratio": String(aspect()), width: `min(100cqw, calc(100cqh * ${aspect()}))` }}>
            <For each={pictures().images}>
              {(p) => (
                <img
                  src={p.src}
                  alt=""
                  classList={{ "pgs-low": p.y > pictures().height / 2 }}
                  style={{
                    left: `${(p.x / pictures().width) * 100}%`,
                    top: `${(p.y / pictures().height) * 100}%`,
                    width: `${(p.w / pictures().width) * 100}%`,
                  }}
                />
              )}
            </For>
          </div>
        </div>
      </Show>

      <Show when={failure() ?? videoError()}>
        {(msg) => (
          <div class="player-message" role="alert">
            <p>{msg()}</p>
            <div class="setting-actions">
              <Show when={videoError() && (stream()?.sources.length ?? 0) > sourceIndex() + 1}>
                <button type="button" class="btn btn-primary" onClick={() => chooseServer(sourceIndex() + 1)}>Try next source</button>
              </Show>
              <For each={videoError() && stream() ? players() ?? [] : []}>
                {(p) => (
                  <button type="button" class="btn btn-primary" onClick={() => void openExternal(p)}>
                    Open in {p === "mpv" ? "MPV" : "VLC"}
                  </button>
                )}
              </For>
              <button type="button" class="btn" onClick={back}>Back to the show</button>
            </div>
          </div>
        )}
      </Show>
      {/* Loaded, but the room is held after a switch until everyone has loaded it. */}
      <Show
        when={
          !failure() && !videoError() && !waiting() && watchStore.room() && watchStore.nowPlaying()?.held &&
          watchStore.nowPlaying()?.mediaId === mediaId() && watchStore.nowPlaying()?.episode === episode()
        }
      >
        <p class="player-spinner-label player-held">
          <Show
            when={heldMismatch()}
            fallback={
              <>
                Waiting for everyone to load…{" "}
                <span class="num">
                  {watchStore.members().filter((m) => m.mediaId === mediaId() && m.episode === episode() && !m.buffering && watchStore.lengthGap(m) === null).length}/
                  {watchStore.members().length} ready
                </span>
              </>
            }
          >
            {(text) => <>{text()}</>}
          </Show>
          {/* This device has another copy: one tap to the host's source. */}
          <Show when={myGap() !== null && hostSrc()}>
            {(h) => (
              <button type="button" class="btn btn-sm btn-primary player-held-start" onClick={() => toHostSource(h().id)}>
                Play from {h().name}
              </button>
            )}
          </Show>
          {/* Only the host starts it without everyone: never by itself. */}
          <Show when={watchStore.room()?.host}>
            <button type="button" class="btn btn-sm player-held-start" onClick={() => together.startNow()}>
              Start without waiting
            </button>
          </Show>
        </p>
      </Show>
      <Show when={!failure() && !videoError() && waiting()}>
        <div class="player-spinner" aria-label="Loading" />
        {/* In a room, say what's loading: the room may just have switched show or episode. */}
        <Show when={watchStore.room() && watchStore.started()}>
          <p class="player-spinner-label">
            Loading {title() || "the episode"} · Episode {episode()} for the room…
          </p>
        </Show>
      </Show>
      {/* Touch screens: play and pause from the middle (a tap on the picture only shows the
          controls). Hidden with them, and while loading. */}
      <Show when={!failure() && !videoError() && !waiting()}>
        <button
          type="button"
          class="player-center"
          aria-label={playing() ? "Pause" : "Play"}
          onPointerUp={(e) => e.stopPropagation()}
          onClick={() => {
            toggle();
            wake();
          }}
        >
          <Show when={playing()} fallback={<Play size={34} stroke-width={1.75} fill="currentColor" aria-hidden="true" />}>
            <Pause size={34} stroke-width={1.75} fill="currentColor" aria-hidden="true" />
          </Show>
        </button>
      </Show>

      <header class="player-top" onPointerEnter={barsEnter} onPointerLeave={barsLeave}>
        <button type="button" class="back-link player-back" onClick={back} aria-label="Back">
          <ArrowLeft size={16} stroke-width={2} aria-hidden="true" />
          <span class="player-btn-label">Back</span>
        </button>
        <div class="player-title">
          <p class="player-show">{title()}</p>
          <p class="player-ep num">
            Episode {episode()}
            <Show when={epTitle() && !/^episode\s*\d+$/i.test(epTitle()!)}> · {epTitle()}</Show>
          </p>
        </div>
        <button
          type="button"
          class="btn btn-sm player-eps-btn"
          classList={{ "is-on": menu() === "episodes" }}
          aria-haspopup="dialog"
          aria-expanded={menu() === "episodes"}
          aria-label="Episodes"
          title="Choose another episode (E)"
          onClick={() => setMenu(menu() === "episodes" ? null : "episodes")}
        >
          <ListVideo size={16} stroke-width={1.75} aria-hidden="true" />
          <span class="player-btn-label">Episodes</span>
        </button>
        {/* In a room: a chip to the Together page (the room, the people, Ready and Start). */}
        <Show when={watchStore.room()}>
          <button
            type="button"
            class="btn btn-sm player-together-btn is-live"
            title={
              watchStore.reconnecting()
                ? "Connection to the room dropped; getting back in"
                : watchStore.started()
                  ? "Watching together"
                  : "In the lobby: the host starts the episode"
            }
            aria-haspopup="dialog"
            aria-expanded={menu() === "together"}
            // The room opens over the episode, which keeps playing (leaving for the Together page
            // would close the player).
            onClick={() => setMenu(menu() === "together" ? null : "together")}
          >
            <Users size={16} stroke-width={1.75} aria-hidden="true" />
            <span class="num">{watchStore.members().length}</span>
            <Show
              when={watchStore.reconnecting()}
              fallback={<Show when={!watchStore.started()}><span class="player-btn-label">· Lobby</span></Show>}
            >
              <span class="player-btn-label">· Reconnecting…</span>
            </Show>
          </button>
        </Show>
      </header>

      <Show when={menu() === "together"}>
        <aside class="player-drawer player-together" role="dialog" aria-label="Watch together">
          <Together embedded onClose={() => setMenu(null)} />
        </aside>
      </Show>

      <Show when={menu() === "episodes"}>
        <aside class="player-drawer" role="dialog" aria-label="Episodes">
          <div class="drawer-head">
            <div class="drawer-title">
              <p class="drawer-show">{title()}</p>
              <p class="drawer-meta num">
                {[FORMAT_NAME[media()?.format ?? ""] ?? media()?.format, media()?.seasonYear, media()?.episodes ? `${media()!.episodes} episodes` : null]
                  .filter(Boolean)
                  .join(" · ")}
              </p>
            </div>
            <button type="button" class="icon-btn" aria-label="Close" onClick={() => setMenu(null)}>
              <X size={18} stroke-width={1.75} aria-hidden="true" />
            </button>
          </div>
          <button type="button" class="btn btn-sm drawer-details" onClick={back}>Show details</button>
          <Show when={episodeChoices().length > 12}>
            <form
              class="drawer-find"
              onSubmit={(e) => {
                e.preventDefault();
                // Enter plays the episode with exactly that number, if there is one.
                const n = Number(epQuery());
                if (episodeChoices().some((x) => x.number === n)) {
                  setMenu(null);
                  if (n !== episode()) goTo(n);
                }
              }}
            >
              <label class="search">
                <Search size={14} stroke-width={1.75} aria-hidden="true" />
                <span class="sr-only">Find an episode by number</span>
                <input
                  type="search"
                  inputmode="numeric"
                  placeholder="Episode number"
                  value={epQuery()}
                  onInput={(e) => setEpQuery(e.currentTarget.value)}
                />
              </label>
            </form>
          </Show>
          <Show
            when={episodeChoices().length}
            fallback={<p class="drawer-empty">{local() ? "No other episodes in your library." : "Episodes are still loading from the source…"}</p>}
          >
            <Show when={!shownChoices().length}>
              <p class="drawer-empty">No episode {epQuery().trim()}.</p>
            </Show>
            <ol class="drawer-eps">
              <For each={shownChoices()}>
                {(e) => {
                  const current = () => e.number === episode();
                  const watched = () => e.number <= watchedUpTo();
                  return (
                    <li>
                      <button
                        type="button"
                        class="drawer-ep"
                        aria-current={current() ? "true" : undefined}
                        data-watched={watched() ? "" : undefined}
                        ref={(el) => current() && !epQuery() && queueMicrotask(() => el.scrollIntoView({ block: "center" }))}
                        onClick={() => {
                          setMenu(null);
                          if (!current()) goTo(e.number);
                        }}
                      >
                        <span class="drawer-ep-num num">{e.number}</span>
                        <span class="drawer-ep-title">{e.title || `Episode ${e.number}`}</span>
                        <span class="drawer-ep-state">{current() ? "Playing" : watched() ? "Watched" : ""}</span>
                      </button>
                    </li>
                  );
                }}
              </For>
            </ol>
          </Show>
        </aside>
      </Show>

      <Show when={activeSkip()}>
        {(s) => (
          <button type="button" class="skip-btn" onClick={skip}>
            {SKIP_LABEL[s().kind] ?? "Skip"} <kbd>S</kbd>
          </button>
        )}
      </Show>

      <Show when={countdown() !== null}>
        <div class="next-card" role="status">
          <p class="next-label">Next: Episode {episode() + 1}</p>
          <p class="next-count num">Playing in {countdown()} s</p>
          <div class="setting-actions">
            <button type="button" class="btn btn-primary" onClick={() => { cancelNext(); goTo(episode() + 1); }}>Play now</button>
            <button type="button" class="btn" onClick={cancelNext}>Cancel</button>
          </div>
        </div>
      </Show>

      <div class="player-controls" onPointerDown={(e) => e.stopPropagation()} onPointerEnter={barsEnter} onPointerLeave={barsLeave}>
        <div class="seek" onPointerMove={onSeekHover} onPointerLeave={() => { if (holding) return; setHover(null); preview.leave(); }}>
          <div class="seek-track" aria-hidden="true">
            <div class="seek-buffered" style={{ width: `${pct(buffered())}%` }} />
            {/* Openings, endings and recaps: amber until played, then red like the rest. */}
            <For each={skips()}>
              {(s) => <div class="seek-chapter" style={{ left: `${pct(s.start)}%`, width: `${pct(s.end) - pct(s.start)}%` }} />}
            </For>
            <div class="seek-played" style={{ width: `${pct(shownTime())}%` }} />
            <For each={skips()}>
              {(s) => (
                <>
                  <i class="seek-cut" style={{ left: `${pct(s.start)}%` }} />
                  <i class="seek-cut" style={{ left: `${pct(s.end)}%` }} />
                </>
              )}
            </For>
          </div>
          <Show when={hover() && duration() ? hover() : null}>
            {(h) => (
              <span
                class="seek-tip num"
                classList={{ "has-frame": preview.active() }}
                style={{ left: `clamp(${preview.active() ? 104 : 40}px, ${h().x * 100}%, calc(100% - ${preview.active() ? 104 : 40}px))` }}
              >
                {/* The box shows at once; the frame fills it when it's in (loading meanwhile). */}
                <Show when={preview.active()}>
                  <Show
                    when={preview.frame()}
                    fallback={
                      <span class="seek-frame seek-frame-wait" style={{ "aspect-ratio": String(preview.aspect()) }} role="status" aria-label="Loading the preview">
                        <i class="spin-dot" aria-hidden="true" />
                      </span>
                    }
                  >
                    {(f) => <img class="seek-frame" src={f().src} alt="" style={{ "aspect-ratio": String(preview.aspect()) }} />}
                  </Show>
                </Show>
                <span class="seek-tip-time">
                  <Show when={sectionAt(h().t)}>{(s) => <b>{SECTION_NAME[s().kind] ?? "Section"}</b>}</Show>
                  {fmtClock(h().t)}
                </span>
              </span>
            )}
          </Show>
          <input
            type="range"
            class="seek-input"
            min="0"
            max={duration() || 0}
            step="0.1"
            value={shownTime()}
            aria-label="Seek"
            aria-valuetext={`${fmtClock(shownTime())} of ${fmtClock(duration())}`}
            // While held: only the bar, the time and the preview follow. On release the video goes
            // there (change fires once, on release; and per press with the arrow keys).
            onInput={(e) => {
              if (!together.mayMoveTimeline()) {
                e.currentTarget.value = String(video.currentTime);
                return;
              }
              const t = Number(e.currentTarget.value);
              setScrub(t);
              if (duration()) {
                setHover({ x: t / duration(), t });
                preview.hover(t);
              }
            }}
            // Pressed: the bar keeps the pointer until it's released (see commitScrub).
            onPointerDown={(e) => {
              holding = true;
              e.currentTarget.setPointerCapture?.(e.pointerId);
            }}
            onChange={(e) => {
              // Released (or an arrow key): the video goes there, once (the release itself may
              // have taken it there already).
              if (scrub() === null) {
                const t = Number(e.currentTarget.value);
                if (Math.abs(t - video.currentTime) < 0.25) return;
                setScrub(t);
              }
              holding = false;
              commitScrub();
            }}
          />
        </div>

        <div class="player-bar">
          <div class="bar-group">
            <button type="button" class="icon-btn" onClick={toggle} aria-label={playing() ? "Pause (K)" : "Play (K)"}>
              <Show when={playing()} fallback={<Play size={20} fill="currentColor" stroke-width={0} />}>
                <Pause size={20} fill="currentColor" stroke-width={0} />
              </Show>
            </button>
            <button type="button" class="icon-btn ep-prev" disabled={!hasPrev()} onClick={() => goTo(episode() - 1)} aria-label="Previous episode (P)">
              <SkipBack size={18} fill="currentColor" stroke-width={0} />
            </button>
            <button type="button" class="icon-btn" disabled={!hasNext()} onClick={() => goTo(episode() + 1)} aria-label="Next episode (N)">
              <SkipForward size={18} fill="currentColor" stroke-width={0} />
            </button>
            <div class="volume">
              <button type="button" class="icon-btn" onClick={toggleMute} aria-label={muted() ? "Unmute (M)" : "Mute (M)"}>
                <Show when={muted() || volume() === 0} fallback={<Volume2 size={18} stroke-width={2} />}>
                  <VolumeX size={18} stroke-width={2} />
                </Show>
              </button>
              <input
                type="range"
                class="volume-input"
                min="0"
                max="1"
                step="0.01"
                value={muted() ? 0 : volume()}
                aria-label="Volume"
                onInput={(e) => setVol(Number(e.currentTarget.value))}
              />
            </div>
            <span class="player-time num">
              {fmtClock(shownTime())} / {fmtClock(duration())}
            </span>
          </div>

          <div class="bar-group">
            <Show when={audioTracks().length > 1 || audioSources().length > 1}>
              <div class="menu-wrap">
                <button type="button" class="icon-btn" aria-haspopup="menu" aria-expanded={menu() === "audio"} onClick={() => setMenu(menu() === "audio" ? null : "audio")} aria-label="Audio language">
                  <AudioLines size={20} stroke-width={2} />
                </button>
                <Show when={menu() === "audio"}>
                  <div class="menu" role="menu">
                    <p class="menu-head">Audio</p>
                    <Show
                      when={audioTracks().length > 1}
                      fallback={
                        <For each={audioSources()}>
                          {(s) => (
                            <button type="button" role="menuitemradio" aria-checked={currentAudio() === s.audio} onClick={() => { chooseAudioSource(s.audio); setMenu(null); }}>
                              {audioName(s.audio)}
                            </button>
                          )}
                        </For>
                      }
                    >
                      <For each={audioTracks()}>
                        {(t, i) => (
                          <button type="button" role="menuitemradio" aria-checked={audioTrack() === i()} onClick={() => chooseAudio(i())}>
                            {audioLabel(t, i())}
                          </button>
                        )}
                      </For>
                    </Show>
                  </div>
                </Show>
              </div>
            </Show>

            <div class="menu-wrap">
              <button type="button" class="icon-btn" classList={{ "is-on": activeTrack() >= 0 }} aria-haspopup="menu" aria-expanded={menu() === "subs"} onClick={() => setMenu(menu() === "subs" ? null : "subs")} aria-label="Subtitles (C)">
                <Captions size={20} stroke-width={2} />
              </button>
              <Show when={menu() === "subs"}>
                <div class="menu menu-wide menu-paged" role="menu">
                  {/* One menu, two pages: the main page has a Language row (showing the current
                      track) that opens the (often long) language list in place, with a way back. */}
                  <Show
                    when={subPage() === "main"}
                    fallback={
                      <>
                        <button type="button" class="menu-back" onClick={() => setSubPage("main")}>
                          <ChevronLeft size={16} stroke-width={2} aria-hidden="true" /> Language
                        </button>
                        <div class="menu-scroll" aria-label="Subtitle language">
                          <button type="button" role="menuitemradio" aria-checked={activeTrack() === -1} onClick={() => chooseTrack(-1)}>Off</button>
                          <Show when={subListLoading()}>
                            <p class="menu-loading menu-loading-row" role="status">
                              <i class="spin-dot" aria-hidden="true" /> Loading subtitles…
                            </p>
                          </Show>
                          <For each={tracks()}>
                            {(t, i) => (
                              <button type="button" role="menuitemradio" aria-checked={activeTrack() === i()} onClick={() => chooseTrack(i(), false)}>
                                {trackName(t, i())}
                                <Show when={activeTrack() === i() && subLoading()}>
                                  <span class="menu-loading" role="status">
                                    <i class="spin-dot" aria-hidden="true" /> Loading…
                                  </span>
                                </Show>
                              </button>
                            )}
                          </For>
                        </div>
                        <Show when={inTauri}>
                          <button type="button" role="menuitem" class="menu-action" onClick={() => void loadSubtitleFile()}>
                            Load from file…
                          </button>
                        </Show>
                      </>
                    }
                  >
                  <p class="menu-head">Subtitles</p>
                  <button type="button" role="menuitem" class="menu-next" aria-haspopup="menu" onClick={() => setSubPage("lang")}>
                    <span>Language</span>
                    <span class="menu-next-value">
                      <Show
                        when={!subLoading() && !(subListLoading() && activeTrack() < 0)}
                        fallback={<><i class="spin-dot" aria-hidden="true" /> Loading…</>}
                      >
                        {activeTrack() >= 0 && tracks()[activeTrack()] ? trackName(tracks()[activeTrack()], activeTrack()) : "Off"}
                      </Show>
                      <ChevronRight size={16} stroke-width={2} aria-hidden="true" />
                    </span>
                  </button>

                  <p class="menu-head">Size</p>
                  <div class="menu-row" role="group" aria-label="Subtitle size">
                    <For each={[["s", "Small"], ["m", "Medium"], ["l", "Large"]] as const}>
                      {([v, label]) => (
                        <button type="button" aria-pressed={subSize() === v} onClick={() => setSubSize(v)}>{label}</button>
                      )}
                    </For>
                  </div>

                  <p class="menu-head">Position</p>
                  <div class="menu-row" role="group" aria-label="Subtitle position">
                    <For each={[["low", "Low"], ["mid", "Middle"], ["high", "High"]] as const}>
                      {([v, label]) => (
                        <button type="button" aria-pressed={subPos() === v} onClick={() => setSubPos(v)}>{label}</button>
                      )}
                    </For>
                  </div>

                  <p class="menu-head">Timing <span class="menu-hint">Z / X</span></p>
                  <div class="menu-row" role="group" aria-label="Subtitle timing">
                    <button type="button" onClick={() => nudgeSubs(-0.5)} aria-label="Show subtitles 0.5 seconds earlier">−0.5 s</button>
                    <button type="button" class="menu-value num" onClick={() => setSubOffset(0)} title="Reset">
                      {subOffset() > 0 ? "+" : ""}{subOffset().toFixed(1)} s
                    </button>
                    <button type="button" onClick={() => nudgeSubs(0.5)} aria-label="Show subtitles 0.5 seconds later">+0.5 s</button>
                  </div>
                  </Show>
                </div>
              </Show>
            </div>

            <div class="menu-wrap">
                <button type="button" class="icon-btn" aria-haspopup="menu" aria-expanded={menu() === "quality"} onClick={() => setMenu(menu() === "quality" ? null : "quality")} aria-label="Settings">
                  <Settings2 size={20} stroke-width={2} />
                </button>
                <Show when={menu() === "quality"}>
                  <div class="menu" role="menu">
                    <button
                      type="button"
                      role="menuitemcheckbox"
                      class="menu-toggle"
                      aria-checked={autoSkip()}
                      onClick={() => setAutoSkip(!autoSkip())}
                    >
                      <span>
                        Auto-skip
                        <span class="menu-sub">{canSkip() ? "Openings, endings and recaps" : "The host skips for the room"}</span>
                      </span>
                      <span class="menu-switch" aria-hidden="true" />
                    </button>
                    <Show when={!onAndroid}>
                      <button
                        type="button"
                        role="menuitemcheckbox"
                        class="menu-toggle"
                        aria-checked={previews()}
                        onClick={() => setPreviews(!previews())}
                      >
                        <span>
                          Timeline previews
                          <span class="menu-sub">{local() ? "A frame where you point" : "Loads a small piece where you point"}</span>
                        </span>
                        <span class="menu-switch" aria-hidden="true" />
                      </button>
                    </Show>
                    {/* Phones: here rather than its own button, so the control bar fits held upright. */}
                    <Show when={onAndroid}>
                      <button type="button" role="menuitem" onClick={() => { setMenu(null); togglePip(); }}>
                        Picture in picture
                      </button>
                    </Show>
                    <Show when={levels().length > 1}>
                      <p class="menu-head">Quality</p>
                      <button type="button" role="menuitemradio" aria-checked={level() === -1} onClick={() => chooseLevel(-1)}>Auto</button>
                      <For each={levels().map((l, i) => ({ ...l, i })).sort((a, b) => b.height - a.height)}>
                        {(l) => (
                          <button type="button" role="menuitemradio" aria-checked={level() === l.i} onClick={() => chooseLevel(l.i)}>
                            {qualityLabel(l.height)}
                          </button>
                        )}
                      </For>
                    </Show>
                    <Show when={(stream()?.sources.length ?? 0) > 1}>
                      <p class="menu-head">Source</p>
                      <For each={stream()!.sources}>
                        {(s, i) => (
                          <button type="button" role="menuitemradio" aria-checked={sourceIndex() === i()} onClick={() => { chooseServer(i()); setMenu(null); }}>
                            {s.label}
                          </button>
                        )}
                      </For>
                    </Show>
                  </div>
                </Show>
            </div>

            <Show when={players()?.length}>
              <div class="menu-wrap">
                <button type="button" class="icon-btn" aria-haspopup="menu" aria-expanded={menu() === "external"} onClick={() => setMenu(menu() === "external" ? null : "external")} aria-label="Open in another player">
                  <ExternalLink size={18} stroke-width={2} />
                </button>
                <Show when={menu() === "external"}>
                  <div class="menu" role="menu">
                    <p class="menu-head">Open in</p>
                    <For each={players()}>
                      {(p) => <button type="button" role="menuitem" onClick={() => void openExternal(p)}>{p === "mpv" ? "MPV" : "VLC"}</button>}
                    </For>
                  </div>
                </Show>
              </div>
            </Show>

            {/* Android: in the settings menu instead (the bar must fit a phone held upright). */}
            <Show when={!onAndroid}>
              <button type="button" class="icon-btn" onClick={togglePip} aria-label="Picture in picture">
                <PictureInPicture size={18} stroke-width={2} />
              </button>
            </Show>
            <button type="button" class="icon-btn" onClick={() => void toggleFullscreen()} aria-label={fullscreen() ? "Exit full screen (F)" : "Full screen (F)"}>
              <Show when={fullscreen()} fallback={<Maximize size={18} stroke-width={2} />}>
                <Minimize size={18} stroke-width={2} />
              </Show>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

/**
 * A master playlist's qualities (one per height, its highest bitrate), with each one's playlist,
 * or null when there's no choice to offer: fewer than two, or audio in its own playlists (a
 * quality's playlist alone would play without sound).
 */
function playlistLevels(text: string, base: string): { height: number; bitrate: number; url: string }[] | null {
  if (/#EXT-X-MEDIA:[^\n]*TYPE=AUDIO[^\n]*URI=/.test(text)) return null;
  const lines = text.split(/\r?\n/);
  const byHeight = new Map<number, { height: number; bitrate: number; url: string }>();
  lines.forEach((line, i) => {
    if (!line.startsWith("#EXT-X-STREAM-INF:")) return;
    const res = /RESOLUTION=\d+x(\d+)/.exec(line);
    const uri = lines.slice(i + 1).find((l) => l.trim() && !l.startsWith("#"));
    if (!res || !uri) return;
    const height = Number(res[1]);
    const bitrate = Number(/[:,]BANDWIDTH=(\d+)/.exec(line)?.[1] ?? 0);
    const prev = byHeight.get(height);
    if (!prev || bitrate > prev.bitrate) byHeight.set(height, { height, bitrate, url: new URL(uri.trim(), base).href });
  });
  return byHeight.size > 1 ? [...byHeight.values()] : null;
}

