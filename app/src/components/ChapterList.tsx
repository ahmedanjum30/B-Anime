import { For, Show, createEffect, createSignal, on, type JSX } from "solid-js";

import { BookOpen } from "../lib/icons";
import type { EntrySaver } from "../lib/entry";
import type { SourceItem } from "../lib/types";

const CHUNK = 100;
const fmtNum = (n: number) => (Number.isInteger(n) ? String(n) : n.toFixed(1).replace(/\.0$/, ""));

/** Chapters from the manga source, as a listings column with read state and Read buttons. */
export default function ChapterList(props: {
  items: SourceItem[] | undefined;
  loading: boolean;
  saver: EntrySaver;
  total: number | null;
  panel?: JSX.Element;
  onRead: (chapter: SourceItem) => void;
}) {
  const progress = () => props.saver.entry()?.progress ?? 0;
  const list = () => props.items ?? [];
  const chunks = () => Math.max(1, Math.ceil(list().length / CHUNK));
  const [chunk, setChunk] = createSignal(0);
  // Open on the range holding the next unread chapter.
  createEffect(on(() => props.items, (items) => {
    const i = (items ?? []).findIndex((c) => Math.floor(c.number) > progress());
    setChunk(Math.max(0, Math.floor((i < 0 ? 0 : i) / CHUNK)));
  }));
  const rows = () => list().slice(chunk() * CHUNK, (chunk() + 1) * CHUNK);
  const nextUnread = () => list().find((c) => Math.floor(c.number) > progress());

  return (
    <section class="section" aria-labelledby="chs-h">
      <h2 id="chs-h" class="section-head">
        Chapters
        <small class="num">
          {progress()} read{props.total ? ` of ${props.total}` : ""}
          {props.items ? ` · ${props.items.length} on this source` : ""}
        </small>
      </h2>
      {props.panel}

      <Show when={props.loading && !props.items}>
        <p class="empty">Loading chapters…</p>
      </Show>
      <Show when={props.items && props.items.length === 0}>
        <p class="empty">This source lists no chapters.</p>
      </Show>

      <Show when={chunks() > 1}>
        <div class="tabs ep-ranges" role="tablist" aria-label="Chapter range">
          <For each={Array.from({ length: chunks() }, (_, i) => i)}>
            {(i) => (
              <button type="button" role="tab" class="tab num" aria-selected={chunk() === i} onClick={() => setChunk(i)}>
                {fmtNum(list()[i * CHUNK].number)}–{fmtNum(list()[Math.min(list().length, (i + 1) * CHUNK) - 1].number)}
              </button>
            )}
          </For>
        </div>
      </Show>

      <ol class="episodes">
        <For each={rows()}>
          {(c) => {
            const read = () => Math.floor(c.number) <= progress();
            const next = () => nextUnread() === c;
            return (
              <li class="ep-row" data-watched={read() ? "" : undefined} data-next={next() ? "" : undefined}>
                <span class="ep-num num">{fmtNum(c.number)}</span>
                <span class="ep-text">
                  <span class="ep-title">{c.title || `Chapter ${fmtNum(c.number)}`}</span>
                  <Show when={c.volume}><span class="ep-file num">Volume {c.volume}</span></Show>
                </span>
                <span class="ep-state">
                  <Show when={read()}><span class="slot-stamp">Read</span></Show>
                  <Show when={next()}><span class="ep-next">Up next</span></Show>
                </span>
                <span class="ep-action">
                  <button type="button" class="btn btn-sm is-play" classList={{ "btn-primary": next() }} onClick={() => props.onRead(c)}>
                    <BookOpen size={12} stroke-width={2.25} aria-hidden="true" /> Read
                  </button>
                </span>
              </li>
            );
          }}
        </For>
      </ol>
    </section>
  );
}
