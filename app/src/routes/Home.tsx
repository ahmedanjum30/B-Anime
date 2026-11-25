import { For, Match, Show, Switch, createResource, createSignal, onCleanup } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { Search } from "../lib/icons";

import UpNextCard from "../components/UpNextCard";
import WeekListings from "../components/WeekListings";
import { homeStore } from "../lib/store";
import { fmtDateLine } from "../lib/format";
import type { UpNext } from "../lib/types";
import { playLibraryEpisode } from "../lib/play";
import { recallPlayFrom } from "../lib/source";
import { getPlayerPrefs, inTauri } from "../lib/api";
import { recallDub } from "../lib/dub";

export default function Home() {
  const { home, refetch, notify } = homeStore;
  const navigate = useNavigate();
  const [prefs] = createResource(() => (inTauri ? getPlayerPrefs() : Promise.resolve(null)));

  // Re-render the date line and "today" column across midnight.
  const [now, setNow] = createSignal(new Date());
  const tick = window.setInterval(() => setNow(new Date()), 60_000);
  onCleanup(() => clearInterval(tick));

  const play = (item: UpNext) => {
    if (home()?.isSample) {
      notify(`"${item.title}" is a sample show. Add a library folder to play your own episodes.`);
      return;
    }
    // Not on disk, or this show plays from its source (picked on its page or in a room).
    if (!item.source || recallPlayFrom(item.mediaId) === "source") {
      // Not on disk: straight to the player, which streams it from the source last used for
      // this kind of show (the show page's choice), in Sub or Dub as last watched for this show
      // (else the preferred audio).
      const dub = recallDub(item.mediaId) ?? prefs()?.audio === "en";
      navigate(`/watch/${item.mediaId}/${item.episode}${dub ? "?dub=1" : ""}`);
      return;
    }
    void playLibraryEpisode(item.mediaId, item.episode, navigate, notify);
  };

  return (
    <div class="home">
      <header class="topbar">
        <label class="search">
          <Search size={16} stroke-width={1.75} aria-hidden="true" />
          <span class="sr-only">Search</span>
          <input
            type="search"
            placeholder="Search AniList"
            spellcheck={false}
            onInput={(e) => navigate(`/discover?q=${encodeURIComponent(e.currentTarget.value)}`)}
          />
        </label>
        <p class="dateline">{fmtDateLine(now())}</p>
      </header>

      <Switch>
        <Match when={home.error}>
          <div class="notice is-error" role="alert">
            <p>
              <strong>Couldn't load your shows.</strong> {String(home.error?.message ?? home.error)}
            </p>
            <button type="button" class="btn" onClick={() => refetch()}>Try again</button>
          </div>
        </Match>

        <Match when={home()}>
          {(h) => (
            <>
              <Show when={h().isSample}>
                <div class="notice">
                  <p>
                    <strong>These are sample shows.</strong> Connect AniList and add a library folder to see your own.
                  </p>
                  <button type="button" class="btn btn-primary" onClick={() => navigate("/settings")}>
                    Set up library
                  </button>
                </div>
              </Show>

              <section class="section" aria-labelledby="upnext-h">
                <h2 id="upnext-h" class="section-head">
                  Up next
                  <small>
                    <Show when={h().upNext.length} fallback="Nothing waiting">
                      {h().upNext.length} {h().upNext.length === 1 ? "show" : "shows"} with unwatched episodes
                    </Show>
                  </small>
                </h2>
                <Show
                  when={h().upNext.length}
                  fallback={<p class="empty">You're caught up. New episodes show up here as soon as they're in your library.</p>}
                >
                  <div class="upnext-grid">
                    <For each={h().upNext}>{(item) => <UpNextCard item={item} onPlay={play} />}</For>
                  </div>
                </Show>
              </section>

              <section class="section" aria-labelledby="week-h">
                <h2 id="week-h" class="section-head">
                  This week
                  <small>Airing shows on your Watching list · local times</small>
                </h2>
                <Show
                  when={h().week.length}
                  fallback={<p class="empty">Nothing on your Watching list airs this week.</p>}
                >
                  <div class="listings-wrap">
                    <WeekListings slots={h().week} now={now()} />
                  </div>
                </Show>
              </section>
            </>
          )}
        </Match>

        <Match when={home.loading}>
          <HomeSkeleton />
        </Match>
      </Switch>
    </div>
  );
}

function HomeSkeleton() {
  return (
    <div class="skeleton" aria-busy="true" aria-label="Loading your shows">
      <div class="section-head sk-line" />
      <div class="upnext-grid">
        <For each={[0, 1, 2, 3]}>{() => <div class="sk-card" />}</For>
      </div>
    </div>
  );
}
