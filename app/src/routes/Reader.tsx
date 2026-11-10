import { For, Show, createEffect, createMemo, createResource, createSignal, on, onCleanup, onMount } from "solid-js";
import { useNavigate, useParams, useSearchParams } from "@solidjs/router";

import { ArrowLeft, ArrowLeftRight, ChevronLeft, ChevronRight, Columns2, File, Rows3 } from "../lib/icons";
import { errorText, getMedia, getPages, inTauri, saveEntry } from "../lib/api";
import { applyRules } from "../lib/entry";
import { createSource } from "../lib/source";
import { homeStore } from "../lib/store";

type Mode = "single" | "double" | "strip";
const MODES: { id: Mode; label: string; icon: typeof File }[] = [
  { id: "single", label: "Single page", icon: File },
  { id: "double", label: "Two pages", icon: Columns2 },
  { id: "strip", label: "Long strip", icon: Rows3 },
];

const load = <T extends string>(key: string, fallback: T, allowed: readonly T[]): T => {
  try {
    const v = localStorage.getItem(key) as T | null;
    return v && allowed.includes(v) ? v : fallback;
  } catch {
    return fallback;
  }
};
const save = (key: string, v: string) => {
  try {
    localStorage.setItem(key, v);
  } catch {
    /* not essential */
  }
};

const fmtNum = (n: number) => (Number.isInteger(n) ? String(n) : n.toFixed(1).replace(/\.0$/, ""));

