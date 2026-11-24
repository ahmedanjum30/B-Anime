import { createEffect, createRoot, createSignal, on, onCleanup } from "solid-js";

import {
  errorText, inTauri, onEvent, watchCheck, watchHost, watchJoin, watchLeave, watchReady, watchRoom, watchSendState,
  watchSendStatus, watchStart,
} from "./api";
import { homeStore } from "./store";
import type { PlayState, RoomInfo, RoomMember, Roster, SourceRef } from "./types";

/** A state from the room, stamped with when it arrived (to account for time passing while playing). */
export type RemoteState = PlayState & { at: number };
/** What a device sends: everything but who sent it (the backend fills that in). */
export type OwnState = Omit<PlayState, "by">;

const NAME_KEY = "banime.watch.name";

/** The website's join page: opens B-Anime into the room, or offers the download. The code stays
 *  after the "#", which browsers never send to the server. */
export const INVITE_PAGE = "https://ahmedanjum30.github.io/B-Anime/join/";
export const inviteLink = (code: string) => `${INVITE_PAGE}#${code}`;

/** The room code in a banime://join/<code> link (or an invite page link pasted into the app). */
export function codeFromLink(link: string): string | null {
  const m = /^banime:\/\/join\/?([A-Za-z0-9_-]+)/.exec(link.trim()) ?? /\/join\/?#([A-Za-z0-9_-]+)/.exec(link.trim());
  return m ? m[1] : null;
}

/**
 * Watch together: the room this device is in, who's there, and what the room is playing.
 * Devices connect directly (the backend's iroh room); this keeps the UI side of it.
 */
