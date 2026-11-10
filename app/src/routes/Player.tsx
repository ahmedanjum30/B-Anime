import { For, Show, createEffect, createMemo, createResource, createSignal, on, onCleanup, onMount } from "solid-js";
import { useNavigate, useParams, useSearchParams } from "@solidjs/router";
import type Hls from "hls.js";

import {
  ArrowLeft, AudioLines, Captions, ExternalLink, Maximize, Minimize, Pause, PictureInPicture, Play, Settings2,
  SkipBack, SkipForward, Volume2, VolumeX,
} from "../lib/icons";
import {
  errorText, externalPlayers, getMedia, getPlayerPrefs, getPosition, getSkipTimes, getStream, inTauri, localEpisodes,
  localStream, openExternalPlayer, pickSubtitleFile, saveEntry, savePosition,
} from "../lib/api";
import type { Stream } from "../lib/types";
import { applyRules } from "../lib/entry";
import { fmtClock } from "../lib/format";
import { createSource } from "../lib/source";
import { homeStore } from "../lib/store";
import type { SkipTime } from "../lib/types";

const WATCHED_AT = 0.8; // fraction of the episode that counts as watched
const AUTO_NEXT_SECONDS = 8;
const SKIP_LABEL: Record<string, string> = {
  op: "Skip intro",
  "mixed-op": "Skip intro",
  ed: "Skip outro",
  "mixed-ed": "Skip outro",
  recap: "Skip recap",
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

export default function Player() {
  const params = useParams();
  const [search] = useSearchParams();
  const navigate = useNavigate();
  const { notify } = homeStore;

  const mediaId = () => Number(params.id);
  const episode = () => Number(params.ep);
  const extParam = () => (typeof search.ext === "string" ? search.ext : undefined);

  /** Playing a file from the library instead of an extension's stream. */
  const local = () => search.local === "1";
  /** The English-dub version of the source. */
  const dubParam = () => search.dub === "1";

  const [page] = createResource(mediaId, getMedia);
  const media = () => page()?.media;
  const [prefs] = createResource(getPlayerPrefs);
  const source = createSource(() => (local() ? undefined : media()), extParam, dubParam);
  const item = () => source.byNumber().get(episode());
  const [localEps] = createResource(() => (local() ? mediaId() : null), localEpisodes);
  type Key = { local: true; id: number; ep: number } | { local: false; ext: string; id: string };
  const [stream] = createResource<Stream, Key>(
    (): Key | null => {
      if (local()) return { local: true, id: mediaId(), ep: episode() };
      return item() && source.ext() ? { local: false, ext: source.ext()!, id: item()!.id } : null;
    },
    (k) => (k.local ? localStream(k.id, k.ep) : getStream(k.ext, k.id)),
  );
  const [players] = createResource(externalPlayers);

  const has = (ep: number) => (local() ? !!localEps()?.includes(ep) : source.byNumber().has(ep));
  const hasNext = () => has(episode() + 1);
  const hasPrev = () => has(episode() - 1);
  const goTo = (ep: number) =>
    navigate(
      local()
        ? `/watch/${mediaId()}/${ep}?local=1`
        : `/watch/${mediaId()}/${ep}?ext=${encodeURIComponent(source.ext() ?? "")}${dubParam() ? "&dub=1" : ""}`,
      { replace: true },
    );
  const back = () => navigate(`/show/${mediaId()}`);

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
  const [videoError, setVideoError] = createSignal<string | null>(null);
  const [tracks, setTracks] = createSignal<TextTrack[]>([]);
  const [activeTrack, setActiveTrack] = createSignal(-1);
  const [menu, setMenu] = createSignal<null | "subs" | "audio" | "quality" | "external">(null);
  /** Subtitle files the user loaded for this episode. */
  const [extraSubs, setExtraSubs] = createSignal<{ url: string; label: string }[]>([]);
  const [subSize, setSubSizeSignal] = createSignal<"s" | "m" | "l">(
    (["s", "m", "l"] as const)[Math.max(0, Math.min(2, stored("banime.subsize", 1)))],
  );
  const setSubSize = (s: "s" | "m" | "l") => {
    setSubSizeSignal(s);
    store("banime.subsize", ["s", "m", "l"].indexOf(s));
  };
  /** Subtitle timing correction in seconds (positive shows them later). */
  const [subOffset, setSubOffset] = createSignal(0);
  /** Alternate audio tracks inside an HLS stream (e.g. Japanese and English). */
  const [audioTracks, setAudioTracks] = createSignal<{ name: string; lang: string }[]>([]);
  const [audioTrack, setAudioTrack] = createSignal(-1);
  const [fullscreen, setFullscreen] = createSignal(false);
  const [skips, setSkips] = createSignal<SkipTime[]>([]);
  const [countdown, setCountdown] = createSignal<number | null>(null);

  const destroy = () => {
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

  // Several sources with known audio: start on the one in the preferred language.
  createEffect(
    on(stream, (s) => {
      const want = prefs()?.audio;
      const i = s && want ? s.sources.findIndex((x) => x.audio === want) : -1;
      setSourceIndex(i > 0 ? i : 0);
      setExtraSubs([]);
      setSubOffset(0);
    }),
  );

  createEffect(
    on([stream, sourceIndex], async ([s, idx]) => {
      destroy();
      setVideoError(null);
      setLevels([]);
      setLevel(-1);
      setAudioTracks([]);
      setAudioTrack(-1);
      setWaiting(true);
      if (!s || !video) return;
      const src = s.sources[Math.min(idx, s.sources.length - 1)];
      // HLS goes through hls.js whenever Media Source Extensions exist: newer WebView2 builds
      // claim native HLS support but play proxied streams unreliably, and hls.js gives us levels.
      const HlsLib = src.kind === "hls" ? (await import("hls.js")).default : null;
      if (src.kind === "hls" && !HlsLib?.isSupported() && !video.canPlayType("application/vnd.apple.mpegurl")) {
        return setVideoError("This system can't play HLS streams.");
      }
      if (HlsLib && HlsLib.isSupported()) {
        const instance = new HlsLib({ enableWorker: true, backBufferLength: 90, maxBufferLength: 60 });
        hls = instance;
        instance.on(HlsLib.Events.MANIFEST_PARSED, (_e, data) => {
          setLevels(data.levels.map((l) => ({ height: l.height, bitrate: l.bitrate })));
        });
        instance.on(HlsLib.Events.AUDIO_TRACKS_UPDATED, (_e, data) => {
          const list = data.audioTracks.map((t) => ({ name: t.name, lang: t.lang ?? "" }));
          setAudioTracks(list);
          // Switch to the preferred audio language when the stream carries it.
          const want = prefs()?.audio;
          const i = want ? list.findIndex((t) => langMatches(t.lang, t.name, want)) : -1;
          if (i >= 0 && i !== instance.audioTrack) instance.audioTrack = i;
          setAudioTrack(instance.audioTrack);
        });
        instance.on(HlsLib.Events.AUDIO_TRACK_SWITCHED, (_e, data) => setAudioTrack(data.id));
        let recovered = 0;
        instance.on(HlsLib.Events.ERROR, (_e, data) => {
          if (!data.fatal) return;
          if (data.type === HlsLib.ErrorTypes.MEDIA_ERROR && recovered++ < 2) return instance.recoverMediaError();
          if (data.type === HlsLib.ErrorTypes.NETWORK_ERROR && recovered++ < 3) return instance.startLoad();
          setVideoError(`Playback failed (${data.details}).${s.sources.length > idx + 1 ? " Try another source." : ""}`);
        });
        instance.loadSource(src.url);
        instance.attachMedia(video);
      } else {
        video.src = src.url;
      }
    }),
  );

  // External subtitles are <track>s; HLS ones arrive as TextTracks too. Keep one list.
  let autoPicked = false;
  const refreshTracks = () => {
    const list = Array.from(video.textTracks).filter((t) => t.kind === "subtitles" || t.kind === "captions");
    setTracks(list);
    for (const t of list) t.oncuechange = applyOffset;
    applyOffset();
    const showing = list.findIndex((t) => t.mode === "showing");
    if (showing >= 0) setActiveTrack(showing);
    else if (!autoPicked && list.length) {
      // The preferred subtitle language (Settings → Playback); "off" shows none.
      const want = prefs()?.subtitles ?? "en";
      const preferred = want === "off" ? -1 : list.findIndex((t) => langMatches(t.language, t.label, want));
      chooseTrack(preferred, false);
      autoPicked = preferred >= 0 || want === "off";
    }
  };
  /** A track's menu name; tracks sharing a name are numbered so they can be told apart. */
  const trackName = (t: TextTrack, i: number) => {
    const base = t.label || t.language || `Track ${i + 1}`;
    const same = tracks().filter((x) => (x.label || x.language) === (t.label || t.language));
    return same.length > 1 ? `${base} (${same.indexOf(t) + 1})` : base;
  };
  const chooseTrack = (i: number, close = true) => {
    tracks().forEach((t, j) => (t.mode = j === i ? "showing" : "disabled"));
    setActiveTrack(i);
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

  const chooseAudio = (i: number) => {
    if (hls) hls.audioTrack = i;
    setAudioTrack(i);
    setMenu(null);
  };
  /** Sources that differ by audio language, for the Audio menu when the stream has no tracks. */
  const audioSources = () => (stream()?.sources ?? []).map((s, i) => ({ ...s, i })).filter((s) => s.audio);
  const audioName = (code: string | null) => (code === "ja" ? "Japanese" : code === "en" ? "English" : code ?? "Unknown");

  // ── Resume, position saving, watched-at-80%, skip times
  let resumed = false;
  let marked = false;
  let lastSave = 0;
  createEffect(on(() => `${mediaId()}:${episode()}`, () => {
    resumed = false;
    marked = false;
    setSkips([]);
    setCountdown(null);
    setActiveTrack(-1);
  }));

  const persist = () => {
    if (!inTauri || !duration()) return;
    void savePosition(mediaId(), episode(), video.currentTime, duration()).catch(() => {});
  };

  const onMeta = async () => {
    setDuration(video.duration || 0);
    video.volume = volume();
    if (!resumed && inTauri) {
      resumed = true;
      const pos = await getPosition(mediaId(), episode()).catch(() => null);
      if (pos && pos > 30 && pos < video.duration * 0.9) {
        video.currentTime = pos;
        notify(`Resumed at ${fmtClock(pos)}.`);
      }
    }
    const mal = media()?.idMal;
    if (mal && inTauri) setSkips(await getSkipTimes(mal, episode(), video.duration || 0).catch(() => []));
    void video.play().catch(() => {});
  };

  const onTime = () => {
    const t = video.currentTime;
    setTime(t);
    const b = video.buffered;
    for (let i = 0; i < b.length; i++) if (b.start(i) <= t && t <= b.end(i)) setBuffered(b.end(i));
    if (performance.now() - lastSave > 10_000) {
      lastSave = performance.now();
      persist();
    }
    if (!marked && duration() > 0 && t / duration() >= WATCHED_AT) {
      marked = true;
      void markWatched();
    }
  };

  const markWatched = async () => {
    const entry = page()?.listEntry ?? null;
    if (!inTauri || (entry && entry.progress >= episode())) return;
    const total = media()?.episodes ?? null;
    try {
      await saveEntry(mediaId(), applyRules(entry, { progress: episode() }, total));
      notify(`Episode ${episode()} marked watched on AniList.`);
    } catch (e) {
      notify(`Couldn't update AniList: ${errorText(e)}`);
    }
  };

  const activeSkip = () => skips().find((s) => time() >= s.start && time() < s.end - 1);
  const skip = () => {
    const s = activeSkip();
    if (s) video.currentTime = s.end;
  };

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
    persist();
  });

  // ── Controls
  const toggle = () => (video.paused ? void video.play() : video.pause());
  const seekBy = (d: number) => (video.currentTime = Math.max(0, Math.min(duration(), video.currentTime + d)));
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
    setLevel(i);
    setMenu(null);
  };
  const toggleFullscreen = async () => {
    const next = !fullscreen();
    setFullscreen(next);
    if (inTauri) {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().setFullscreen(next).catch(() => {});
    } else if (next) void shell.requestFullscreen?.().catch(() => {});
    else if (document.fullscreenElement) void document.exitFullscreen();
  };
  onCleanup(() => {
    if (fullscreen() && inTauri) void import("@tauri-apps/api/window").then(({ getCurrentWindow }) => getCurrentWindow().setFullscreen(false));
  });
  const togglePip = () => {
    if (document.pictureInPictureElement) void document.exitPictureInPicture();
    else void video.requestPictureInPicture?.().catch(() => {});
  };
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
          ? "Opened in MPV. Progress syncs to AniList when you're 80% through."
          : "Opened in VLC. Progress isn't tracked there.",
      );
    } catch (e) {
      notify(errorText(e));
    }
  };

  // Controls hide after a moment of stillness while playing.
  const [idle, setIdle] = createSignal(false);
  let idleTimer: number | undefined;
  const wake = () => {
    setIdle(false);
    clearTimeout(idleTimer);
    idleTimer = window.setTimeout(() => playing() && !menu() && setIdle(true), 2600);
  };
  onCleanup(() => clearTimeout(idleTimer));

  const onKey = (e: KeyboardEvent) => {
    if ((e.target as HTMLElement)?.closest("input, select, textarea")) return;
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
        video.currentTime = (Number(k) / 10) * duration();
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
      classList={{ "is-fullscreen": fullscreen(), "is-idle": idle() && playing() && !menu() }}
      data-subsize={subSize()}
      onPointerMove={wake}
      onPointerDown={wake}
    >
      <video
        ref={video}
        class="player-video"
        playsinline
        crossorigin="anonymous"
        onLoadedMetadata={() => void onMeta()}
        onTimeUpdate={onTime}
        onDurationChange={() => setDuration(video.duration || 0)}
        onPlay={() => { setPlaying(true); wake(); }}
        onPause={() => { setPlaying(false); setIdle(false); persist(); }}
        onWaiting={() => setWaiting(true)}
        onPlaying={() => setWaiting(false)}
        onCanPlay={() => setWaiting(false)}
        onEnded={onEnded}
        onError={() =>
          !hls &&
          setVideoError(
            local()
              ? "The built-in player can't play this file's format (common with MKV)."
              : "This video couldn't be played.",
          )
        }
        onClick={toggle}
        onDblClick={() => void toggleFullscreen()}
      >
        <For each={stream()?.subtitles ?? []}>
          {(s) => <track kind="subtitles" src={s.url} srclang={s.lang} label={s.label} onLoad={refreshTracks} />}
        </For>
        <For each={extraSubs()}>
          {(s) => <track kind="subtitles" src={s.url} label={s.label} onLoad={refreshTracks} />}
        </For>
      </video>

      <Show when={failure() ?? videoError()}>
        {(msg) => (
          <div class="player-message" role="alert">
            <p>{msg()}</p>
            <div class="setting-actions">
              <Show when={videoError() && (stream()?.sources.length ?? 0) > sourceIndex() + 1}>
                <button type="button" class="btn btn-primary" onClick={() => setSourceIndex(sourceIndex() + 1)}>Try next source</button>
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
      <Show when={!failure() && !videoError() && waiting()}>
        <div class="player-spinner" aria-label="Loading" />
      </Show>

      <header class="player-top">
        <button type="button" class="back-link" onClick={back}>
          <ArrowLeft size={16} stroke-width={2} aria-hidden="true" /> Back
        </button>
        <div class="player-title">
          <p class="player-show">{title()}</p>
          <p class="player-ep num">
            Episode {episode()}
            <Show when={epTitle() && !/^episode\s*\d+$/i.test(epTitle()!)}> · {epTitle()}</Show>
          </p>
        </div>
      </header>

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

      <div class="player-controls" onPointerDown={(e) => e.stopPropagation()}>
        <div class="seek">
          <div class="seek-track" aria-hidden="true">
            <div class="seek-buffered" style={{ width: `${duration() ? (buffered() / duration()) * 100 : 0}%` }} />
            <div class="seek-played" style={{ width: `${duration() ? (time() / duration()) * 100 : 0}%` }} />
            <For each={skips()}>
              {(s) => (
                <div
                  class="seek-chapter"
                  style={{ left: `${duration() ? (s.start / duration()) * 100 : 0}%`, width: `${duration() ? ((s.end - s.start) / duration()) * 100 : 0}%` }}
                />
              )}
            </For>
          </div>
          <input
            type="range"
            class="seek-input"
            min="0"
            max={duration() || 0}
            step="0.1"
            value={time()}
            aria-label="Seek"
            aria-valuetext={`${fmtClock(time())} of ${fmtClock(duration())}`}
            onInput={(e) => (video.currentTime = Number(e.currentTarget.value))}
          />
        </div>

        <div class="player-bar">
          <div class="bar-group">
            <button type="button" class="icon-btn" onClick={toggle} aria-label={playing() ? "Pause (K)" : "Play (K)"}>
              <Show when={playing()} fallback={<Play size={20} fill="currentColor" stroke-width={0} />}>
                <Pause size={20} fill="currentColor" stroke-width={0} />
              </Show>
            </button>
            <button type="button" class="icon-btn" disabled={!hasPrev()} onClick={() => goTo(episode() - 1)} aria-label="Previous episode (P)">
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
              {fmtClock(time())} / {fmtClock(duration())}
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
                            <button type="button" role="menuitemradio" aria-checked={sourceIndex() === s.i} onClick={() => { setSourceIndex(s.i); setMenu(null); }}>
                              {audioName(s.audio)}
                            </button>
                          )}
                        </For>
                      }
                    >
                      <For each={audioTracks()}>
                        {(t, i) => (
                          <button type="button" role="menuitemradio" aria-checked={audioTrack() === i()} onClick={() => chooseAudio(i())}>
                            {t.name || audioName(t.lang)}
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
                <div class="menu menu-wide" role="menu">
                  <p class="menu-head">Subtitles</p>
                  <button type="button" role="menuitemradio" aria-checked={activeTrack() === -1} onClick={() => chooseTrack(-1)}>Off</button>
                  <For each={tracks()}>
                    {(t, i) => (
                      <button type="button" role="menuitemradio" aria-checked={activeTrack() === i()} onClick={() => chooseTrack(i())}>
                        {trackName(t, i())}
                      </button>
                    )}
                  </For>
                  <Show when={inTauri}>
                    <button type="button" role="menuitem" class="menu-action" onClick={() => void loadSubtitleFile()}>
                      Load from file…
                    </button>
                  </Show>

                  <p class="menu-head">Size</p>
                  <div class="menu-row" role="group" aria-label="Subtitle size">
                    <For each={[["s", "Small"], ["m", "Medium"], ["l", "Large"]] as const}>
                      {([v, label]) => (
                        <button type="button" aria-pressed={subSize() === v} onClick={() => setSubSize(v)}>{label}</button>
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
                </div>
              </Show>
            </div>

            <Show when={levels().length > 1 || (stream()?.sources.length ?? 0) > 1}>
              <div class="menu-wrap">
                <button type="button" class="icon-btn" aria-haspopup="menu" aria-expanded={menu() === "quality"} onClick={() => setMenu(menu() === "quality" ? null : "quality")} aria-label="Quality and source">
                  <Settings2 size={20} stroke-width={2} />
                </button>
                <Show when={menu() === "quality"}>
                  <div class="menu" role="menu">
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
                          <button type="button" role="menuitemradio" aria-checked={sourceIndex() === i()} onClick={() => { setSourceIndex(i()); setMenu(null); }}>
                            {s.label}
                          </button>
                        )}
                      </For>
                    </Show>
                  </div>
                </Show>
              </div>
            </Show>

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

            <button type="button" class="icon-btn" onClick={togglePip} aria-label="Picture in picture">
              <PictureInPicture size={18} stroke-width={2} />
            </button>
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
