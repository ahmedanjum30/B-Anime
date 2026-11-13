import { createEffect, createMemo, createResource, createSignal, on } from "solid-js";

import { findSource, inTauri, listExtensions, sourceItems } from "./api";
import type { ExtKind, InstalledExtension, Media, SourceItem } from "./types";

const LAST_KEY = (kind: ExtKind) => `banime.source.${kind}`;
const rememberExt = (kind: ExtKind, id: string) => {
  try {
    localStorage.setItem(LAST_KEY(kind), id);
  } catch {
    /* storage unavailable: just don't remember */
  }
};
const recall = (kind: ExtKind) => {
  try {
    return localStorage.getItem(LAST_KEY(kind));
  } catch {
    return null;
  }
};

// ── Session cache ───────────────────────────────────────────────────────────────
// Matches and episode lists are kept until the app closes, so switching sources (or Sub/Dub)
// back and forth, or opening the same show again, doesn't repeat the calls. A lookup that's
// still running is shared: a hundred quick switches make one call per source. Failures aren't
// kept, so trying again really retries. Video links are never cached (sites expire them).
type MatchState = Awaited<ReturnType<typeof findSource>>;
type Entry<T> = { promise: Promise<T>; value?: T; done: boolean };
const CACHE_LIMIT = 300;
const matchCache = new Map<string, Entry<MatchState>>();
const itemsCache = new Map<string, Entry<SourceItem[]>>();

/** The cached value when it's ready (no loading flash), else the shared in-flight promise. */
function cached<T>(cache: Map<string, Entry<T>>, key: string, load: () => Promise<T>): T | Promise<T> {
  const hit = cache.get(key);
  if (hit) {
    // Refresh its place, so the oldest entries are the ones dropped.
    cache.delete(key);
    cache.set(key, hit);
    return hit.done ? (hit.value as T) : hit.promise;
  }
  const entry: Entry<T> = { done: false, promise: Promise.resolve() as Promise<T> };
  entry.promise = load().then(
    (v) => {
      entry.value = v;
      entry.done = true;
      return v;
    },
    (e) => {
      if (cache.get(key) === entry) cache.delete(key);
      throw e;
    },
  );
  cache.set(key, entry);
  while (cache.size > CACHE_LIMIT) cache.delete(cache.keys().next().value!);
  return entry.promise;
}
function remember<T>(cache: Map<string, Entry<T>>, key: string, value: T) {
  cache.set(key, { promise: Promise.resolve(value), value, done: true });
}
const matchKeyOf = (k: { id: number; ext: string; dub: boolean }) => `${k.id}|${k.ext}|${k.dub ? 1 : 0}`;
const itemsKeyOf = (k: { ext: string; sourceId: string }) => `${k.ext}|${k.sourceId}`;

/** A value accessor with live `loading` / `error` properties, shaped like a Solid resource. */
function withState<T>(read: () => T, loading: () => boolean, error: () => unknown) {
  return Object.defineProperties(read, {
    loading: { get: loading, enumerable: true },
    error: { get: error, enumerable: true },
  }) as (() => T) & { readonly loading: boolean; readonly error: unknown };
}

/** Titles to search a source with, most specific first, without duplicates. */
export function searchTitles(m: Media): string[] {
  return [...new Set([m.title.romaji, m.title.english, m.title.userPreferred].filter((t): t is string => !!t?.trim()))];
}

/**
 * The online source for one show: which extension, what it matched to, and its episodes or
 * chapters. Shared by the show page, the player and the reader.
 */
