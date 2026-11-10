import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal, on, onCleanup, onMount } from "solid-js";
import { A } from "@solidjs/router";
import { createVirtualizer } from "@tanstack/solid-virtual";

import CoverCard, { FORMAT_LABEL } from "../components/CoverCard";
import { Search } from "../lib/icons";
import { getLibrary, inTauri, onLibraryChanged } from "../lib/api";
import { fmtCount } from "../lib/format";
import { homeStore } from "../lib/store";
import type { LibraryEntry, LibraryKind, ListStatus } from "../lib/types";

const STATUS_ORDER: ListStatus[] = ["CURRENT", "PLANNING", "COMPLETED", "PAUSED", "DROPPED", "REPEATING"];
const STATUS_LABEL: Record<LibraryKind, Record<ListStatus, string>> = {
  ANIME: { CURRENT: "Watching", PLANNING: "Planning", COMPLETED: "Completed", PAUSED: "Paused", DROPPED: "Dropped", REPEATING: "Rewatching" },
  MANGA: { CURRENT: "Reading", PLANNING: "Planning", COMPLETED: "Completed", PAUSED: "Paused", DROPPED: "Dropped", REPEATING: "Rereading" },
};

type SortId = "updated" | "title" | "score" | "progress";
const SORTS: { id: SortId; label: string }[] = [
  { id: "updated", label: "Recently updated" },
  { id: "title", label: "Title" },
  { id: "score", label: "Your score" },
  { id: "progress", label: "Progress" },
];

// Grid metrics, on the 8px cell grid.
const MIN_CARD = 152;
const GAP = 24;
const ROW_GAP = 32;
const TEXT_BLOCK = 60; // title (2 lines) + meta

const LETTERS = "#ABCDEFGHIJKLMNOPQRSTUVWXYZ".split("");
const letterOf = (title: string) => {
  const c = title.trim().charAt(0).toUpperCase();
  return c >= "A" && c <= "Z" ? c : "#";
};
const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

// View state survives navigating away and back, per kind.
const views = {
  ANIME: makeView(),
  MANGA: makeView(),
};
function makeView() {
  const [tab, setTab] = createSignal<ListStatus>("CURRENT");
  const [sort, setSort] = createSignal<SortId>("updated");
  const [format, setFormat] = createSignal("ALL");
  const [query, setQuery] = createSignal("");
  return { tab, setTab, sort, setSort, format, setFormat, query, setQuery };
}

