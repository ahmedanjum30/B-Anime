import { For, Show, createMemo } from "solid-js";

import Mark from "./Mark";
import type { AiringSlot } from "../lib/types";
import { fmtDay, fmtTime, sameDay, startOfWeek } from "../lib/format";
import { homeStore } from "../lib/store";

interface Placed extends AiringSlot {
  at: Date;
}

/** This week's airing shows, set as a listings grid: days across, times down. */
export default function WeekListings(props: { slots: AiringSlot[]; now: Date }) {
  const { hoverMedia, setHoverMedia } = homeStore;

  const days = createMemo(() => {
    const start = startOfWeek(props.now);
    return Array.from({ length: 7 }, (_, i) => new Date(start.getFullYear(), start.getMonth(), start.getDate() + i));
  });

  const placed = createMemo<Placed[]>(() => props.slots.map((s) => ({ ...s, at: new Date(s.airsAt) })));

  // One row per distinct local time slot, earliest first.
  const rows = createMemo(() => {
    const times = new Map<string, number>();
    for (const s of placed()) {
      const key = fmtTime(s.at);
      times.set(key, s.at.getHours() * 60 + s.at.getMinutes());
    }
    return [...times.entries()].sort((a, b) => a[1] - b[1]).map(([key]) => key);
  });

  const cell = (time: string, day: Date) =>
    placed().filter((s) => fmtTime(s.at) === time && sameDay(s.at, day));

  return (
    <table class="listings">
      <caption class="sr-only">Episodes airing this week for shows on your Watching list</caption>
      <thead>
        <tr>
          <th scope="col" class="listings-corner">
            <span class="sr-only">Time</span>
          </th>
          <For each={days()}>
            {(d) => (
              <th scope="col" class="listings-day" data-today={sameDay(d, props.now) ? "" : undefined}>
                {fmtDay(d)} <span class="num">{d.getDate()}</span>
              </th>
            )}
          </For>
        </tr>
      </thead>
      <tbody>
        <For each={rows()}>
          {(time) => (
            <tr>
              <th scope="row" class="listings-time num">{time}</th>
              <For each={days()}>
                {(d) => (
                  <td data-today={sameDay(d, props.now) ? "" : undefined}>
                    <For each={cell(time, d)}>
                      {(s) => (
                        <div
                          class="slot"
                          data-watched={s.watched ? "" : undefined}
                          data-linked={hoverMedia() === s.mediaId ? "" : undefined}
                          onPointerEnter={() => setHoverMedia(s.mediaId)}
                          onPointerLeave={() => setHoverMedia(null)}
                        >
                          <span class="slot-title">
                            <i class="swatch" style={{ "background-color": s.color ?? "var(--z5)" }} aria-hidden="true" />
                            <span class="slot-name">{s.title}</span>
                          </span>
                          <span class="slot-line">
                            <Show when={s.mark}>{(m) => <Mark kind={m()} />}</Show>
                            <span class="num">Ep {s.episode}</span>
                            <Show when={s.watched}>
                              <span class="slot-stamp">watched</span>
                            </Show>
                          </span>
                        </div>
                      )}
                    </For>
                  </td>
                )}
              </For>
            </tr>
          )}
        </For>
      </tbody>
    </table>
  );
}