export const watchStore = createRoot(() => {
  const [room, setRoom] = createSignal<RoomInfo | null>(null);
  const [members, setMembers] = createSignal<RoomMember[]>([]);
  const [remote, setRemote] = createSignal<RemoteState | null>(null);
  /** False in the lobby: everyone gathers and gets ready, then the host starts. */
  const [started, setStarted] = createSignal(false);
  /** What the room is on: the last state from the room or sent from here. */
  const [nowPlaying, setNowPlaying] = createSignal<OwnState | null>(null);
  const [busy, setBusy] = createSignal<"host" | "join" | null>(null);
  /** The link to the room dropped and this device is getting back in. */
  const [reconnecting, setReconnecting] = createSignal(false);
  /** A room code from an invite link, waiting for the Together page to join it. */
  const [invite, setInvite] = createSignal<string | null>(null);

  void watchRoom().then((r) => r && setRoom(r)).catch(() => {});
  void onEvent<PlayState>("watch-state", (s) => {
    setRemote({ ...s, at: performance.now() });
    setNowPlaying(s);
  });
  void onEvent<Roster>("watch-roster", (r) => {
    announceChanges(members(), r.members);
    setMembers(r.members);
    setStarted(r.started);
  });
  void onEvent<RoomInfo>("watch-info", (info) => {
    const was = room();
    setRoom(info);
    if (info.host && !was?.host) homeStore.notify("The host left. You're hosting the room now.");
    else if (was && info.code !== was.code) homeStore.notify("The host left. The room carries on with a new host.");
  });
  void onEvent<"reconnecting" | "connected">("watch-link", (state) => setReconnecting(state === "reconnecting"));
  const reset = () => {
    setRoom(null);
    setMembers([]);
    setRemote(null);
    setStarted(false);
    setNowPlaying(null);
    setReconnecting(false);
  };
  void onEvent<string>("watch-closed", (reason) => {
    reset();
    homeStore.notify(reason);
  });

  /** "Sam joined", "Sam left": the room's comings and goings, as notices. */
  function announceChanges(before: RoomMember[], after: RoomMember[]) {
    const me = room()?.me;
    if (!before.length || !me) return;
    const was = new Set(before.map((m) => m.id));
    const now = new Set(after.map((m) => m.id));
    for (const m of after) if (!was.has(m.id) && m.id !== me) homeStore.notify(`${m.name} joined the room.`);
    for (const m of before) if (!now.has(m.id) && m.id !== me && !m.host) homeStore.notify(`${m.name} left the room.`);
  }

  // Invite links: banime://join/<code> opens (or is handed to) B-Anime.
  if (inTauri) {
    void (async () => {
      try {
        const { getCurrent, onOpenUrl } = await import("@tauri-apps/plugin-deep-link");
        const take = (urls: string[] | null) => {
          const code = urls?.map(codeFromLink).find(Boolean);
          if (code) setInvite(code);
        };
        take(await getCurrent());
        await onOpenUrl(take);
      } catch {
        /* the plugin isn't available (a browser preview) */
      }
    })();
  }

  const savedName = () => {
    try {
      return localStorage.getItem(NAME_KEY) ?? "";
    } catch {
      return "";
    }
  };
  /** The name others see: the one typed last, else the AniList name. */
  const defaultName = () => savedName() || homeStore.account()?.viewer?.name || "";
  const rememberName = (name: string) => {
    try {
      localStorage.setItem(NAME_KEY, name);
    } catch {
      /* private mode: just not remembered */
    }
  };

  const enter = async (kind: "host" | "join", run: () => Promise<RoomInfo>) => {
    setBusy(kind);
    try {
      reset();
      setRoom(await run());
      return true;
    } catch (e) {
      homeStore.notify(errorText(e));
      return false;
    } finally {
      setBusy(null);
    }
  };
  const host = (name: string) => {
    rememberName(name);
    return enter("host", () => watchHost(name || "Host"));
  };
  const join = (code: string, name: string) => {
    rememberName(name);
    const c = codeFromLink(code) ?? code.trim();
    return enter("join", () => watchJoin(c, name || "Guest"));
  };
  /** Leaves; a host ends the room for everyone (`end`) or hands it to the next guest. */
  const leave = async (end = false) => {
    reset();
    await watchLeave(end).catch(() => {});
  };
  /** This device in the member list. */
  const me = () => members().find((m) => m.id === room()?.me);
  const setReady = (ready: boolean) => void watchReady(ready).catch((e) => homeStore.notify(errorText(e)));
  const start = () => void watchStart().catch((e) => homeStore.notify(errorText(e)));
  /** Whether this device can play the room's episode (null: still checking), and with what. */
  let lastCheck = "";
  const check = (playable: boolean | null, source: string | null) => {
    const key = `${playable}|${source}`;
    if (!room() || key === lastCheck) return;
    lastCheck = key;
    void watchCheck(playable, source).catch(() => {});
  };
  createEffect(on(room, () => (lastCheck = "")));

  /** The episode this device switched to itself, and when: the room's older state is ignored for a moment. */
  let ownSwitch: { mediaId: number; episode: number; at: number } | null = null;
  const send = (s: OwnState) => {
    if (!room()) return;
    setNowPlaying(s);
    void watchSendState({ ...s, by: "" }).catch(() => {});
  };
  const status = (position: number, buffering: boolean) => {
    if (room()) void watchSendStatus(position, buffering).catch(() => {});
  };
  const markOwnSwitch = (mediaId: number, episode: number) => {
    ownSwitch = { mediaId, episode, at: performance.now() };
  };
  /** A state that's about another episode than this device just picked: stale, from before the room heard. */
  const staleForSwitch = (s: RemoteState) =>
    !!ownSwitch &&
    s.at - ownSwitch.at < 4000 &&
    (s.mediaId !== ownSwitch.mediaId || s.episode !== ownSwitch.episode);

  /** As host: people the room doesn't pause for while they load (a slow connection). */
  const [notWaiting, setNotWaiting] = createSignal<ReadonlySet<string>>(new Set());
  const waitFor = (id: string, wait: boolean) => {
    const next = new Set(notWaiting());
    if (wait) next.delete(id);
    else next.add(id);
    setNotWaiting(next);
  };
  createEffect(on(room, () => setNotWaiting(new Set<string>())));

  return {
    room, members, remote, started, nowPlaying, busy, me, defaultName, reconnecting, invite, setInvite,
    host, join, leave, setReady, start, check, send, status, markOwnSwitch, staleForSwitch, notWaiting, waitFor,
  };
});

/** Seconds of difference that are corrected by a jump rather than by playing slightly faster or slower. */
const JUMP = 3;
/** Smaller differences than this are left alone. */
const CLOSE_ENOUGH = 0.3;
/** Someone loading this long pauses the room (the host's player does it) until they catch up. */
const WAIT_FOR = 2000;
/** A jump to where the room is that isn't loaded yet lands this far ahead, so the room arrives
 *  about when the video is ready there (instead of stalling behind it again). */
const JUMP_LEAD = 2;
/** This many stalls within a minute: a slow connection, so no faster catch-up (it needs 8 % more
 *  data); drift is left to the jump. */
const SLOW_STALLS = 2;

