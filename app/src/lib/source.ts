import { createEffect, createMemo, createResource, createSignal } from "solid-js";

import { findSource, inTauri, listExtensions, sourceItems } from "./api";
import type { ExtKind, InstalledExtension, Media, SourceItem } from "./types";

const LAST_KEY = (kind: ExtKind) => `banime.source.${kind}`;
const remember = (kind: ExtKind, id: string) => {
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
    remember(kind(), id);
    setExtSignal(id);
  };

  const matchKey = createMemo(() => {
    const m = media();
    const e = ext();
    const d = kind() === "anime" && dub();
    return m && e ? { id: m.id, ext: e, dub: d } : null;
  }, null, { equals: (a, b) => a?.id === b?.id && a?.ext === b?.ext && a?.dub === b?.dub });
  const [match, { refetch: rematch, mutate: setMatch }] = createResource(matchKey, (k) => {
    const m = media()!;
    return findSource(k.id, k.ext, searchTitles(m), m.seasonYear ?? m.startDate.year, k.dub);
  });
  /** Whether the current match is for the dub (what the player should be told). */
  const isDub = () => matchKey()?.dub ?? false;

  const itemsKey = createMemo(() => {
    const mt = match();
    const e = ext();
    return mt?.status === "matched" && e ? { ext: e, sourceId: mt.sourceId } : null;
  }, null, { equals: (a, b) => a?.ext === b?.ext && a?.sourceId === b?.sourceId });
  const [items, { refetch: reloadItems }] = createResource(itemsKey, (k) => sourceItems(k.ext, k.sourceId));

  /** Source items by whole number (AniList progress is whole episodes/chapters). */
  const byNumber = createMemo(() => {
    const map = new Map<number, SourceItem>();
    for (const i of items() ?? []) if (!map.has(Math.floor(i.number))) map.set(Math.floor(i.number), i);
    return map;
  });

  return { kind, exts, usable, ext, setExt, isDub, match, rematch, setMatch, items, reloadItems, byNumber };
}

export type SourceState = ReturnType<typeof createSource>;
