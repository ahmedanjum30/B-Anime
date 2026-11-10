import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { A, useSearchParams } from "@solidjs/router";

import { FORMAT_LABEL } from "../components/CoverCard";
import { LIST_STATUS_LABEL } from "../components/ListEditor";
import { Plus, Search } from "../lib/icons";
import { browse, errorText, inTauri, saveEntry } from "../lib/api";
import { homeStore } from "../lib/store";
import type { DiscoverSection, LibraryKind, MediaCard } from "../lib/types";

const SECTIONS: { id: DiscoverSection; label: string }[] = [
  { id: "trending", label: "Trending now" },
  { id: "this_season", label: "This season" },
  { id: "next_season", label: "Next season" },
  { id: "all_time", label: "All-time popular" },
];
const GENRES = [
  "Action", "Adventure", "Comedy", "Drama", "Ecchi", "Fantasy", "Horror", "Mahou Shoujo", "Mecha", "Music",
  "Mystery", "Psychological", "Romance", "Sci-Fi", "Slice of Life", "Sports", "Supernatural", "Thriller",
];
const FORMATS: Record<LibraryKind, string[]> = {
  ANIME: ["TV", "TV_SHORT", "MOVIE", "SPECIAL", "OVA", "ONA", "MUSIC"],
  MANGA: ["MANGA", "NOVEL", "ONE_SHOT"],
};
const THIS_YEAR = new Date().getFullYear();
const YEARS = Array.from({ length: THIS_YEAR + 2 - 1970 }, (_, i) => THIS_YEAR + 1 - i);

// Filters survive navigating away and back.
const [section, setSection] = createSignal<DiscoverSection>("trending");
const [kind, setKind] = createSignal<LibraryKind>("ANIME");
const [genre, setGenre] = createSignal("");
const [format, setFormat] = createSignal("");
const [year, setYear] = createSignal("");

