import { For, Show, createEffect, createMemo, createSignal, on } from "solid-js";

import { homeStore } from "../lib/store";
import { inTauri } from "../lib/api";
import type { EntrySaver } from "../lib/entry";
import type { Media } from "../lib/types";

const CHUNK = 100;
const dateFmt = new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", hour12: false });

/** Episodes as a listings column: number, title, state, and one action per row. */
export default function EpisodeList(props: {
  media: Media;
  saver: EntrySaver;
  localFiles: { episode: number; fileName: string }[];
}) {
  const { account } = homeStore;
  const m = () => props.media;
  const progress = () => props.saver.entry()?.progress ?? 0;
  const canEdit = () => !inTauri || !!account()?.viewer;

  // Episode count: known total, else whatever has aired plus the next one.
  const count = () =>
    Math.max(m().episodes ?? 0, m().nextAiringEpisode?.episode ?? 0, progress(), m().streamingEpisodes.length);
  const released = () =>
    m().nextAiringEpisode ? m().nextAiringEpisode!.episode - 1 : m().status === "NOT_YET_RELEASED" ? 0 : count();

  const titles = createMemo(() => {
    const map = new Map<number, string>();
    for (const s of m().streamingEpisodes) {
      const match = s.title?.match(/episode\s+(\d+)\s*[-–:]?\s*(.*)$/i);
      if (match) map.set(Number(match[1]), match[2].trim());
    }
    return map;
  });
  const files = createMemo(() => new Map(props.localFiles.map((f) => [f.episode, f.fileName])));

  // Long shows page by 100, opening on the range with the next episode.
  const chunks = () => Math.max(1, Math.ceil(Math.min(count(), released() + 1) / CHUNK));
  const [chunk, setChunk] = createSignal(0);
  createEffect(on(() => m().id, () => setChunk(Math.min(Math.floor(progress() / CHUNK), Math.max(0, chunks() - 1)))));
  // Aired episodes plus the next one to air; the rest collapse into one line.
  const lastListed = () => Math.min(count(), released() + 1);
  const rows = () => {
    const start = chunk() * CHUNK + 1;
    const end = Math.min(lastListed(), start + CHUNK - 1);
    return Array.from({ length: Math.max(0, end - start + 1) }, (_, i) => start + i);
  };
  const unairedFrom = () => lastListed() + 1;
  const showUnaired = () => unairedFrom() <= count() && (chunk() + 1) * CHUNK >= unairedFrom();

  return (
    <Show when={count() > 0}>
      <section class="section" aria-labelledby="eps-h">
        <h2 id="eps-h" class="section-head">
          Episodes
          <small class="num">
            {progress()} of {m().episodes ?? released()} watched
            {m().nextAiringEpisode ? ` · ${released()} out so far` : ""}
          </small>
        </h2>

        <Show when={chunks() > 1}>
          <div class="tabs ep-ranges" role="tablist" aria-label="Episode range">
            <For each={Array.from({ length: chunks() }, (_, i) => i)}>
              {(i) => (
                <button type="button" role="tab" class="tab num" aria-selected={chunk() === i} onClick={() => setChunk(i)}>
                  {i * CHUNK + 1}–{Math.min(count(), (i + 1) * CHUNK)}
                </button>
              )}
            </For>
          </div>
        </Show>

        <ol class="episodes">
          <For each={rows()}>
            {(ep) => {
              const watched = () => ep <= progress();
              const aired = () => ep <= released();
              const next = () => ep === progress() + 1 && aired();
              const airing = () => m().nextAiringEpisode?.episode === ep;
              return (
                <li class="ep-row" data-watched={watched() ? "" : undefined} data-next={next() ? "" : undefined} data-unaired={!aired() ? "" : undefined}>
                  <span class="ep-num num">{ep}</span>
                  <span class="ep-text">
                    <span class="ep-title">{titles().get(ep) || `Episode ${ep}`}</span>
                    <Show when={files().get(ep)}>{(f) => <span class="ep-file">{f()}</span>}</Show>
                  </span>
                  <span class="ep-state">
                    <Show when={watched()}>
                      <span class="slot-stamp">Watched</span>
                    </Show>
                    <Show when={next()}>
                      <span class="ep-next">Up next</span>
                    </Show>
                    <Show when={airing()}>
                      <span class="num">Airs {dateFmt.format(new Date(m().nextAiringEpisode!.airingAt * 1000))}</span>
                    </Show>
                    <Show when={!aired() && !airing()}>Not aired yet</Show>
                  </span>
                  <span class="ep-action">
                    <Show when={canEdit() && aired()}>
                      <Show
                        when={watched()}
                        fallback={
                          <button type="button" class="btn btn-sm" classList={{ "btn-primary": next() }} onClick={() => props.saver.save({ progress: ep })}>
                            {next() ? "Mark watched" : "Watched up to here"}
                          </button>
                        }
                      >
                        <button type="button" class="btn btn-sm btn-quiet" onClick={() => props.saver.save({ progress: ep - 1 })}>
                          Unwatch
                        </button>
                      </Show>
                    </Show>
                  </span>
                </li>
              );
            }}
          </For>
        </ol>
        <Show when={showUnaired()}>
          <p class="ep-unaired num">
            {unairedFrom() === count()
              ? `Episode ${count()} hasn't aired yet.`
              : `Episodes ${unairedFrom()}–${count()} haven't aired yet.`}
          </p>
        </Show>
      </section>
    </Show>
  );
}
