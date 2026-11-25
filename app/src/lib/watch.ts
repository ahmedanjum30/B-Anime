import { createEffect, createRoot, createSignal, on, onCleanup } from "solid-js";

import {
  errorText, inTauri, onEvent, uiLog, watchCheck, watchHost, watchJoin, watchLeave, watchReady, watchRejoin, watchRename,
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
  /**
   * Where this device plays the room's show from, as picked on the Together page: its library's
   * files or a source (which one is remembered per show, like everywhere else). Unset: the
   * library when it has the episode.
   */

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
    const p = nowPlaying();
    const entry: RecentRoom = {
      key: recentKey,
      title: roomName() || p?.title || "",
      names: others.map((m) => m.name),
      codes,
      at: Date.now(),
      place: p ? { state: { ...p, by: "", position: roomPosition(), held: false }, started: started(), name: roomName() } : undefined,
      hosted: r.host,
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
    remember();
  });
  // Where the room is, kept for a Rejoin after the app closes.
  window.setInterval(() => remember(), 5000);
  void onEvent<Roster>("watch-roster", (r) => {
    announceChanges(members(), r.members);
    setMembers(r.members);
    setStarted(r.started);
    setRoomName(r.name);
    remember();
  });
  void onEvent<RoomInfo>("watch-info", (info) => {
    const was = room();
    const oldHost = members().find((m) => m.host && m.id !== was?.me)?.name;
    const left = oldHost ? `${oldHost} (the host) left` : "The host left";
    setRoom(info);
    // This device was hosting and lost its connection; the room went on with a guest hosting,
    // and this device is back in it as a guest.
    if (was?.host && !info.host) return homeStore.notify("Your connection dropped and someone else took over hosting. You're back in the room.");
    // Taking over, the room stays paused (everyone stopped while it changed hands).
    if (was && info.host && !was.host) homeStore.notify(`${left}. You're hosting now; press play to carry on.`);
    else if (was && info.code !== was.code) homeStore.notify(`${left}. The room carries on paused; press play to carry on.`);
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
    if (before.playing && !s.playing) {
      // An app closing pauses its video, and that pause reaches the room just before its owner
      // disappears: only said if they're still here a moment later (then it was theirs).
      const id = s.by;
      window.setTimeout(() => {
        const m = members().find((x) => x.id === id);
        if (m && m.connected !== false && !reconnecting() && room()) homeStore.notify(`${who} paused.`);
      }, 1500);
    } else if (!before.playing && s.playing) homeStore.notify(`${who} resumed.`);
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
    const lost = new Set(before.filter((m) => m.connected === false).map((m) => m.id));
    for (const m of after) {
      if (m.id === me) continue;
      if (!was.has(m.id)) homeStore.notify(`${m.name} joined the room.`);
      else if (lost.has(m.id) && m.connected !== false) homeStore.notify(`${m.name} is back.`);
    }
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
    return enter("join", () => watchRejoin(entry.codes, name || "Guest", entry.place ?? null, !!entry.hosted), entry.key);
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
  const check = (playable: boolean | null, source: string | null, copyLength: number | null) => {
    const key = `${playable}|${source}|${copyLength}`;
    if (!room() || key === lastCheck) return;
    lastCheck = key;
    void watchCheck(playable, source, copyLength).catch(() => {});
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
  const status = (position: number, buffering: boolean, mediaId: number, episode: number, length: number | null) => {
    if (room()) void watchSendStatus(position, buffering, mediaId, episode, length).catch(() => {});
  };

  /**
   * How much longer (+) or shorter (−) someone's copy of the room's episode is than the host's,
   * in whole seconds, when it's enough to matter (a different cut: a Blu-ray file against a TV
   * stream). Such copies don't line up: the same time shows a different moment after the cut.
   */
  const lengthGap = (m: RoomMember): number | null => {
    const p = nowPlaying();
    const host = members().find((x) => x.host);
    if (!p || !host || m.host) return null;
    // The player's own length when it has the room's episode open; else the copy's, which the
    // waiting room knows before anyone plays it (from the file, or the stream's playlist).
    const len = (x: RoomMember) =>
      (x.mediaId === p.mediaId && x.episode === p.episode && x.length) || x.copyLength || null;
    const [a, b] = [len(m), len(host)];
    if (!a || !b) return null;
    const gap = Math.round(a - b);
    return Math.abs(gap) > LENGTH_SLACK ? gap : null;
  };
  /** "40 s shorter" / "2 min longer". */
  const gapText = (gap: number) => {
    const s = Math.abs(gap);
    return `${s >= 90 ? `${Math.round(s / 60)} min` : `${s} s`} ${gap < 0 ? "shorter" : "longer"}`;
  };
  /** The source the host plays the room's show from, when this device is a guest: its default. */
  const hostSourceFor = (mediaId: number) => {
    const p = nowPlaying();
    return room() && !room()!.host && p?.mediaId === mediaId ? (p.source ?? null) : null;
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
  // On the phone, while in a room: keeps playing and connected with the app in the background.
  createEffect(
    on(
      () => !!room(),
      (inRoom) =>
        (window as unknown as { BAnimeAndroid?: { setKeepAlive?(on: boolean): void } }).BAnimeAndroid?.setKeepAlive?.(inRoom),
    ),
  );

  return {
    room, members, remote, started, nowPlaying, busy, me, defaultName, reconnecting, invite, setInvite,
    recent, rejoin, forget, pendingRejoin, setPendingRejoin, roomPosition, roomPath, pick, nowAt, roomName, rename, nameRoom,
    lengthGap, gapText, hostSourceFor,
    host, join, leave, setReady, start, check, send, status, markOwnSwitch, staleForSwitch, notWaiting, waitFor,
  };
});

/** Seconds of difference that are corrected by a jump rather than by playing slightly faster or slower. */
const JUMP = 3;
/** Smaller differences than this are left alone. */
const CLOSE_ENOUGH = 0.3;
/** After a jump to catch up, no other jump for this long (ms): it loads there first. */
const JUMP_AGAIN_AFTER = 8000;
/** A counted start plays this long after the host sends it: time for it to reach everyone. */
const START_IN = 800;
/** Copies of an episode this many seconds apart in length are different cuts: they don't line up,
 *  so a held start waits for everyone to have the host's copy. */
const LENGTH_SLACK = 2;
/** Paused by someone this far ahead (at most): this device watches up to there instead of skipping to it. */
const CATCH_UP_MAX = 10;
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
  /**
   * This video hasn't taken the room's place yet (it just opened): its first play or seek is
   * the player starting up, not the host moving the room, so it joins the room instead of
   * sending its own spot (which took everyone back to it).
   */
  let joined = false;
  /** The player (or the whole app) is closing: the browser pauses its video, which isn't
   *  anyone's pause (it reached the room as "… paused" just before its owner disappeared). */
  let closing = false;
  onCleanup(() => (closing = true));
  const onUnload = () => (closing = true);
  window.addEventListener("pagehide", onUnload);
  window.addEventListener("beforeunload", onUnload);
  onCleanup(() => {
    window.removeEventListener("pagehide", onUnload);
    window.removeEventListener("beforeunload", onUnload);
  });
  /** The phone app is closing (swiped away): its page gets no unload of its own. */
  const appClosing = () =>
    !!(window as unknown as { BAnimeAndroid?: { isClosing?(): boolean } }).BAnimeAndroid?.isClosing?.();
  const sendNow = () => {
    const v = opts.video();
    if (closing || !v?.isConnected || (v.paused && appClosing())) return;
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
    // A guest's player (re)loading the episode hasn't taken the room's place yet: its plays and
    // pauses are its own starting up, not the viewer's.
    if (!joined && guest()) return;
    // Held after a switch: plays (the player's own autoplay) wait for everyone, quietly.
    if (v && !v.paused && w.room() && w.nowPlaying()?.held) {
      selfPause(v);
      return;
    }
    // Just opened, the room already going: take its place first.
    if (!joined && w.room()?.host && w.started()) {
      const p = w.nowPlaying();
      if (p && !p.held && p.mediaId === opts.mediaId() && p.episode === opts.episode()) {
        apply({ ...p, by: "", position: w.roomPosition(), at: performance.now() });
        return;
      }
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
  /** A guest in a running room: only the host moves the timeline. */
  const guest = () => !!w.room() && !w.room()!.host && w.started();
  /**
   * The viewer's own seek (the bar, ±10 s, keys, a double-tap), asked before it happens: a guest
   * is told only the host moves the timeline, and nothing moves. (Guessing from the video's
   * "seeked" events instead caught the player's own jumps: starting a stream, recovering from a
   * stall. Each was sent back with that notice, the stream started over, and so on.)
   */
  const mayMoveTimeline = () => {
    if (!guest()) return true;
    if (performance.now() - seekNoticeAt > 4000) {
      seekNoticeAt = performance.now();
      homeStore.notify("Only the host can move the timeline.");
    }
    return false;
  };
  const onSeeked = () => {
    const v = opts.video();
    if (closing || !v?.isConnected || ours("seek")) return;
    // A guest's player moving by itself (a stream starting, a stall's recovery, the small
    // window's buttons) isn't the room's: the room's next update (every 2 s) puts it back if it's
    // off. Nor is a player that's still (re)loading and hasn't taken the room's place yet.
    if (guest() || (!joined && w.room())) return;
    sendNow();
  };

  /** Watching up to where someone paused (see `apply`): stops there, unless something else happens first. */
  let catchUp: number | undefined;
  const watchUntil = (v: HTMLVideoElement, until: number) => {
    const check = () => {
      catchUp = undefined;
      // Paused by hand meanwhile (that's the viewer's own, and went to the room), or closing.
      if (closing || !v.isConnected || v.paused) return;
      const left = until - v.currentTime;
      if (left <= 0.05) return selfPause(v);
      catchUp = window.setTimeout(check, Math.min(250, left * 1000));
    };
    check();
  };
  onCleanup(() => clearTimeout(catchUp));

  /** When this player last jumped to catch up with the room (see apply). */
  let jumpedAt = -1e9;
  /** A counted start (see release): when it plays, and its timer. */
  let startsAt = 0;
  let startTimer: number | undefined;
  onCleanup(() => clearTimeout(startTimer));
  const apply = (s: RemoteState) => {
    const v = opts.video();
    if (!v) return;
    // Each word from the room replaces the last one's catching up.
    clearTimeout(catchUp);
    catchUp = undefined;
    clearTimeout(startTimer);
    // A counted start: at its position, paused, then playing at the moment everyone does (the
    // message took about half this device's ping to arrive).
    if (s.startIn && s.playing && !s.held && v.readyState >= 1) {
      const ping = w.me()?.rttMs ?? 0;
      const wait = Math.max(0, s.startIn - ping / 2 - (performance.now() - s.at));
      pending = null;
      joined = true;
      appliedAt = performance.now() + wait;
      startsAt = appliedAt;
      v.playbackRate = 1;
      if (Math.abs(v.currentTime - s.position) > CLOSE_ENOUGH) selfSeek(v, Math.max(0, s.position));
      selfPause(v);
      startTimer = window.setTimeout(() => selfPlay(v), wait);
      return;
    }
    if (s.held) s = { ...s, playing: false };
    if (v.readyState < 1) {
      pending = s;
      return;
    }
    pending = null;
    joined = true;
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
    if (!s.playing) {
      clearTimeout(rateReset);
      v.playbackRate = 1;
      if (s.held) {
        // Held at a start (Start, or a switch): everyone exactly there.
        if (Math.abs(gap) > CLOSE_ENOUGH) selfSeek(v, Math.max(0, expected));
      } else if (gap > CLOSE_ENOUGH && gap <= CATCH_UP_MAX && !v.paused) {
        // Paused by someone a little ahead of this device: it watches up to where they paused
        // and stops there, rather than skipping the bit in between.
        return watchUntil(v, expected);
      } else if (Math.abs(gap) > JUMP) {
        // Really somewhere else (the host moved it while paused, or this device was far off).
        selfSeek(v, Math.max(0, expected));
      }
      // A little ahead (or already stopped a little behind): it just stops where it is; playing
      // again brings everyone back in step.
      return selfPause(v);
    }
    if (Math.abs(gap) > JUMP && performance.now() - jumpedAt < JUMP_AGAIN_AFTER) {
      // Jumped a moment ago and still behind (it's loading there): no chasing the room with jump
      // after jump (each one loads again, and the room moves on meanwhile). Plays on from here.
      v.playbackRate = 1;
    } else if (Math.abs(gap) > JUMP) {
      // Not loaded there yet: land a little ahead, where the room will be once it has loaded.
      const lead = !isLoaded(v, expected) ? JUMP_LEAD : 0;
      uiLog(`sync: jump ${gap.toFixed(1)} s to ${(expected + lead).toFixed(1)} (lead ${lead}, at ${v.currentTime.toFixed(1)})`);
      jumpedAt = performance.now();
      selfSeek(v, Math.max(0, expected + lead));
      v.playbackRate = 1;
    } else if (Math.abs(gap) > CLOSE_ENOUGH && !(gap > 0 && slow())) {
      // Catch up (or let the others catch up) over a few seconds instead of a visible jump.
      v.playbackRate = gap > 0 ? 1.08 : 0.92;
      clearTimeout(rateReset);
      rateReset = window.setTimeout(() => (v.playbackRate = 1), (Math.abs(gap) / 0.08) * 1000);
    }
    selfPlay(v);
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
    w.members().filter(
      (m) => (m.buffering || m.connected === false) && m.id !== w.room()?.me && !w.notWaiting().has(m.id),
    );
  /** "Waiting for Ana to load, Ben to reconnect…" */
  const waitText = (list: RoomMember[]) => {
    const lost = list.filter((m) => m.connected === false).map((m) => m.name);
    const loading = list.filter((m) => m.connected !== false).map((m) => m.name);
    if (lost.length && !loading.length) return `${lost.join(", ")} lost connection. Waiting for them to come back…`;
    if (!lost.length) return `Waiting for ${loading.join(", ")} to load…`;
    return `Waiting for ${loading.join(", ")} to load and ${lost.join(", ")} to reconnect…`;
  };
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
          uiLog(`sync: the room waits for ${waitingFor} (loading or lost)`);
          v.pause(); // the pause goes to the room like any other
          homeStore.notify(waitText(still), {
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

  // This device lost its link to the room: it pauses too (the room waits for it), and follows
  // the room again once it's back.
  createEffect(
    on(w.reconnecting, (lost) => {
      const v = opts.video();
      if (lost && v && w.started() && !w.room()?.host) selfPause(v);
    }),
  );

  // Held at Start or after a switch (the host's player): once everyone's player is on the
  // episode and has loaded it, play it for everyone. Never by itself otherwise: the host decides
  // (Start without waiting, or Don't wait for someone).
  // Ready: on the episode, loaded, and the same copy as the host's (a different cut, a Blu-ray
  // file against a TV stream, would drift out of step: it waits for them to switch).
  const ready = (m: RoomMember) =>
    m.mediaId === opts.mediaId() && m.episode === opts.episode() && !m.buffering && w.lengthGap(m) === null;
  const release = (late: string[]) => {
    const v = opts.video();
    const p = w.nowPlaying();
    if (!v || !p?.held) return;
    // A counted start: everyone plays at the same moment (each guest takes off the time the
    // message took to reach it), instead of the host first and the others a little behind.
    w.send({ ...p, held: false, playing: true, position: v.currentTime, startIn: START_IN });
    startsAt = performance.now() + START_IN;
    appliedAt = startsAt;
    clearTimeout(startTimer);
    startTimer = window.setTimeout(() => selfPlay(v), START_IN);
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
    if (v && w.room()) {
      const length = Number.isFinite(v.duration) && v.duration > 0 ? v.duration : null;
      // Paused, it's loaded once it has the picture where it is: a paused phone doesn't buffer
      // ahead, and counted as loading (Start and switches kept waiting for it).
      const loading = opts.waiting() || v.readyState < (v.paused ? 2 : 3);
      w.status(v.currentTime, loading, opts.mediaId(), opts.episode(), length);
    }
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
  /** When the video last left the small window (picture in picture). */
  let pipLeftAt = -1e9;
  const onLeftPip = () => (pipLeftAt = performance.now());
  const onPauseEvent = () => {
    if (ours("pause")) return;
    const v = opts.video();
    // The small window's close button pauses the video on its way out, and the player puts it
    // back playing (the room still is): that pause goes to the room only if it stays.
    if (v && (document.pictureInPictureElement === v || performance.now() - pipLeftAt < 800)) {
      // Meanwhile it's this viewer's pause as far as the room's updates go (they don't undo it).
      own = { playing: false, at: performance.now() };
      window.setTimeout(() => v.paused && sendNow(), 500);
      return;
    }
    sendNow();
  };

  const attach = (v: HTMLVideoElement) => {
    joined = false;
    const onReady = () => {
      if (pending) return apply(pending);
      // A guest follows the room's last word. Not the host: its own changes since (a pause, say)
      // aren't in it, but they are in nowPlaying, below.
      const s = w.remote();
      if (s && !w.room()?.host && s.mediaId === opts.mediaId() && s.episode === opts.episode()) return apply(s);
      // Back in the player after browsing (or after picking): the room carried on meanwhile, so
      // start where it is now. The host hears no states of its own, so this is how it catches up.
      const p = w.nowPlaying();
      if (w.room() && w.started() && p && p.mediaId === opts.mediaId() && p.episode === opts.episode()) {
        apply({ ...p, by: "", position: w.roomPosition(), at: performance.now() });
      }
    };
    v.addEventListener("loadedmetadata", onReady);
    // A new stream in the same video (another server, a reload): it joins the room again.
    const onEmptied = () => {
      joined = false;
      clearTimeout(startTimer);
    };
    v.addEventListener("emptied", onEmptied);
    onCleanup(() => v.removeEventListener("emptied", onEmptied));
    if (v.readyState >= 1) queueMicrotask(onReady);
    v.addEventListener("play", onPlayEvent);
    v.addEventListener("pause", onPauseEvent);
    v.addEventListener("leavepictureinpicture", onLeftPip);
    v.addEventListener("seeked", onSeeked);
    v.addEventListener("waiting", onStall);
    v.addEventListener("playing", statusSoon);
    v.addEventListener("canplay", statusSoon);
    onCleanup(() => {
      v.removeEventListener("loadedmetadata", onReady);
      v.removeEventListener("play", onPlayEvent);
      v.removeEventListener("pause", onPauseEvent);
      v.removeEventListener("leavepictureinpicture", onLeftPip);
      v.removeEventListener("seeked", onSeeked);
      v.removeEventListener("waiting", onStall);
      v.removeEventListener("playing", statusSoon);
      v.removeEventListener("canplay", statusSoon);
      clearTimeout(rateReset);
    });
  };

  /**
   * The host's player out of step with the room (it couldn't follow a guest's play, pause or
   * the time passing while the app was in the background): it catches up with the room, instead
   * of its regular report taking everyone back to where it stopped. True when it had to.
   */
  const followRoom = () => {
    const v = opts.video();
    const s = current();
    const p = w.nowPlaying();
    if (!v || !s || !p || !w.room()?.host || !w.started() || p.held || performance.now() < startsAt + 500) return false;
    if (p.mediaId !== opts.mediaId() || p.episode !== opts.episode()) return false;
    // This viewer's own play or pause is on its way to the room: it's the room's new state.
    if (own && performance.now() - own.at < 1500) return false;
    const at = w.roomPosition();
    if (s.playing === p.playing && Math.abs(at - s.position) <= JUMP) return false;
    apply({ ...p, by: "", position: at, at: performance.now() });
    return true;
  };

  // The host's regular position (guests correct drift from it) and everyone's status.
  const tick = window.setInterval(() => {
    if (!w.room()) return;
    reportStatus();
    if (followRoom()) return;
    const s = current();
    if (w.room()?.host && s && performance.now() - appliedAt > 800) w.send(s);
  }, 2000);
  onCleanup(() => clearInterval(tick));
  // Back from the background: catch up at once, before anything is reported.
  const onVisible = () => {
    if (document.visibilityState === "visible") followRoom();
  };
  document.addEventListener("visibilitychange", onVisible);
  onCleanup(() => document.removeEventListener("visibilitychange", onVisible));

  // Out of the room (left it, or it ended): the player pauses rather than playing on by
  // itself, out of step; play carries on alone.
  createEffect(
    on(
      w.room,
      (r, was) => {
        const v = opts.video();
        if (was && !r && v) selfPause(v);
      },
      { defer: true },
    ),
  );

  return { attach, startNow, mayMoveTimeline };
}