export default function Discover() {
  const { account, notify } = homeStore;
  const [params, setParams] = useSearchParams();
  const urlQuery = () => (typeof params.q === "string" ? params.q : "");

  // Typing updates the box at once and the search (and URL) after a short pause.
  const [draft, setDraft] = createSignal(urlQuery());
  const [query, setQuery] = createSignal(urlQuery().trim());
  let typing: number | undefined;
  const onType = (v: string) => {
    setDraft(v);
    clearTimeout(typing);
    typing = window.setTimeout(() => {
      setQuery(v.trim());
      setParams({ q: v.trim() || undefined }, { replace: true });
    }, 350);
  };
  onCleanup(() => clearTimeout(typing));
  createEffect(on(urlQuery, (q) => {
    if (q.trim() !== query()) {
      setDraft(q);
      setQuery(q.trim());
    }
  }, { defer: true }));

  const searching = () => query().length > 0;
  const seasonal = () => !searching() && (section() === "this_season" || section() === "next_season");

  // Results accumulate page by page; any filter change starts over.
  const [items, setItems] = createSignal<MediaCard[]>([]);
  const [page, setPage] = createSignal(0);
  const [hasNext, setHasNext] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  let seq = 0;

  const filterKey = createMemo(() =>
    JSON.stringify([query(), section(), kind(), genre(), format(), seasonal() ? "" : year()]),
  );

  const load = async (next: number) => {
    const mine = ++seq;
    setLoading(true);
    setError(null);
    try {
      const r = await browse({
        search: query() || undefined,
        section: section(),
        kind: kind(),
        genre: genre() || undefined,
        format: format() || undefined,
        year: !seasonal() && year() ? Number(year()) : undefined,
        page: next,
      });
      if (mine !== seq) return;
      setItems((prev) => {
        if (next === 1) return r.items;
        const seen = new Set(prev.map((m) => m.id));
        return [...prev, ...r.items.filter((m) => !seen.has(m.id))];
      });
      setPage(next);
      setHasNext(r.hasNext);
    } catch (e) {
      if (mine === seq) setError(errorText(e));
    } finally {
      if (mine === seq) setLoading(false);
    }
  };
  createEffect(on(filterKey, () => {
    setItems([]);
    void load(1);
    document.querySelector(".main")?.scrollTo({ top: 0 });
  }));

  // Infinite scroll: load the next page as the end of the grid comes into view.
  let sentinel!: HTMLDivElement;
  let searchInput!: HTMLInputElement;
  onMount(() => {
    if (urlQuery()) searchInput.focus();
    const io = new IntersectionObserver(
      ([e]) => {
        if (e.isIntersecting && hasNext() && !loading() && !error()) void load(page() + 1);
      },
      { root: document.querySelector(".main"), rootMargin: "600px" },
    );
    io.observe(sentinel);
    onCleanup(() => io.disconnect());
  });

  const canAdd = () => !inTauri || !!account()?.viewer;
  const addToPlanning = async (m: MediaCard) => {
    setItems((list) => list.map((x) => (x.id === m.id ? { ...x, listStatus: "PLANNING" } : x)));
    try {
      await saveEntry(m.id, { status: "PLANNING" });
      notify(`Added ${m.title} to Planning.`);
    } catch (e) {
      setItems((list) => list.map((x) => (x.id === m.id ? { ...x, listStatus: null } : x)));
      notify(`Couldn't add ${m.title}: ${errorText(e)}`);
    }
  };

  const clearFilters = () => {
    setGenre("");
    setFormat("");
    setYear("");
  };

  return (
    <div class="library">
      <div class="lib-head">
        <div class="lib-titlebar">
          <h1 class="page-title">Discover</h1>
        </div>

        <label class="search search-lg">
          <Search size={18} stroke-width={1.75} aria-hidden="true" />
          <span class="sr-only">Search AniList</span>
          <input
            ref={searchInput}
            type="search"
            placeholder={`Search all ${kind() === "ANIME" ? "anime" : "manga"} on AniList`}
            value={draft()}
            onInput={(e) => onType(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && draft()) {
                e.preventDefault();
                clearTimeout(typing);
                setDraft("");
                setQuery("");
                setParams({ q: undefined }, { replace: true });
              }
            }}
            spellcheck={false}
          />
        </label>

        <Show
          when={!searching()}
          fallback={<div class="tabs"><span class="tab" aria-selected="true">Results for “{query()}”</span></div>}
        >
          <div class="tabs" role="tablist" aria-label="Browse">
            <For each={SECTIONS}>
              {(s) => (
                <button type="button" role="tab" class="tab" aria-selected={section() === s.id} onClick={() => setSection(s.id)}>
                  {s.label}
                </button>
              )}
            </For>
          </div>
        </Show>

        <div class="toolbar">
          <div class="segmented" role="group" aria-label="Type">
            <button type="button" aria-pressed={kind() === "ANIME"} onClick={() => { setKind("ANIME"); setFormat(""); }}>Anime</button>
            <button type="button" aria-pressed={kind() === "MANGA"} onClick={() => { setKind("MANGA"); setFormat(""); }}>Manga</button>
          </div>
          <label class="select">
            <span class="select-label">Genre</span>
            <select value={genre()} onChange={(e) => setGenre(e.currentTarget.value)}>
              <option value="">Any</option>
              <For each={GENRES}>{(g) => <option value={g}>{g}</option>}</For>
            </select>
          </label>
          <label class="select">
            <span class="select-label">Format</span>
            <select value={format()} onChange={(e) => setFormat(e.currentTarget.value)}>
              <option value="">Any</option>
              <For each={FORMATS[kind()]}>{(f) => <option value={f}>{FORMAT_LABEL[f] ?? f}</option>}</For>
            </select>
          </label>
          <Show when={!seasonal()}>
            <label class="select">
              <span class="select-label">Year</span>
              <select value={year()} onChange={(e) => setYear(e.currentTarget.value)}>
                <option value="">Any</option>
                <For each={YEARS}>{(y) => <option value={y}>{y}</option>}</For>
              </select>
            </label>
          </Show>
          <Show when={genre() || format() || (!seasonal() && year())}>
            <button type="button" class="btn btn-quiet btn-sm" onClick={clearFilters}>Clear filters</button>
          </Show>
        </div>
      </div>

      <div class="lib-body">
        <div>
          <Switch>
            <Match when={error() && items().length === 0}>
              <div class="empty-block">
                <p class="inline-error">Couldn't reach AniList: {error()}</p>
                <button type="button" class="btn" onClick={() => void load(1)}>Try again</button>
              </div>
            </Match>
            <Match when={!loading() && items().length === 0 && page() > 0}>
              <p class="empty">
                {searching() ? `Nothing on AniList matches “${query()}”.` : "Nothing matches these filters."}
              </p>
            </Match>
          </Switch>

          <div class="discover-grid">
            <For each={items()}>
              {(m) => <DiscoverCard media={m} canAdd={canAdd()} onAdd={addToPlanning} />}
            </For>
            <Show when={loading()}>
              <For each={Array.from({ length: items().length ? 6 : 18 })}>{() => <div class="sk-cover" />}</For>
            </Show>
          </div>

          <Show when={error() && items().length > 0}>
            <div class="empty-block">
              <p class="inline-error">Couldn't load more: {error()}</p>
              <button type="button" class="btn" onClick={() => void load(page() + 1)}>Try again</button>
            </div>
          </Show>
          <div ref={sentinel} class="sentinel" aria-hidden="true" />
        </div>
      </div>
    </div>
  );
}

function DiscoverCard(props: { media: MediaCard; canAdd: boolean; onAdd: (m: MediaCard) => void }) {
  const m = () => props.media;
  const meta = () =>
    [
      m().format ? FORMAT_LABEL[m().format!] ?? m().format : null,
      m().seasonYear,
      m().averageScore ? `${m().averageScore}%` : null,
    ]
      .filter(Boolean)
      .join(" · ");

  return (
    <div class="discover-card">
      <A href={`/show/${m().id}`} class="cover-card" title={m().title}>
        <div class="cover" style={{ "background-color": m().color ?? "var(--z3)" }}>
          <Show when={m().coverUrl}>
            {(src) => <img src={src()} alt="" loading="lazy" decoding="async" draggable={false} />}
          </Show>
          <Show when={m().listStatus}>
            {(s) => <span class="list-badge">{LIST_STATUS_LABEL[m().kind][s()]}</span>}
          </Show>
        </div>
        <p class="cover-title">{m().title}</p>
        <p class="cover-meta num">
          <span class="cover-meta-main">{meta()}</span>
        </p>
      </A>
      <Show when={props.canAdd && !m().listStatus}>
        <button type="button" class="quick-add" onClick={() => props.onAdd(m())} aria-label={`Add ${m().title} to Planning`}>
          <Plus size={14} stroke-width={2.25} aria-hidden="true" /> Planning
        </button>
      </Show>
    </div>
  );
}