export default function Library(props: { kind: LibraryKind }) {
  const v = views[props.kind];
  const { account } = homeStore;
  const [entries, { refetch }] = createResource(() => props.kind, getLibrary);

  let unlisten: (() => void) | undefined;
  void onLibraryChanged(() => void refetch()).then((u) => (unlisten = u));
  onCleanup(() => unlisten?.());

  const labels = () => STATUS_LABEL[props.kind];
  const noun = () => (props.kind === "ANIME" ? "shows" : "series");

  const counts = createMemo(() => {
    const c: Partial<Record<ListStatus, number>> = {};
    for (const e of entries() ?? []) c[e.status] = (c[e.status] ?? 0) + 1;
    return c;
  });
  const tabs = createMemo(() => STATUS_ORDER.filter((s) => s !== "REPEATING" || (counts()[s] ?? 0) > 0));
  const formats = createMemo(() =>
    [...new Set((entries() ?? []).map((e) => e.format).filter((f): f is string => !!f))].sort(),
  );

  const visible = createMemo<LibraryEntry[]>(() => {
    const q = v.query().trim().toLowerCase();
    const fmt = v.format();
    const list = (entries() ?? []).filter(
      (e) =>
        e.status === v.tab() &&
        (fmt === "ALL" || e.format === fmt) &&
        (!q || e.title.toLowerCase().includes(q) || (e.titleEnglish?.toLowerCase().includes(q) ?? false)),
    );
    const ratio = (e: LibraryEntry) => (e.total ? e.progress / e.total : 0);
    switch (v.sort()) {
      case "title":
        return list.sort((a, b) => collator.compare(a.title, b.title));
      case "score":
        return list.sort((a, b) => (b.score ?? -1) - (a.score ?? -1) || collator.compare(a.title, b.title));
      case "progress":
        return list.sort((a, b) => ratio(b) - ratio(a) || collator.compare(a.title, b.title));
      default:
        return list.sort((a, b) => (a.updatedAt < b.updatedAt ? 1 : a.updatedAt > b.updatedAt ? -1 : 0));
    }
  });

  // ── Virtualised grid: only rows near the viewport exist in the DOM.
  let gridEl!: HTMLDivElement;
  let headEl!: HTMLDivElement;
  const [scrollEl, setScrollEl] = createSignal<HTMLElement | null>(null);
  const [width, setWidth] = createSignal(0);
  const [margin, setMargin] = createSignal(0);
  const [headH, setHeadH] = createSignal(0);

  onMount(() => {
    setScrollEl(gridEl.closest<HTMLElement>(".main"));
    const ro = new ResizeObserver(() => {
      setWidth(gridEl.clientWidth);
      setMargin(gridEl.offsetTop);
      setHeadH(headEl.offsetHeight);
    });
    ro.observe(gridEl);
    ro.observe(headEl);
    onCleanup(() => ro.disconnect());
  });

  const cols = () => Math.max(1, Math.floor((width() + GAP) / (MIN_CARD + GAP)));
  const cardW = () => (width() - GAP * (cols() - 1)) / cols();
  const rowH = () => Math.round(cardW() * 1.5 + TEXT_BLOCK + ROW_GAP);
  const rowCount = () => Math.ceil(visible().length / cols());

  const virt = createVirtualizer({
    get count() {
      return rowCount();
    },
    getScrollElement: () => scrollEl(),
    estimateSize: () => rowH(),
    overscan: 3,
    get scrollMargin() {
      return margin();
    },
    get scrollPaddingStart() {
      return headH() + 16;
    },
  });
  createEffect(on([rowH, cols], () => virt.measure(), { defer: true }));

  // Back to the top when the list itself changes (tab, filter, sort).
  createEffect(
    on([v.tab, v.sort, v.format, v.query], () => scrollEl()?.scrollTo({ top: 0 }), { defer: true }),
  );

  const rows = () => virt.getVirtualItems().map((r) => r.index);
  const rowItems = (row: number) => visible().slice(row * cols(), row * cols() + cols());

  // ── A–Z rail (title sort): each letter's height follows how many shows it holds.
  const letterIndex = createMemo(() => {
    const first = new Map<string, number>();
    const count = new Map<string, number>();
    visible().forEach((e, i) => {
      const l = letterOf(e.title);
      if (!first.has(l)) first.set(l, i);
      count.set(l, (count.get(l) ?? 0) + 1);
    });
    return { first, count };
  });
  /** Letter of the first row under the sticky header. */
  const currentLetter = () => {
    virt.getVirtualItems(); // re-evaluate on scroll
    const top = (virt.scrollOffset ?? 0) - margin() + headH();
    const item = visible()[Math.max(0, Math.floor(top / rowH())) * cols()];
    return item ? letterOf(item.title) : null;
  };
  const jump = (l: string) => {
    const i = letterIndex().first.get(l);
    if (i !== undefined) virt.scrollToIndex(Math.floor(i / cols()), { align: "start" });
  };

  const notConnected = () => inTauri && account() && !account()!.viewer;

  return (
    <div class="library">
      <div class="lib-head" ref={headEl}>
        <div class="lib-titlebar">
          <h1 class="page-title">{props.kind === "ANIME" ? "Library" : "Manga"}</h1>
          <Show when={entries()}>
            <p class="lib-total num">
              {fmtCount(entries()!.length)} {noun()} on your lists
            </p>
          </Show>
        </div>

        <div class="tabs" role="tablist" aria-label="List">
          <For each={tabs()}>
            {(s) => (
              <button
                type="button"
                role="tab"
                class="tab"
                aria-selected={v.tab() === s}
                onClick={() => v.setTab(s)}
              >
                {labels()[s]}
                <span class="tab-count num">{fmtCount(counts()[s] ?? 0)}</span>
              </button>
            )}
          </For>
        </div>

        <div class="toolbar">
          <label class="search search-sm">
            <Search size={16} stroke-width={1.75} aria-hidden="true" />
            <span class="sr-only">Filter by title</span>
            <input
              type="search"
              placeholder={`Filter ${labels()[v.tab()].toLowerCase()}`}
              value={v.query()}
              onInput={(e) => v.setQuery(e.currentTarget.value)}
              spellcheck={false}
            />
          </label>
          <label class="select">
            <span class="select-label">Sort</span>
            <select value={v.sort()} onChange={(e) => v.setSort(e.currentTarget.value as SortId)}>
              <For each={SORTS}>{(s) => <option value={s.id}>{s.label}</option>}</For>
            </select>
          </label>
          <label class="select">
            <span class="select-label">Format</span>
            <select value={v.format()} onChange={(e) => v.setFormat(e.currentTarget.value)}>
              <option value="ALL">All formats</option>
              <For each={formats()}>{(f) => <option value={f}>{FORMAT_LABEL[f] ?? f}</option>}</For>
            </select>
          </label>
          <p class="toolbar-count num" aria-live="polite">
            {fmtCount(visible().length)} shown
          </p>
        </div>
      </div>

      <div
        class="lib-body"
        data-rail={v.sort() === "title" && visible().length > 0 ? "" : undefined}
        style={{ "--lib-head-h": `${headH()}px` }}
      >
        <div class="lib-grid-wrap">
          <Switch>
            <Match when={notConnected()}>
              <div class="empty-block">
                <p>Connect AniList to see your lists here.</p>
                <A href="/settings" class="btn btn-primary">Connect AniList</A>
              </div>
            </Match>
            <Match when={entries.error}>
              <div class="empty-block">
                <p class="inline-error">Couldn't load your library: {String(entries.error)}</p>
                <button type="button" class="btn" onClick={() => void refetch()}>Try again</button>
              </div>
            </Match>
            <Match when={entries() && visible().length === 0 && v.query().trim()}>
              <div class="empty-block">
                <p>No {labels()[v.tab()].toLowerCase()} {noun()} match “{v.query().trim()}”.</p>
                <button type="button" class="btn" onClick={() => v.setQuery("")}>Clear filter</button>
              </div>
            </Match>
            <Match when={entries() && visible().length === 0}>
              <p class="empty">Nothing in {labels()[v.tab()]}{v.format() !== "ALL" ? ` with this format` : ""}.</p>
            </Match>
            <Match when={entries.loading && !entries()}>
              <div class="lib-skeleton" aria-busy="true" aria-label="Loading your library">
                <For each={Array.from({ length: 12 })}>{() => <div class="sk-cover" />}</For>
              </div>
            </Match>
          </Switch>

          <div ref={gridEl} class="lib-grid" style={{ height: `${virt.getTotalSize()}px` }}>
            <For each={rows()}>
              {(row) => (
                <div
                  class="lib-row"
                  style={{
                    transform: `translateY(${row * rowH()}px)`,
                    height: `${rowH() - ROW_GAP}px`,
                    "grid-template-columns": `repeat(${cols()}, minmax(0, 1fr))`,
                  }}
                >
                  <For each={rowItems(row)}>{(e) => <CoverCard entry={e} kind={props.kind} />}</For>
                </div>
              )}
            </For>
          </div>
        </div>

        <Show when={v.sort() === "title" && visible().length > 0}>
          <nav class="alpha-rail" aria-label="Jump to letter">
            <For each={LETTERS}>
              {(l) => {
                const n = () => letterIndex().count.get(l) ?? 0;
                return (
                  <button
                    type="button"
                    class="alpha"
                    style={{ "flex-grow": n() }}
                    disabled={n() === 0}
                    data-current={currentLetter() === l ? "" : undefined}
                    onClick={() => jump(l)}
                    title={`${l}: ${n()}`}
                    aria-label={`${l === "#" ? "Numbers and symbols" : l}, ${n()} ${noun()}`}
                  >
                    {l}
                  </button>
                );
              }}
            </For>
          </nav>
        </Show>
      </div>
    </div>
  );
}
