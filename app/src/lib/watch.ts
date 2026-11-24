import { createEffect, createRoot, createSignal, on, onCleanup } from "solid-js";

import { errorText, onEvent, watchHost, watchJoin, watchLeave, watchReady, watchRoom, watchSendState, watchSendStatus, watchStart } from "./api";
import { homeStore } from "./store";
import type { PlayState, RoomInfo, RoomMember, Roster } from "./types";

/** A state from the room, stamped with when it arrived (to account for time passing while playing). */
export type RemoteState = PlayState & { at: number };

const NAME_KEY = "banime.watch.name";

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
  const [nowPlaying, setNowPlaying] = createSignal<Omit<PlayState, "by"> | null>(null);
  const [busy, setBusy] = createSignal<"host" | "join" | null>(null);

  void watchRoom().then((r) => r && setRoom(r)).catch(() => {});
  void onEvent<PlayState>("watch-state", (s) => {
    setRemote({ ...s, at: performance.now() });
    setNowPlaying(s);
  });
  void onEvent<Roster>("watch-roster", (r) => {
    setMembers(r.members);
    setStarted(r.started);
  });
  const reset = () => {
    setRoom(null);
    setMembers([]);
    setRemote(null);
    setStarted(false);
    setNowPlaying(null);
  };
  void onEvent<string>("watch-closed", (reason) => {
    reset();
    homeStore.notify(reason);
  });

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
    return enter("join", () => watchJoin(code.trim(), name || "Guest"));
  };
  const leave = async () => {
    reset();
    await watchLeave().catch(() => {});
  };
  /** This device in the member list. */
  const me = () => members().find((m) => m.id === room()?.me);
  const setReady = (ready: boolean) => void watchReady(ready).catch((e) => homeStore.notify(errorText(e)));
  const start = () => void watchStart().catch((e) => homeStore.notify(errorText(e)));

  /** The episode this device switched to itself, and when: the room's older state is ignored for a moment. */
  let ownSwitch: { mediaId: number; episode: number; at: number } | null = null;
  const send = (s: Omit<PlayState, "by">) => {
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

  return {
    room, members, remote, started, nowPlaying, busy, me, defaultName,
    host, join, leave, setReady, start, send, status, markOwnSwitch, staleForSwitch,
  };
});

/** Seconds of difference that are corrected by a jump rather than by playing slightly faster or slower. */
const JUMP = 1.5;
/** Smaller differences than this are left alone. */
const CLOSE_ENOUGH = 0.3;

/**
 * Keeps a player in step with the room. Local play, pause and seeks go to the room; the room's
 * changes are applied here. The host also reports its position every 2 s, which guests use to
 * correct drift: small gaps by nudging the playback rate, large ones by jumping.
 */
export function syncPlayer(opts: {
  video: () => HTMLVideoElement | undefined;
  mediaId: () => number;
  episode: () => number;
  title: () => string;
  dub: () => boolean;
  waiting: () => boolean;
}) {
  const w = watchStore;
  /** Events caused by applying the room's state aren't sent back to it. */
  let quietUntil = 0;
  let rateReset: number | undefined;
  let pending: RemoteState | null = null;

  const current = (): Omit<PlayState, "by"> | null => {
    const v = opts.video();
    if (!v || !(v.readyState >= 1)) return null;
    return {
      mediaId: opts.mediaId(),
      episode: opts.episode(),
      title: opts.title(),
      dub: opts.dub(),
      position: v.currentTime,
      playing: !v.paused,
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
    const s = current();
    if (s) w.send(s);
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
    const expected = s.position + (s.playing ? (performance.now() - s.at) / 1000 : 0);
    const gap = expected - v.currentTime;
    if (Math.abs(gap) > JUMP || !s.playing) {
      if (Math.abs(gap) > (s.playing ? JUMP : CLOSE_ENOUGH)) v.currentTime = Math.max(0, expected);
      v.playbackRate = 1;
    } else if (Math.abs(gap) > CLOSE_ENOUGH) {
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
    v.addEventListener("seeked", sendNow);
    onCleanup(() => {
      v.removeEventListener("loadedmetadata", onReady);
      v.removeEventListener("play", sendNow);
      v.removeEventListener("pause", sendNow);
      v.removeEventListener("seeked", sendNow);
      clearTimeout(rateReset);
    });
  };

  // The host's regular position (guests correct drift from it) and everyone's status.
  const tick = window.setInterval(() => {
    if (!w.room()) return;
    const v = opts.video();
    if (!v) return;
    w.status(v.currentTime, opts.waiting());
    const s = current();
    if (w.room()?.host && s && performance.now() >= quietUntil) w.send(s);
  }, 2000);
  onCleanup(() => clearInterval(tick));

  return { attach };
}
