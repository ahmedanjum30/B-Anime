import { createEffect, createRoot, createSignal, on, onCleanup } from "solid-js";

import {
  errorText, inTauri, onEvent, watchCheck, watchHost, watchJoin, watchLeave, watchReady, watchRejoin, watchRename,
  watchRoom, watchSendState, watchSendStatus, watchSetRoomName, watchStart,
} from "./api";
import { homeStore } from "./store";
import type { PlayState, RecentRoom, RoomInfo, RoomMember, Roster, SourceRef } from "./types";

/** A state from the room, stamped with when it arrived (to account for time passing while playing). */
export type RemoteState = PlayState & { at: number };
/** What a device sends: everything but who sent it (the backend fills that in). */
export type OwnState = Omit<PlayState, "by">;

const NAME_KEY = "banime.watch.name";
/** The room name this device hosts under (when the host renamed it; kept for next time). */
const ROOM_NAME_KEY = "banime.watch.roomName";
const RECENT_KEY = "banime.watch.recent";
/** Rooms remembered for Rejoin: just the last one. */
const RECENT_MAX = 1;
/** On launch, a room this recent is offered straight away (the app was closed mid-session). */
const OFFER_REJOIN_MS = 30 * 60 * 1000;

function loadRecent(): RecentRoom[] {
  try {
    const list = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(list) ? list.filter((r) => r && Array.isArray(r.codes) && r.codes.length).slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}

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
  /** The room's name ("Ahmed's room" unless the host renamed it). */
  const [roomName, setRoomName] = createSignal("");
  /** What the room is on: the last state from the room or sent from here. */
  const [nowPlaying, setNowPlaying] = createSignal<OwnState | null>(null);
  /** When `nowPlaying` was last known (performance.now()), to work out where the room is now. */
  const [nowAt, setNowAt] = createSignal(0);
  /** Where the room is now: its last known position, moved on by the time since if playing. */
  const roomPosition = () => {
    const p = nowPlaying();
    if (!p) return 0;
    return p.position + (p.playing ? (performance.now() - nowAt()) / 1000 : 0);
  };
  /** The room's episode, as a path. */
  const roomPath = () => {
    const p = nowPlaying();
    return p ? `/watch/${p.mediaId}/${p.episode}${p.dub ? "?dub=1" : ""}` : null;
  };
  const [busy, setBusy] = createSignal<"host" | "join" | null>(null);
  /** The link to the room dropped and this device is getting back in. */
  const [reconnecting, setReconnecting] = createSignal(false);
  /** A room code from an invite link, waiting for the Together page to join it. */
  const [invite, setInvite] = createSignal<string | null>(null);
  /** The last rooms this device was in, newest first, to rejoin them. */
  const [recent, setRecent] = createSignal<RecentRoom[]>(loadRecent());
  /** A recent room to rejoin from the launch notice, waiting for the Together page. */
  const [pendingRejoin, setPendingRejoin] = createSignal<RecentRoom | null>(null);
  /** The entry for the room this device is in (its code when entered). */
  let recentKey: string | null = null;
  const saveRecent = (list: RecentRoom[]) => {
    setRecent(list);
    try {
      localStorage.setItem(RECENT_KEY, JSON.stringify(list));
    } catch {
      /* private mode: not remembered */
    }
  };
  /** Keeps this room's entry current: who's in it (their codes, the host first) and what's on. */
  const remember = () => {
    const r = room();
    if (!r || !recentKey) return;
    const others = members().filter((m) => m.id !== r.me);
    const codes = [...others.filter((m) => m.host), ...others.filter((m) => !m.host)].map((m) => m.code).filter(Boolean);
    if (!codes.length) return;
    const entry: RecentRoom = {
      key: recentKey,
      title: roomName() || nowPlaying()?.title || "",
      names: others.map((m) => m.name),
      codes,
      at: Date.now(),
    };
    saveRecent([entry, ...recent().filter((x) => x.key !== recentKey)].slice(0, RECENT_MAX));
  };
  const forget = (key: string) => saveRecent(recent().filter((x) => x.key !== key));

  void watchRoom().then((r) => r && setRoom(r)).catch(() => {});
  void onEvent<PlayState>("watch-state", (s) => {
    announcePlayback(nowPlaying(), s);
    setRemote({ ...s, at: performance.now() });
    setNowPlaying(s);
    setNowAt(performance.now());
  });
  void onEvent<Roster>("watch-roster", (r) => {
    announceChanges(members(), r.members);
    setMembers(r.members);
    setStarted(r.started);
    setRoomName(r.name);
    remember();
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
    setRoomName("");
    setNowPlaying(null);
    setReconnecting(false);
  };
  void onEvent<string>("watch-closed", (reason) => {
    reset();
    homeStore.notify(reason);
  });

  /** "Sam paused", "Sam resumed", "Sam jumped to 12:41": someone else's change, as a notice. */
  function announcePlayback(before: OwnState | null, s: PlayState) {
    const me = room()?.me;
    if (!before || !me || s.by === me || !s.by || s.held || before.held) return;
    if (before.mediaId !== s.mediaId || before.episode !== s.episode) return;
    const who = members().find((m) => m.id === s.by)?.name;
    if (!who) return;
    if (before.playing && !s.playing) homeStore.notify(`${who} paused.`);
    else if (!before.playing && s.playing) homeStore.notify(`${who} resumed.`);
    else if (Math.abs(s.position - roomPositionOf(before)) > 5) homeStore.notify(`${who} jumped to ${clock(s.position)}.`);
  }
  /** Where `p` would be now, from when it was last known. */
  const roomPositionOf = (p: OwnState) => p.position + (p.playing ? (performance.now() - nowAt()) / 1000 : 0);
  const clock = (sec: number) => {
    const t = Math.max(0, Math.floor(sec));
    const h = Math.floor(t / 3600);
    const m = Math.floor((t % 3600) / 60);
    const ss = String(t % 60).padStart(2, "0");
    return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
  };

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

  const enter = async (kind: "host" | "join", run: () => Promise<RoomInfo>, key?: string) => {
    setBusy(kind);
    try {
      reset();
      const info = await run();
      recentKey = key ?? info.code;
      setRoom(info);
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
    let roomName: string | null = null;
    try {
      roomName = localStorage.getItem(ROOM_NAME_KEY);
    } catch {
      /* not remembered */
    }
    return enter("host", () => watchHost(name || "Host", roomName));
  };
  const join = (code: string, name: string) => {
    rememberName(name);
    const c = codeFromLink(code) ?? code.trim();
    return enter("join", () => watchJoin(c, name || "Guest"));
  };
  /**
   * The host picks what the room watches. In the lobby it's just the pick; once started,
   * everyone moves to it and it plays (the same room, no new one): the caller then opens it here
   * too (`roomPath`). Anyone still loading is waited for as usual.
   */
  const pick = (s: Omit<OwnState, "position" | "playing">) => {
    // Once started: held at the start until everyone's player has loaded it (the host's player
    // then plays it for everyone). In the lobby it's just the pick.
    send({ ...s, position: 0, playing: false, held: started() });
    return started();
  };

  /** Rename yourself in the room (remembered for next time too). */
  const rename = (name: string) => {
    const n = name.trim();
    if (!n || !room()) return;
    rememberName(n);
    void watchRename(n).catch((e) => homeStore.notify(errorText(e)));
  };
  /** The host names the room; empty goes back to "<host>'s room". */
  const nameRoom = (name: string) => {
    const n = name.trim();
    // Kept for this device's next rooms; empty goes back to "<host>'s room".
    try {
      if (n) localStorage.setItem(ROOM_NAME_KEY, n);
      else localStorage.removeItem(ROOM_NAME_KEY);
    } catch {
      /* not remembered */
    }
    void watchSetRoomName(n).catch((e) => homeStore.notify(errorText(e)));
  };

  /** Back into a room from the recent list, through whoever hosts it now. */
  const rejoin = (entry: RecentRoom, name: string) => {
    rememberName(name);
    // A failed try keeps the room listed: someone may still be taking over hosting. (Forget
    // removes it; a room that ended here leaves the list by itself.)
    return enter("join", () => watchRejoin(entry.codes, name || "Guest"), entry.key);
  };
  /** Leaves; a host ends the room for everyone (`end`) or hands it to the next guest. A room
   *  ended here is gone, so it leaves the recent list too. */
  const leave = async (end = false) => {
    if (end && room()?.host && recentKey) forget(recentKey);
    recentKey = null;
    reset();
    await watchLeave(end).catch(() => {});
  };

  // Closed mid-session: offer the room again right away.
  queueMicrotask(() => {
    const last = recent()[0];
    if (!inTauri || !last || Date.now() - last.at > OFFER_REJOIN_MS) return;
    void watchRoom().then((r) => {
      if (r) return;
      const who = last.names.length ? ` with ${last.names.join(", ")}` : "";
      homeStore.notify(`You were watching ${last.title || "together"}${who}.`, {
        label: "Rejoin",
        run: () => setPendingRejoin(last),
      });
    });
  });
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
    setNowAt(performance.now());
    void watchSendState({ ...s, by: "" }).catch(() => {});
  };
  const status = (position: number, buffering: boolean, mediaId: number, episode: number) => {
    if (room()) void watchSendStatus(position, buffering, mediaId, episode).catch(() => {});
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
    recent, rejoin, forget, pendingRejoin, setPendingRejoin, roomPosition, roomPath, pick, nowAt, roomName, rename, nameRoom,
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
  /**
   * Play, pause and seek events this code caused (applying the room's state, undoing an early
   * play) aren't sent back to the room: each change made here expects its one event and skips
   * it. Anything else is the viewer's own and always goes to the room. (A time window instead
   * swallowed real presses that happened to land just after a room update.)
   */
  const expect = { play: 0, pause: 0, seek: 0 };
  const EXPECT_MS = 1500;
  const ours = (kind: keyof typeof expect) => {
    if (performance.now() < expect[kind]) {
      expect[kind] = 0;
      return true;
    }
    return false;
  };
  const selfPause = (v: HTMLVideoElement) => {
    if (v.paused) return;
    expect.pause = performance.now() + EXPECT_MS;
    v.pause();
  };
  const selfPlay = (v: HTMLVideoElement) => {
    if (!v.paused) return;
    expect.play = performance.now() + EXPECT_MS;
    void v.play().catch(() => (expect.play = 0));
  };
  const selfSeek = (v: HTMLVideoElement, t: number) => {
    expect.seek = performance.now() + EXPECT_MS;
    v.currentTime = t;
  };
  /** When the room's state was last applied here (the host's regular report waits a moment). */
  let appliedAt = 0;
  /** This viewer's own last play or pause: room updates sent before the room heard it (saying
   *  the opposite) are ignored for a moment, instead of undoing it. */
  let own: { playing: boolean; at: number } | null = null;
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
      // A hold after a switch stays until the host releases it (everyone has loaded).
      held: !!w.nowPlaying()?.held && w.nowPlaying()?.mediaId === opts.mediaId() && w.nowPlaying()?.episode === opts.episode(),
    };
  };
  /** The player is closing: the browser pauses its video, which isn't anyone's pause. */
  let closing = false;
  onCleanup(() => (closing = true));
  const sendNow = () => {
    const v = opts.video();
    if (closing || !v?.isConnected) return;
    // In the lobby only Start plays the episode: any other play (the player's own autoplay
    // included) is undone, and the room hears the episode, paused.
    if (v && !v.paused && w.room() && !w.started()) {
      selfPause(v);
      homeStore.notify(
        w.room()!.host
          ? "Press Start on the Together page once everyone's ready."
          : "The host starts the episode once everyone's ready.",
      );
      const s = current();
      if (s) w.send({ ...s, playing: false });
      return;
    }
    // Held after a switch: plays (the player's own autoplay) wait for everyone, quietly.
    if (v && !v.paused && w.room() && w.nowPlaying()?.held) {
      selfPause(v);
      return;
    }
    // A play by hand ends a wait for someone loading (they'll catch up by jumping).
    if (v && !v.paused) waitingFor = null;
    const s = current();
    if (s) {
      own = { playing: s.playing, at: performance.now() };
      w.send(s);
    }
  };

  // Only the host moves the timeline: a guest's own seek (the bar, ±10 s, keys, a double-tap)
  // goes back to where the room is.
  let seekNoticeAt = 0;
  const onSeeked = () => {
    const v = opts.video();
    if (closing || !v?.isConnected || ours("seek")) return;
    const room = w.remote();
    if (v && w.room() && !w.room()!.host && w.started() && room && room.mediaId === opts.mediaId() && room.episode === opts.episode()) {
      const expected = room.position + (room.playing ? (performance.now() - room.at) / 1000 : 0);
      if (Math.abs(v.currentTime - expected) > 2) {
        selfSeek(v, Math.max(0, expected));
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
    if (s.held) s = { ...s, playing: false };
    if (v.readyState < 1) {
      pending = s;
      return;
    }
    pending = null;
    // Crossed in flight with this viewer's own play or pause: the room hasn't heard it yet.
    if (own && performance.now() - own.at < 1500 && s.playing !== own.playing && !s.held) return;
    appliedAt = performance.now();
    // Loading right now: no corrections (each would restart the load); the room waits for this
    // device instead, and play or pause still follows.
    if (opts.waiting() && s.playing) {
      selfPlay(v);
      return;
    }
    const expected = s.position + (s.playing ? (performance.now() - s.at) / 1000 : 0);
    const gap = expected - v.currentTime;
    if (Math.abs(gap) > JUMP || !s.playing) {
      if (Math.abs(gap) > (s.playing ? JUMP : CLOSE_ENOUGH)) {
        // Not loaded there yet: land a little ahead, where the room will be once it has loaded.
        const lead = s.playing && !isLoaded(v, expected) ? JUMP_LEAD : 0;
        selfSeek(v, Math.max(0, expected + lead));
      }
      v.playbackRate = 1;
    } else if (Math.abs(gap) > CLOSE_ENOUGH && !(gap > 0 && slow())) {
      // Catch up (or let the others catch up) over a few seconds instead of a visible jump.
      v.playbackRate = gap > 0 ? 1.08 : 0.92;
      clearTimeout(rateReset);
      rateReset = window.setTimeout(() => (v.playbackRate = 1), (Math.abs(gap) / 0.08) * 1000);
    }
    if (s.playing) selfPlay(v);
    else selfPause(v);
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

  // Held at Start or after a switch (the host's player): once everyone's player is on the
  // episode and has loaded it, play it for everyone. Never by itself otherwise: the host decides
  // (Start without waiting, or Don't wait for someone).
  const ready = (m: { mediaId: number | null; episode: number | null; buffering: boolean }) =>
    m.mediaId === opts.mediaId() && m.episode === opts.episode() && !m.buffering;
  const release = (late: string[]) => {
    const v = opts.video();
    const p = w.nowPlaying();
    if (!v || !p?.held) return;
    w.send({ ...p, held: false, playing: true, position: v.currentTime });
    selfPlay(v);
    if (late.length) homeStore.notify(`Started without waiting for ${late.join(", ")}.`);
  };
  createEffect(
    on([w.members, w.nowPlaying], ([members, p]) => {
      if (!w.room()?.host || !p?.held || p.mediaId !== opts.mediaId() || p.episode !== opts.episode()) return;
      const waitingFor = members.filter((m) => !w.notWaiting().has(m.id) && !ready(m));
      if (!waitingFor.length) release([]);
    }),
  );
  /** The host's "Start without waiting": plays it now, saying who it didn't wait for. */
  const startNow = () => release(w.members().filter((m) => !ready(m)).map((m) => m.name));

  const reportStatus = () => {
    const v = opts.video();
    if (v && w.room()) w.status(v.currentTime, opts.waiting() || v.readyState < 3, opts.mediaId(), opts.episode());
  };
  // Loading starts and ends reach the room at once (not just with the 2 s report), so waits
  // for someone start and end promptly. After the Player has updated `waiting`.
  const statusSoon = () => queueMicrotask(reportStatus);
  const onStall = () => {
    stalls.push(performance.now());
    statusSoon();
  };

  const onPlayEvent = () => {
    if (!ours("play")) sendNow();
  };
  const onPauseEvent = () => {
    if (!ours("pause")) sendNow();
  };

  const attach = (v: HTMLVideoElement) => {
    const onReady = () => {
      if (pending) return apply(pending);
      const s = w.remote();
      if (s && s.mediaId === opts.mediaId() && s.episode === opts.episode()) return apply(s);
      // Back in the player after browsing (or after picking): the room carried on meanwhile, so
      // start where it is now. The host hears no states of its own, so this is how it catches up.
      const p = w.nowPlaying();
      if (w.room() && w.started() && p && p.mediaId === opts.mediaId() && p.episode === opts.episode()) {
        apply({ ...p, by: "", position: w.roomPosition(), at: performance.now() });
      }
    };
    v.addEventListener("loadedmetadata", onReady);
    v.addEventListener("play", onPlayEvent);
    v.addEventListener("pause", onPauseEvent);
    v.addEventListener("seeked", onSeeked);
    v.addEventListener("waiting", onStall);
    v.addEventListener("playing", statusSoon);
    v.addEventListener("canplay", statusSoon);
    onCleanup(() => {
      v.removeEventListener("loadedmetadata", onReady);
      v.removeEventListener("play", onPlayEvent);
      v.removeEventListener("pause", onPauseEvent);
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
    if (w.room()?.host && s && performance.now() - appliedAt > 800) w.send(s);
  }, 2000);
  onCleanup(() => clearInterval(tick));

  return { attach, startNow };
}