/**
 * Keeps a player in step with the room. Local play, pause and seeks go to the room; the room's
 * changes are applied here. The host also reports its position every 2 s, which guests use to
 * correct drift: small gaps by nudging the playback rate, large ones by jumping. And the host
 * pauses everyone while someone is loading, and plays again once they've caught up.
 */
export function syncPlayer(opts: {
  video: () => HTMLVideoElement | undefined;
  mediaId: () => number;
  episode: () => number;
  title: () => string;
  dub: () => boolean;
  waiting: () => boolean;
  /** The extension this device plays the show with (the host offers it to the others). */
  source: () => SourceRef | null;
}) {
  const w = watchStore;
  /** Events caused by applying the room's state aren't sent back to it. */
  let quietUntil = 0;
  let rateReset: number | undefined;
  let pending: RemoteState | null = null;
  /** When this device's video stalled to load, recently. */
  let stalls: number[] = [];
  const slow = () => {
    const since = performance.now() - 60_000;
    stalls = stalls.filter((t) => t > since);
    return stalls.length >= SLOW_STALLS;
  };
  const isLoaded = (v: HTMLVideoElement, at: number) => {
    for (let i = 0; i < v.buffered.length; i++) {
      if (v.buffered.start(i) <= at && at + 1 <= v.buffered.end(i)) return true;
    }
    return false;
  };

  const current = (): OwnState | null => {
    const v = opts.video();
    if (!v || !(v.readyState >= 1)) return null;
    return {
      mediaId: opts.mediaId(),
      episode: opts.episode(),
      title: opts.title(),
      // The host sets the room's starting Sub or Dub; switching audio in your own player later
      // is yours alone, so the room keeps its setting.
      dub: w.nowPlaying()?.dub ?? opts.dub(),
      position: v.currentTime,
      playing: !v.paused,
      source: w.room()?.host ? opts.source() : null,
    };
  };
  const sendNow = () => {
    if (performance.now() < quietUntil) return;
    const v = opts.video();
    // In the lobby only Start plays the episode: any other play (the player's own autoplay
    // included) is undone, and the room hears the episode, paused.
    if (v && !v.paused && w.room() && !w.started()) {
      quietUntil = performance.now() + 500;
      v.pause();
      homeStore.notify(
        w.room()!.host
          ? "Press Start on the Together page once everyone's ready."
          : "The host starts the episode once everyone's ready.",
      );
      const s = current();
      if (s) w.send({ ...s, playing: false });
      return;
    }
    // A play by hand ends a wait for someone loading (they'll catch up by jumping).
    if (v && !v.paused) waitingFor = null;
    const s = current();
    if (s) w.send(s);
  };

  // Only the host moves the timeline: a guest's own seek (the bar, ±10 s, keys, a double-tap)
  // goes back to where the room is.
  let seekNoticeAt = 0;
  const onSeeked = () => {
    if (performance.now() < quietUntil) return;
    const v = opts.video();
    const room = w.remote();
    if (v && w.room() && !w.room()!.host && w.started() && room && room.mediaId === opts.mediaId() && room.episode === opts.episode()) {
      const expected = room.position + (room.playing ? (performance.now() - room.at) / 1000 : 0);
      if (Math.abs(v.currentTime - expected) > 2) {
        quietUntil = performance.now() + 800;
        v.currentTime = Math.max(0, expected);
        if (performance.now() - seekNoticeAt > 4000) {
          seekNoticeAt = performance.now();
          homeStore.notify("Only the host can move the timeline.");
        }
        return;
      }
    }
    sendNow();
  };

  const apply = (s: RemoteState) => {
    const v = opts.video();
    if (!v) return;
    if (v.readyState < 1) {
      pending = s;
      return;
    }
    pending = null;
    quietUntil = performance.now() + 800;
    // Loading right now: no corrections (each would restart the load); the room waits for this
    // device instead, and play or pause still follows.
    if (opts.waiting() && s.playing) {
      if (v.paused) void v.play().catch(() => {});
      return;
    }
    const expected = s.position + (s.playing ? (performance.now() - s.at) / 1000 : 0);
    const gap = expected - v.currentTime;
    if (Math.abs(gap) > JUMP || !s.playing) {
      if (Math.abs(gap) > (s.playing ? JUMP : CLOSE_ENOUGH)) {
        // Not loaded there yet: land a little ahead, where the room will be once it has loaded.
        const lead = s.playing && !isLoaded(v, expected) ? JUMP_LEAD : 0;
        v.currentTime = Math.max(0, expected + lead);
      }
      v.playbackRate = 1;
    } else if (Math.abs(gap) > CLOSE_ENOUGH && !(gap > 0 && slow())) {
      // Catch up (or let the others catch up) over a few seconds instead of a visible jump.
      v.playbackRate = gap > 0 ? 1.08 : 0.92;
      clearTimeout(rateReset);
      rateReset = window.setTimeout(() => (v.playbackRate = 1), (Math.abs(gap) / 0.08) * 1000);
    }
    if (s.playing && v.paused) void v.play().catch(() => {});
    if (!s.playing && !v.paused) v.pause();
  };

  // The room's changes for this episode (other episodes are the Shell's to follow).
  createEffect(
    on(w.remote, (s) => {
      if (!s || !w.room()) return;
      if (s.mediaId === opts.mediaId() && s.episode === opts.episode()) apply(s);
    }),
  );

  // This player switched to an episode the room isn't on: it was picked here, so the room follows.
  // Only a switch: joining (or the room's first state arriving) makes this device follow the room.
  createEffect(
    on([opts.mediaId, opts.episode], ([id, ep]) => {
      const s = w.remote();
      if (!w.room() || !s || (s.mediaId === id && s.episode === ep)) return;
      w.markOwnSwitch(id, ep);
      w.send({ mediaId: id, episode: ep, title: opts.title(), dub: opts.dub(), position: 0, playing: false });
    }),
  );

  // Wait for everyone (the host's player): someone loading for WAIT_FOR pauses the room; once
  // nobody is, it plays again (only if it was this wait that paused it). The host can choose
  // not to wait for someone (Don't wait, in the notice or the People list).
  let waitingFor: string | null = null;
  let waitTimer: number | undefined;
  const loadingOthers = () =>
    w.members().filter((m) => m.buffering && m.id !== w.room()?.me && !w.notWaiting().has(m.id));
  createEffect(
    on(loadingOthers, (loading) => {
      const v = opts.video();
      if (!w.room()?.host || !w.started() || !v) return;
      clearTimeout(waitTimer);
      if (loading.length) {
        if (waitingFor || v.paused) return;
        waitTimer = window.setTimeout(() => {
          const still = loadingOthers();
          if (!still.length || v.paused) return;
          waitingFor = still.map((m) => m.name).join(", ");
          v.pause(); // the pause goes to the room like any other
          homeStore.notify(`Waiting for ${waitingFor} to load…`, {
            label: "Don't wait",
            run: () => {
              for (const m of still) w.waitFor(m.id, false);
              waitingFor = null;
              void v.play().catch(() => {});
            },
          });
        }, WAIT_FOR);
      } else if (waitingFor) {
        waitingFor = null;
        void v.play().catch(() => {});
      }
    }),
  );
  onCleanup(() => clearTimeout(waitTimer));

  const reportStatus = () => {
    const v = opts.video();
    if (v && w.room()) w.status(v.currentTime, opts.waiting());
  };
  // Loading starts and ends reach the room at once (not just with the 2 s report), so waits
  // for someone start and end promptly. After the Player has updated `waiting`.
  const statusSoon = () => queueMicrotask(reportStatus);
  const onStall = () => {
    stalls.push(performance.now());
    statusSoon();
  };

  const attach = (v: HTMLVideoElement) => {
    const onReady = () => {
      if (pending) apply(pending);
      else {
        const s = w.remote();
        if (s && s.mediaId === opts.mediaId() && s.episode === opts.episode()) apply(s);
      }
    };
    v.addEventListener("loadedmetadata", onReady);
    v.addEventListener("play", sendNow);
    v.addEventListener("pause", sendNow);
    v.addEventListener("seeked", onSeeked);
    v.addEventListener("waiting", onStall);
    v.addEventListener("playing", statusSoon);
    v.addEventListener("canplay", statusSoon);
    onCleanup(() => {
      v.removeEventListener("loadedmetadata", onReady);
      v.removeEventListener("play", sendNow);
      v.removeEventListener("pause", sendNow);
      v.removeEventListener("seeked", onSeeked);
      v.removeEventListener("waiting", onStall);
      v.removeEventListener("playing", statusSoon);
      v.removeEventListener("canplay", statusSoon);
      clearTimeout(rateReset);
    });
  };

  // The host's regular position (guests correct drift from it) and everyone's status.
  const tick = window.setInterval(() => {
    if (!w.room()) return;
    reportStatus();
    const s = current();
    if (w.room()?.host && s && performance.now() >= quietUntil) w.send(s);
  }, 2000);
  onCleanup(() => clearInterval(tick));

  return { attach };
}
