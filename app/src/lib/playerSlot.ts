import { createRoot, createSignal } from "solid-js";
import { watchStore } from "./watch";

/** The episode the player has open, from its /watch address. */
export interface PlayerTarget {
  id: number;
  ep: number;
  ext?: string;
  dub: boolean;
  local: boolean;
  /** Its address (with the query), to go back to it. */
  path: string;
}

/** Reads a /watch/<id>/<ep>?… address. */
export function targetFromPath(path: string): PlayerTarget | null {
  const url = new URL(path, "http://x");
  const m = /^\/watch\/(\d+)\/([\d.]+)$/.exec(url.pathname);
  if (!m) return null;
  return {
    id: Number(m[1]),
    ep: Number(m[2]),
    ext: url.searchParams.get("ext") || undefined,
    dub: url.searchParams.get("dub") === "1",
    local: url.searchParams.get("local") === "1",
    path: url.pathname + url.search,
  };
}

/** `path` is the episode `current` already has open, with nothing said about how to play it
 *  (a link to the room's episode): the player carries on as it is rather than loading it again. */
export function sameEpisode(current: PlayerTarget | null, t: PlayerTarget): boolean {
  if (!current || current.id !== t.id || current.ep !== t.ep || current.dub !== t.dub || t.ext || t.local) return false;
  // In a room, the copy must also be the one picked under Playing from (the library, or a
  // source): a library copy left open doesn't stand in for a source, nor the other way round.
  const choice = watchStore.room() ? watchStore.playFrom() : null;
  return !choice || choice.mediaId !== current.id || choice.library === current.local;
}

/**
 * The player lives in the Shell, not in its page, so it can keep going behind the app: in a
 * Watch together room, leaving the player's page doesn't stop the episode (on the PC it pops
 * out into a small window; the phone keeps its sound). The /watch page just says what to play.
 */
export const playerSlot = createRoot(() => {
  const [target, setTarget] = createSignal<PlayerTarget | null>(null, {
    equals: (a, b) => a?.path === b?.path,
  });
  /** The player's page is open (otherwise it's playing behind the app, or closed). */
  const [onPage, setOnPage] = createSignal(false);
  /** The player's video is in the small window (picture in picture). */
  const [inPip, setInPip] = createSignal(false);
  /** Pops the player's video out into the small window (the player sets it). The browser only
   *  allows it on a click, so the room bar offers it when leaving the player didn't. */
  let popOut: (() => void) | null = null;
  const setPopOut = (f: (() => void) | null) => (popOut = f);
  return { target, setTarget, onPage, setOnPage, inPip, setInPip, setPopOut, popOut: () => popOut?.() };
});
