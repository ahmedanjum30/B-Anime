import { For, Show, createMemo, createResource, createSignal } from "solid-js";
import { A, useNavigate } from "@solidjs/router";

import { getLibrary } from "../lib/api";
import { recallDub } from "../lib/dub";
import { Copy, Search } from "../lib/icons";
import { homeStore } from "../lib/store";
import type { LibraryEntry, ListStatus } from "../lib/types";
import { watchStore } from "../lib/watch";

const TABS: [ListStatus, string][] = [
  ["CURRENT", "Watching"],
  ["PLANNING", "Planning"],
  ["PAUSED", "Paused"],
  ["COMPLETED", "Completed"],
  ["REPEATING", "Rewatching"],
  ["DROPPED", "Dropped"],
];

const clock = (s: number) => {
  const t = Math.max(0, Math.floor(s));
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const sec = String(t % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
};

/**
 * Watch together: start or join a room. In the lobby everyone gathers and says they're ready,
 * the host picks the episode and starts it; from then on play, pause, seeks and episode changes
 * are shared. Each device plays from its own source.
 */
export default function Together() {
  const w = watchStore;
  const navigate = useNavigate();
  const [name, setName] = createSignal(w.defaultName());
  const [code, setCode] = createSignal("");

  const isHost = () => !!w.room()?.host;
  const others = () => w.members().filter((m) => m.id !== w.room()?.me);
  const allReady = () => w.members().length > 1 && w.members().every((m) => m.ready);
  const watchPath = () => {
    const p = w.nowPlaying();
    return p ? `/watch/${p.mediaId}/${p.episode}${p.dub ? "?dub=1" : ""}` : null;
  };
  // The host's picker: their AniList anime by status, searchable.
  const [library] = createResource(() => (w.room()?.host ? ("ANIME" as const) : null), (k) => getLibrary(k));
  const [tab, setTab] = createSignal<ListStatus>("CURRENT");
  const [find, setFind] = createSignal("");
  const counts = createMemo(() => {
    const c: Partial<Record<ListStatus, number>> = {};
    for (const e of library() ?? []) c[e.status] = (c[e.status] ?? 0) + 1;
    return c;
  });
  const picks = createMemo(() => {
    const q = find().trim().toLowerCase();
    return (library() ?? [])
      .filter((e) => (q ? true : e.status === tab()))
      .filter((e) => !q || e.title.toLowerCase().includes(q) || (e.titleEnglish ?? "").toLowerCase().includes(q))
      .slice(0, 24);
  });
  /** The episode to watch next: after their progress, or the first for Planning and Completed. */
  const nextEpisode = (e: LibraryEntry) => {
    if (e.status === "PLANNING" || e.status === "COMPLETED") return 1;
    const last = e.released ?? e.total ?? Infinity;
    return Math.max(1, Math.min(e.progress + 1, last));
  };

  const copy = async () => {
    const c = w.room()?.code;
    if (!c) return;
    try {
      await navigator.clipboard.writeText(c);
      homeStore.notify("Room code copied. Send it to whoever's joining.");
    } catch {
      homeStore.notify("Couldn't copy the code. Select it and copy it instead.");
    }
  };
  /** The host moves the room to another episode of the same show (everyone opens it, paused). */
  const setEpisode = (n: number) => {
    const p = w.nowPlaying();
    if (!p || !Number.isFinite(n) || n < 1 || n === p.episode) return;
    w.send({ ...p, episode: n, position: 0, playing: false });
  };
  const [epDraft, setEpDraft] = createSignal("");
  const start = () => {
    w.start();
    const path = watchPath();
    if (path) navigate(path);
  };

  return (
    <div class="page together-page">
      <h1 class="page-title">Watch together</h1>
      <p class="page-lede">
        Watch an episode in step with friends. Everyone plays it from their own source; only play, pause and the
        position are shared, device to device.
      </p>

      <Show
        when={w.room()}
        fallback={
          <section class="section" aria-labelledby="together-start-h">
            <h2 id="together-start-h" class="section-head">Start or join a room</h2>
            <div class="together-form">
              <label class="watch-field">
                <span class="watch-label">Your name</span>
                <input
                  class="watch-input"
                  value={name()}
                  maxLength={32}
                  placeholder="How others see you"
                  onInput={(e) => setName(e.currentTarget.value)}
                />
              </label>
              <button type="button" class="btn btn-primary" disabled={!!w.busy()} onClick={() => void w.host(name().trim())}>
                {w.busy() === "host" ? "Opening a room…" : "Start a room"}
              </button>
              <form
                class="watch-join"
                onSubmit={(e) => {
                  e.preventDefault();
                  if (code().trim()) void w.join(code(), name().trim());
                }}
              >
                <label class="watch-field">
                  <span class="watch-label">Or join with a code</span>
                  <input
                    class="watch-input watch-code-input"
                    value={code()}
                    placeholder="Paste the room code"
                    spellcheck={false}
                    autocomplete="off"
                    onInput={(e) => setCode(e.currentTarget.value)}
                  />
                </label>
                <button type="submit" class="btn" disabled={!!w.busy() || !code().trim()}>
                  {w.busy() === "join" ? "Joining…" : "Join"}
                </button>
              </form>
            </div>
          </section>
        }
      >
        {(room) => (
          <>
            <section class="section" aria-labelledby="together-room-h">
              <h2 id="together-room-h" class="section-head">
                {w.started() ? "Watching" : "Lobby"}
              </h2>

              <div class="watch-code">
                <span class="watch-label">{room().host ? "Room code: send it to whoever's joining" : "Room code"}</span>
                <div class="watch-code-row">
                  <code class="watch-code-text" title={room().code}>{room().code}</code>
                  <button type="button" class="btn btn-sm" onClick={() => void copy()}>
                    <Copy size={14} stroke-width={1.75} aria-hidden="true" /> Copy
                  </button>
                </div>
              </div>

              <div class="together-now">
                <Show
                  when={w.nowPlaying()}
                  fallback={
                    <p class="together-now-empty">
                      {room().host
                        ? "Pick what to watch: open any episode, or one from your Up next below."
                        : "Waiting for the host to pick an episode."}
                    </p>
                  }
                >
                  {(p) => (
                    <>
                      <div class="together-now-text">
                        <span class="watch-label">{w.started() ? "Now watching" : "Up first"}</span>
                        <p class="together-now-title">{p().title || "An episode"}</p>
                        <p class="together-now-ep num">Episode {p().episode}</p>
                      </div>
                      <A class="btn btn-sm" href={watchPath()!}>Go to the episode</A>
                    </>
                  )}
                </Show>
              </div>

              {/* The host changes the pick: another episode of this show, or another show. */}
              <Show when={room().host}>
                <div class="together-edit">
                  <Show when={w.nowPlaying()}>
                    {(p) => (
                      <form
                        class="together-ep"
                        onSubmit={(e) => {
                          e.preventDefault();
                          setEpisode(Number(epDraft()));
                          setEpDraft("");
                        }}
                      >
                        <span class="watch-label">Episode</span>
                        <button
                          type="button"
                          class="btn btn-sm"
                          aria-label="Previous episode"
                          disabled={p().episode <= 1}
                          onClick={() => setEpisode(p().episode - 1)}
                        >
                          −
                        </button>
                        <input
                          class="watch-input together-ep-input num"
                          inputmode="numeric"
                          aria-label="Episode number"
                          placeholder={String(p().episode)}
                          value={epDraft()}
                          onInput={(e) => setEpDraft(e.currentTarget.value.replace(/[^\d.]/g, ""))}
                        />
                        <button type="button" class="btn btn-sm" aria-label="Next episode" onClick={() => setEpisode(p().episode + 1)}>
                          +
                        </button>
                        <Show when={epDraft()}>
                          <button type="submit" class="btn btn-sm btn-primary">Set</button>
                        </Show>
                      </form>
                    )}
                  </Show>
                  <button
                    type="button"
                    class="btn btn-sm btn-quiet"
                    onClick={() => {
                      homeStore.notify("Open any episode: it becomes the room's pick.");
                      navigate("/library");
                    }}
                  >
                    {w.nowPlaying() ? "Pick another show" : "Browse your library"}
                  </button>
                </div>
              </Show>

              <Show when={room().host}>
                <div class="together-picks">
                  <div class="together-picks-head">
                    <h3 class="together-subhead">Pick from your list</h3>
                    <label class="search search-sm together-find">
                      <Search size={14} stroke-width={1.75} aria-hidden="true" />
                      <span class="sr-only">Find a show on your list</span>
                      <input type="search" placeholder="Find a show" value={find()} onInput={(e) => setFind(e.currentTarget.value)} />
                    </label>
                  </div>
                  <Show when={!find().trim()}>
                    <div class="segmented segmented-sm together-tabs" role="group" aria-label="List">
                      <For each={TABS}>
                        {([status, label]) => (
                          <button
                            type="button"
                            aria-pressed={tab() === status}
                            data-empty={!counts()[status] ? "" : undefined}
                            onClick={() => setTab(status)}
                          >
                            {label} <span class="together-tab-count num">{counts()[status] ?? 0}</span>
                          </button>
                        )}
                      </For>
                    </div>
                  </Show>
                  <Show
                    when={picks().length}
                    fallback={
                      <p class="together-hint">
                        {library.loading ? "Loading your list…" : find().trim() ? "Nothing on your list matches." : "Nothing here."}
                      </p>
                    }
                  >
                    <ul class="together-pick-list">
                      <For each={picks()}>
                        {(e) => {
                          const ep = () => nextEpisode(e);
                          return (
                            <li>
                              <button
                                type="button"
                                class="together-pick"
                                aria-current={w.nowPlaying()?.mediaId === e.mediaId ? "true" : undefined}
                                // Sets the room's episode; nothing opens until Start (or, once
                                // started, everyone moves to it, paused).
                                onClick={() =>
                                  w.send({
                                    mediaId: e.mediaId,
                                    episode: ep(),
                                    title: e.title,
                                    dub: !!recallDub(e.mediaId),
                                    position: 0,
                                    playing: false,
                                  })
                                }
                              >
                                <Show
                                  when={e.coverUrl}
                                  fallback={<span class="together-pick-cover together-pick-cover-empty" aria-hidden="true" />}
                                >
                                  {(src) => <img class="together-pick-cover" src={src()} alt="" width="40" height="56" loading="lazy" />}
                                </Show>
                                <span class="together-pick-text">
                                  <span class="together-pick-title">{e.title}</span>
                                  <span class="together-pick-ep num">
                                    {w.nowPlaying()?.mediaId === e.mediaId ? (
                                      <span class="together-picked">Picked · Ep {w.nowPlaying()!.episode}</span>
                                    ) : (
                                      <>Next: Ep {ep()}{e.total ? ` of ${e.total}` : ""}</>
                                    )}
                                  </span>
                                </span>
                              </button>
                            </li>
                          );
                        }}
                      </For>
                    </ul>
                  </Show>
                </div>
              </Show>
            </section>

            <section class="section" aria-labelledby="together-people-h">
              <h2 id="together-people-h" class="section-head">
                People <small class="num">{w.members().length}</small>
              </h2>
              <Show when={!others().length}>
                <p class="together-hint">Nobody else yet. Send them the room code.</p>
              </Show>
              <ul class="watch-list">
                <For each={w.members()}>
                  {(m) => (
                    <li class="watch-member">
                      <span class="watch-member-name">
                        {m.name}
                        <Show when={m.id === room().me}> <span class="watch-you">(you)</span></Show>
                      </span>
                      <Show when={m.host}><span class="ext-tag watch-host-tag">Host</span></Show>
                      <span class="watch-member-state num" data-ready={!w.started() && m.ready ? "" : undefined}>
                        {w.started() ? (m.buffering ? "Loading…" : clock(m.position)) : m.ready ? "Ready" : "Not ready"}
                      </span>
                    </li>
                  )}
                </For>
              </ul>

              <div class="together-actions">
                <Show when={!w.started()}>
                  <button
                    type="button"
                    class="btn"
                    classList={{ "btn-primary": !w.me()?.ready }}
                    aria-pressed={!!w.me()?.ready}
                    onClick={() => w.setReady(!w.me()?.ready)}
                  >
                    {w.me()?.ready ? "Not ready" : "I'm ready"}
                  </button>
                  <Show when={isHost()}>
                    <button
                      type="button"
                      class="btn"
                      classList={{ "btn-primary": allReady() }}
                      disabled={!w.nowPlaying()}
                      title={!w.nowPlaying() ? "Pick an episode first" : undefined}
                      onClick={start}
                    >
                      {allReady() || !others().length ? "Start" : "Start anyway"}
                    </button>
                  </Show>
                </Show>
                <button type="button" class="btn btn-quiet" onClick={() => void w.leave()}>
                  {room().host ? "End the room" : "Leave the room"}
                </button>
              </div>
              <Show when={!w.started()}>
                <p class="together-hint">
                  {isHost()
                    ? allReady()
                      ? "Everyone's ready."
                      : "Start lights up when everyone's ready. You can start anyway."
                    : "The host starts the episode once everyone's ready."}
                </p>
              </Show>
            </section>
          </>
        )}
      </Show>
    </div>
  );
}
