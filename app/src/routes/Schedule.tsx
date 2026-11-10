import { For, Match, Show, Switch, createMemo, createResource, createSignal, onCleanup } from "solid-js";
import { A } from "@solidjs/router";

import WeekListings from "../components/WeekListings";
import { getSchedule, inTauri, onLibraryChanged } from "../lib/api";
import { fmtCount, startOfWeek } from "../lib/format";
import { homeStore } from "../lib/store";
import type { AiringSlot, ScheduleItem, ScheduleScope } from "../lib/types";

const DAY = 86_400;

/** "in 2d 4h", "in 3h 12m", "in 5m", "airing now". */
function countdown(airsAt: string, now: number): string {
  const mins = Math.round((new Date(airsAt).getTime() - now) / 60_000);
  if (mins <= 0) return "airing now";
  const d = Math.floor(mins / 1440);
  const h = Math.floor((mins % 1440) / 60);
  const m = mins % 60;
  if (d > 0) return `in ${d}d ${h}h`;
  if (h > 0) return `in ${h}h ${m}m`;
  return `in ${m}m`;
}

// Choices survive navigating away and back.
const [week, setWeek] = createSignal<0 | 1>(0);
const [scope, setScope] = createSignal<ScheduleScope>("mine");

export default function Schedule() {
  const { now, account } = homeStore;

  // Monday of the selected week; a number so the 30-second clock tick doesn't refetch.
  const weekStart = createMemo(() => {
    const s = startOfWeek(new Date(now()));
    s.setDate(s.getDate() + 7 * week());
    return s.getTime();
  });
  const key = createMemo(() => ({ scope: scope(), from: Math.floor(weekStart() / 1000) }), undefined, {
    equals: (a, b) => a.scope === b.scope && a.from === b.from,
  });
  const [items, { refetch }] = createResource(key, (k) => getSchedule(k.scope, k.from, k.from + 7 * DAY));

  let unlisten: (() => void) | undefined;
  void onLibraryChanged(() => scope() === "mine" && void refetch()).then((u) => (unlisten = u));
  onCleanup(() => unlisten?.());

  // Upcoming episodes of your own shows, soonest first, for the countdown strip.
  const [mine] = createResource(
    () => Math.floor(startOfWeek(new Date(now())).getTime() / 1000),
    (from) => getSchedule("mine", from, from + 14 * DAY),
  );
  const comingUp = createMemo(() =>
    (mine() ?? [])
      .filter((i) => new Date(i.airsAt).getTime() > now() && i.listStatus !== "PLANNING")
      .slice(0, 5),
  );

  const slots = createMemo<AiringSlot[]>(() =>
    (items.latest ?? []).map((i: ScheduleItem) => {
      const aired = new Date(i.airsAt).getTime() <= now();
      const onList = i.listStatus != null;
      const behind = onList && i.listStatus === "CURRENT" && (i.progress ?? 0) < i.episode;
      return {
        mediaId: i.mediaId,
        title: i.title,
        color: i.color,
        episode: i.episode,
        airsAt: i.airsAt,
        watched: onList && (i.progress ?? 0) >= i.episode,
        onList,
        mark: i.totalEpisodes === i.episode ? "final" : aired && behind ? "new" : null,
      };
    }),
  );

  const notConnected = () => inTauri && account() && !account()!.viewer;

  return (
    <div class="page page-wide">
      <div class="lib-titlebar">
        <h1 class="page-title">Schedule</h1>
        <Show when={items.latest}>
          <p class="lib-total num">
            {fmtCount(items.latest!.length)} {items.latest!.length === 1 ? "episode" : "episodes"}
          </p>
        </Show>
      </div>

      <Show when={comingUp().length > 0}>
        <section class="coming-up" aria-label="Coming up">
          <For each={comingUp()}>
            {(i) => (
              <A href={`/show/${i.mediaId}`} class="coming-card">
                <div class="coming-cover" style={{ "background-color": i.color ?? "var(--z3)" }}>
                  <Show when={i.coverUrl}>{(src) => <img src={src()} alt="" loading="lazy" decoding="async" />}</Show>
                </div>
                <div class="coming-text">
                  <p class="coming-when num">{countdown(i.airsAt, now())}</p>
                  <p class="coming-title">{i.title}</p>
                  <p class="coming-ep num">Episode {i.episode}</p>
                </div>
              </A>
            )}
          </For>
        </section>
      </Show>

      <div class="schedule-controls">
        <div class="tabs" role="tablist" aria-label="Week">
          <button type="button" role="tab" class="tab" aria-selected={week() === 0} onClick={() => setWeek(0)}>
            This week
          </button>
          <button type="button" role="tab" class="tab" aria-selected={week() === 1} onClick={() => setWeek(1)}>
            Next week
          </button>
        </div>
        <div class="segmented" role="group" aria-label="Shows">
          <button type="button" aria-pressed={scope() === "mine"} onClick={() => setScope("mine")}>
            My shows
          </button>
          <button type="button" aria-pressed={scope() === "all"} onClick={() => setScope("all")}>
            Everything airing
          </button>
        </div>
      </div>

      <Switch>
        <Match when={scope() === "mine" && notConnected()}>
          <div class="empty-block">
            <p>Connect AniList to see when your shows air.</p>
            <A href="/settings" class="btn btn-primary">Connect AniList</A>
          </div>
        </Match>
        <Match when={items.error}>
          <div class="empty-block">
            <p class="inline-error">Couldn't load the schedule: {String(items.error)}</p>
            <button type="button" class="btn" onClick={() => void refetch()}>Try again</button>
          </div>
        </Match>
        <Match when={items.latest && slots().length === 0 && !items.loading}>
          <p class="empty">
            {scope() === "mine"
              ? `None of your shows air ${week() === 0 ? "this" : "next"} week.`
              : "Nothing found airing that week."}
          </p>
        </Match>
        <Match when={items.latest}>
          <div class="listings-wrap" data-busy={items.loading ? "" : undefined}>
            <WeekListings
              slots={slots()}
              now={new Date(now())}
              weekStart={new Date(weekStart())}
              caption={scope() === "mine" ? "Episodes airing for shows on your lists" : "Everything airing this week"}
            />
          </div>
        </Match>
        <Match when={items.loading}>
          <div class="sk-listings" aria-busy="true" aria-label="Loading schedule" />
        </Match>
      </Switch>
    </div>
  );
}
