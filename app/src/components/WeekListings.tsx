import { For, Show, createMemo } from "solid-js";
import { A } from "@solidjs/router";

import Mark from "./Mark";
import type { AiringSlot } from "../lib/types";
import { fmtDay, fmtTime, sameDay, startOfWeek } from "../lib/format";
import { homeStore } from "../lib/store";

interface Placed extends AiringSlot {
  at: Date;
}

/** A week of airing episodes, set as a listings grid: days across, times down. */
export default function WeekListings(props: {
  slots: AiringSlot[];
  now: Date;
  /** Monday of the week to show; defaults to the current week. */
  weekStart?: Date;
  caption?: string;
}) {
  const { hoverMedia, setHoverMedia } = homeStore;

  const days = createMemo(() => {
    const start = props.weekStart ?? startOfWeek(props.now);
    return Array.from({ length: 7 }, (_, i) => new Date(start.getFullYear(), start.getMonth(), start.getDate() + i));
  });

  // Only what airs in the week shown: anything else would add empty time rows to the grid and
  // match no day on phones.
  const placed = createMemo<Placed[]>(() => {
    const from = days()[0].getTime();
    const to = new Date(from).setDate(days()[0].getDate() + 7);
    return props.slots.map((s) => ({ ...s, at: new Date(s.airsAt) })).filter((s) => s.at.getTime() >= from && s.at.getTime() < to);
  });

  // One row per distinct local time slot, earliest first.
  const rows = createMemo(() => {
    const times = new Map<string, number>();
    for (const s of placed()) {
      const key = fmtTime(s.at);
      times.set(key, s.at.getHours() * 60 + s.at.getMinutes());
    }
    return [...times.entries()].sort((a, b) => a[1] - b[1]).map(([key]) => key);
  });

  // Bucket once per change instead of filtering per cell.
  const cells = createMemo(() => {
    const map = new Map<string, Placed[]>();
    for (const s of placed()) {
      const key = `${fmtTime(s.at)}|${s.at.toDateString()}`;
      const list = map.get(key);
      if (list) list.push(s);
      else map.set(key, [s]);
    }
    return map;
  });
  const cell = (time: string, day: Date) => cells().get(`${time}|${day.toDateString()}`) ?? [];

  // Phones: the same week as a column of days, each listing its slots by time. Only days with
  // something on, plus today.
  const byDay = createMemo(() =>
    days()
      .map((d) => ({
        day: d,
        today: sameDay(d, props.now),
        slots: placed()
          .filter((s) => sameDay(s.at, d))
          .sort((a, b) => a.at.getTime() - b.at.getTime()),
      }))
      .filter((g) => g.slots.length || g.today),
  );

  return (
    <>
    <ol class="week-days" aria-label={props.caption ?? "Episodes airing this week for shows on your Watching list"}>
      <For each={byDay()}>
        {(g) => (
          <li class="week-day" data-today={g.today ? "" : undefined}>
            <h3 class="week-day-head">
              {fmtDay(g.day)} <span class="num">{g.day.getDate()}</span>
              <Show when={g.today}>
                <span class="week-day-today">Today</span>
              </Show>
            </h3>
            <Show when={g.slots.length} fallback={<p class="week-day-empty">Nothing airs today.</p>}>
              <ul class="week-slots">
                <For each={g.slots}>
                  {(s) => (
                    <li>
                      <A
                        href={`/show/${s.mediaId}`}
                        class="slot week-slot"
                        data-watched={s.watched ? "" : undefined}
                        data-offlist={s.onList === false ? "" : undefined}
                      >
                        <span class="week-slot-time num">{fmtTime(s.at)}</span>
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
                      </A>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </li>
        )}
      </For>
    </ol>
    <table class="listings">
      <caption class="sr-only">{props.caption ?? "Episodes airing this week for shows on your Watching list"}</caption>
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
                        <A
                          href={`/show/${s.mediaId}`}
                          class="slot"
                          data-watched={s.watched ? "" : undefined}
                          data-offlist={s.onList === false ? "" : undefined}
                          data-linked={hoverMedia() === s.mediaId ? "" : undefined}
                          onPointerEnter={() => setHoverMedia(s.mediaId)}
                          onPointerLeave={() => setHoverMedia(null)}
                          title={s.title}
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
                        </A>
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
    </>
  );
}