export function createSource(
  media: () => Media | undefined,
  preferred?: () => string | undefined,
  /** English dub instead of the original audio (anime only; sources without dubs ignore it). */
  dub: () => boolean = () => false,
) {
  const kind = (): ExtKind => (media()?.type === "MANGA" ? "manga" : "anime");
  const [exts] = createResource(() => (inTauri ? listExtensions() : Promise.resolve([] as InstalledExtension[])));
  const usable = createMemo(() => (exts() ?? []).filter((e) => e.enabled && e.kind === kind()));

  const [ext, setExtSignal] = createSignal<string | null>(null);
  createEffect(() => {
    const list = usable();
    if (!list.length) return setExtSignal(null);
    const current = ext();
    if (current && list.some((e) => e.id === current)) return;
    const want = preferred?.() ?? recall(kind());
    setExtSignal(list.find((e) => e.id === want)?.id ?? list[0].id);
  });
  const setExt = (id: string) => {
    rememberExt(kind(), id);
    setExtSignal(id);
  };

  const matchKey = createMemo(() => {
    const m = media();
    const e = ext();
    const d = kind() === "anime" && dub();
    return m && e ? { id: m.id, ext: e, dub: d } : null;
  }, null, { equals: (a, b) => a?.id === b?.id && a?.ext === b?.ext && a?.dub === b?.dub });
  // Lookups never leave their resource in an error state: Solid rethrows a failed resource's
  // error from every read, so one source failing (a site down) used to break the episode list
  // for whichever source was picked next until the page was reloaded. Errors are kept beside
  // the value instead, and a reply for a source that's no longer selected is ignored.
  const [matchError, setMatchError] = createSignal<unknown>(undefined);
  let matchSeq = 0;
  const [matchRes, { refetch: refetchMatch, mutate: mutateMatch }] = createResource(matchKey, (k) => {
    const seq = ++matchSeq;
    setMatchError(undefined);
    const m = media()!;
    const got = cached(matchCache, matchKeyOf(k), () =>
      findSource(k.id, k.ext, searchTitles(m), m.seasonYear ?? m.startDate.year, k.dub),
    );
    if (!(got instanceof Promise)) return got;
    return got.catch((e) => {
      if (seq === matchSeq) setMatchError(e);
      return undefined;
    });
  });
  /** Looks the show up again, skipping the cache (Try again, after clearing a match). */
  const rematch = () => {
    const k = matchKey();
    if (k) matchCache.delete(matchKeyOf(k));
    return refetchMatch();
  };
  /** A match picked by hand: shown at once and kept for the session. */
  const setMatch = (v: MatchState) => {
    const k = matchKey();
    if (k) remember(matchCache, matchKeyOf(k), v);
    return mutateMatch(v);
  };
  // Live getters (Object.assign would run them once and copy frozen values).
  const match = withState(() => matchRes(), () => matchRes.loading, () => matchError());
  /** Whether the current match is for the dub (what the player should be told). */
  const isDub = () => matchKey()?.dub ?? false;

  // Episodes load only for a finished match on the source that's selected now. While another
  // source is being looked up, the last match still belongs to the old source: pairing its
  // show id with the new source fetched nothing, and Play silently disappeared.
  const itemsKey = createMemo(() => {
    if (matchRes.loading) return null;
    const mt = match();
    const k = matchKey();
    return mt?.status === "matched" && k ? { ext: k.ext, sourceId: mt.sourceId } : null;
  }, null, { equals: (a, b) => a?.ext === b?.ext && a?.sourceId === b?.sourceId });
  const [itemsError, setItemsError] = createSignal<unknown>(undefined);
  let itemsSeq = 0;
  const [itemsRes, { refetch: refetchItems }] = createResource(itemsKey, (k) => {
    const seq = ++itemsSeq;
    setItemsError(undefined);
    // An empty list usually means the source hiccuped; don't keep it for the session.
    const got = cached(itemsCache, itemsKeyOf(k), async () => {
      const list = await sourceItems(k.ext, k.sourceId);
      if (!list.length) queueMicrotask(() => itemsCache.delete(itemsKeyOf(k)));
      return list;
    });
    if (!(got instanceof Promise)) return got;
    return got.catch((e) => {
      if (seq === itemsSeq) setItemsError(e);
      return undefined;
    });
  });
  /** Fetches the episode list again, skipping the cache. */
  const reloadItems = () => {
    const k = itemsKey();
    if (k) itemsCache.delete(itemsKeyOf(k));
    return refetchItems();
  };
  // No key (switching sources, or no match): no episodes, rather than the last source's.
  const items = withState(
    () => (itemsKey() ? itemsRes() : undefined),
    () => !!itemsKey() && itemsRes.loading,
    () => (itemsKey() ? itemsError() : undefined),
  );
  // Switching to another source (or show) drops the last one's errors straight away.
  createEffect(on(matchKey, () => {
    setMatchError(undefined);
    setItemsError(undefined);
  }, { defer: true }));

  /** Source items by whole number (AniList progress is whole episodes/chapters). */
  const byNumber = createMemo(() => {
    const map = new Map<number, SourceItem>();
    for (const i of items() ?? []) if (!map.has(Math.floor(i.number))) map.set(Math.floor(i.number), i);
    return map;
  });

  return { kind, exts, usable, ext, setExt, isDub, match, rematch, setMatch, items, reloadItems, byNumber };
}

export type SourceState = ReturnType<typeof createSource>;