export default function Reader() {
  const params = useParams();
  const [search] = useSearchParams();
  const navigate = useNavigate();
  const { notify } = homeStore;

  const mediaId = () => Number(params.id);
  const chapterNo = () => Number(params.ch);
  const extParam = () => (typeof search.ext === "string" ? search.ext : undefined);

  const [page] = createResource(mediaId, getMedia);
  const media = () => page()?.media;
  const source = createSource(media, extParam);
  const chapters = () => source.items() ?? [];
  const index = createMemo(() => chapters().findIndex((c) => c.number === chapterNo()));
  const chapter = () => chapters()[index()];
  const [pages] = createResource(
    () => (chapter() && source.ext() ? { ext: source.ext()!, id: chapter()!.id } : null),
    (k) => getPages(k.ext, k.id),
  );

  const [mode, setModeSignal] = createSignal<Mode>(load("banime.reader.mode", "single", ["single", "double", "strip"] as const));
  const [rtl, setRtlSignal] = createSignal(load("banime.reader.dir", "rtl", ["rtl", "ltr"] as const) === "rtl");
  const setMode = (m: Mode) => {
    setModeSignal(m);
    save("banime.reader.mode", m);
  };
  const toggleDir = () => {
    setRtlSignal(!rtl());
    save("banime.reader.dir", rtl() ? "rtl" : "ltr");
  };

  /** Set once this chapter has been recorded as read. */
  let marked = false;

  // Page position in paged modes; spreads advance by two.
  const [at, setAt] = createSignal(0);
  const step = () => (mode() === "double" ? 2 : 1);
  const count = () => pages()?.length ?? 0;
  createEffect(on(() => `${mediaId()}:${chapterNo()}`, () => {
    setAt(0);
    marked = false;
    document.querySelector(".reader-strip")?.scrollTo({ top: 0 });
  }));

  const goChapter = (offset: number) => {
    const c = chapters()[index() + offset];
    if (c) navigate(`/read/${mediaId()}/${c.number}?ext=${encodeURIComponent(source.ext() ?? "")}`, { replace: true });
  };
  const hasPrevCh = () => index() > 0;
  const hasNextCh = () => index() >= 0 && index() < chapters().length - 1;

  const forward = () => {
    if (at() + step() < count()) setAt(at() + step());
    else if (hasNextCh()) goChapter(1);
  };
  const backward = () => {
    if (at() > 0) setAt(Math.max(0, at() - step()));
    else if (hasPrevCh()) goChapter(-1);
  };
  // Arrow keys follow reading direction: in right-to-left, ← moves forward.
  const left = () => (rtl() && mode() !== "strip" ? forward() : backward());
  const right = () => (rtl() && mode() !== "strip" ? backward() : forward());

  // ── Progress: finishing a chapter's last page counts it as read.
  const markRead = async () => {
    if (marked || !inTauri) return;
    marked = true;
    const n = Math.floor(chapterNo());
    const entry = page()?.listEntry ?? null;
    if (entry && entry.progress >= n) return;
    try {
      await saveEntry(mediaId(), applyRules(entry, { progress: n }, media()?.chapters ?? null));
      notify(`Chapter ${fmtNum(chapterNo())} marked read on AniList.`);
    } catch (e) {
      notify(`Couldn't update AniList: ${errorText(e)}`);
    }
  };
  createEffect(() => {
    if (mode() !== "strip" && count() > 0 && at() + step() >= count()) void markRead();
  });

  // Preload the next few pages in paged modes.
  createEffect(() => {
    const list = pages();
    if (!list || mode() === "strip") return;
    for (const url of list.slice(at() + step(), at() + step() + 4)) {
      const img = new Image();
      img.decoding = "async";
      img.src = url;
    }
  });

  // Long strip: reaching the end marker counts the chapter as read.
  let endMarker: HTMLDivElement | undefined;
  createEffect(() => {
    if (mode() !== "strip" || !endMarker || !pages()) return;
    const io = new IntersectionObserver(([e]) => e.isIntersecting && void markRead(), { threshold: 0.5 });
    io.observe(endMarker);
    onCleanup(() => io.disconnect());
  });

  const onKey = (e: KeyboardEvent) => {
    if ((e.target as HTMLElement)?.closest("input, select, textarea")) return;
    const handled = (() => {
      switch (e.key) {
        case "ArrowLeft": if (mode() !== "strip") { left(); return true; } return false;
        case "ArrowRight": if (mode() !== "strip") { right(); return true; } return false;
        case " ": case "PageDown": if (mode() !== "strip") { forward(); return true; } return false;
        case "PageUp": if (mode() !== "strip") { backward(); return true; } return false;
        case "Home": setAt(0); return mode() !== "strip";
        case "End": setAt(Math.max(0, count() - step())); return mode() !== "strip";
        case "Escape": navigate(`/show/${mediaId()}`); return true;
      }
      return false;
    })();
    if (handled) e.preventDefault();
  };
  onMount(() => {
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  const visible = () => {
    const list = pages() ?? [];
    const shown = list.slice(at(), at() + step());
    return mode() === "double" && rtl() ? [...shown].reverse() : shown;
  };
  const failure = () => {
    if (page.error) return `Couldn't load the series: ${errorText(page.error)}`;
    if (source.exts() && !source.usable().length) return "No manga extension is installed or turned on.";
    if (source.match.error) return errorText(source.match.error);
    if (source.match()?.status === "unmatched") return "This series isn't matched to a source yet. Pick one on its page.";
    if (source.items.error) return errorText(source.items.error);
    if (source.items() && index() < 0) return `Chapter ${fmtNum(chapterNo())} isn't on this source.`;
    if (pages.error) return errorText(pages.error);
    if (pages() && pages()!.length === 0) return "This chapter has no pages on the source.";
    return null;
  };

  return (
    <div class="reader" data-mode={mode()}>
      <header class="reader-bar">
        <button type="button" class="back-link" onClick={() => navigate(`/show/${mediaId()}`)}>
          <ArrowLeft size={16} stroke-width={2} aria-hidden="true" /> Back
        </button>
        <div class="reader-title">
          <p class="player-show">{media()?.title.userPreferred}</p>
          <p class="player-ep num">
            Chapter {fmtNum(chapterNo())}
            <Show when={chapter()?.title && !/^chapter\s*[\d.]+$/i.test(chapter()!.title!)}> · {chapter()!.title}</Show>
          </p>
        </div>
        <div class="reader-tools">
          <Show when={mode() !== "strip" && count()}>
            <span class="reader-page num" aria-live="polite">
              {Math.min(at() + 1, count())}{step() === 2 && at() + 2 <= count() ? `–${at() + 2}` : ""} / {count()}
            </span>
          </Show>
          <div class="segmented" role="group" aria-label="Layout">
            <For each={MODES}>
              {(m) => (
                <button type="button" aria-pressed={mode() === m.id} onClick={() => setMode(m.id)} title={m.label} aria-label={m.label}>
                  <m.icon size={16} stroke-width={2} aria-hidden="true" />
                </button>
              )}
            </For>
          </div>
          <Show when={mode() !== "strip"}>
            <button type="button" class="btn btn-sm" onClick={toggleDir} title="Reading direction">
              <ArrowLeftRight size={14} stroke-width={2} aria-hidden="true" /> {rtl() ? "Right to left" : "Left to right"}
            </button>
          </Show>
          <button type="button" class="icon-btn" disabled={!hasPrevCh()} onClick={() => goChapter(-1)} aria-label="Previous chapter">
            <ChevronLeft size={18} stroke-width={2} />
          </button>
          <button type="button" class="icon-btn" disabled={!hasNextCh()} onClick={() => goChapter(1)} aria-label="Next chapter">
            <ChevronRight size={18} stroke-width={2} />
          </button>
        </div>
      </header>

      <Show when={failure()}>
        {(msg) => (
          <div class="player-message" role="alert">
            <p>{msg()}</p>
            <button type="button" class="btn" onClick={() => navigate(`/show/${mediaId()}`)}>Back to the series</button>
          </div>
        )}
      </Show>
      <Show when={!failure() && !pages()}>
        <div class="player-spinner" aria-label="Loading" />
      </Show>

      <Show when={pages()?.length && mode() !== "strip"}>
        <div class="reader-stage">
          <For each={visible()}>
            {(url) => <img class="reader-page-img" src={url} alt="" decoding="async" draggable={false} />}
          </For>
          {/* Click zones: left and right halves turn pages in reading direction. */}
          <button type="button" class="turn turn-left" onClick={left} aria-label={rtl() ? "Next page" : "Previous page"} />
          <button type="button" class="turn turn-right" onClick={right} aria-label={rtl() ? "Previous page" : "Next page"} />
        </div>
      </Show>

      <Show when={pages()?.length && mode() === "strip"}>
        <div class="reader-strip">
          <For each={pages()}>
            {(url, i) => <img class="strip-img" src={url} alt={`Page ${i() + 1}`} loading={i() < 3 ? "eager" : "lazy"} decoding="async" draggable={false} />}
          </For>
          <div ref={endMarker} class="strip-end">
            <p>End of chapter {fmtNum(chapterNo())}</p>
            <Show when={hasNextCh()}>
              <button type="button" class="btn btn-primary" onClick={() => goChapter(1)}>
                Next: chapter {fmtNum(chapters()[index() + 1]?.number ?? 0)}
              </button>
            </Show>
          </div>
        </div>
      </Show>
    </div>
  );
}
