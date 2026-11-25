import { For, Show, createEffect, createMemo, createResource, createSignal, on } from "solid-js";
import { A, useNavigate } from "@solidjs/router";

import { errorText, getLibrary, getMedia, getPlayerPrefs, getStream, installExtension, localEpisodes, localLength, streamLength, watchMyCode, watchNewLink } from "../lib/api";
import { SUB_LANGS, subLangName } from "../lib/langs";
import { recallDub } from "../lib/dub";
import { Copy, Search, X } from "../lib/icons";
import { fmtAgo } from "../lib/format";
import { createSource } from "../lib/source";
import { homeStore } from "../lib/store";
import type { LibraryEntry, ListStatus } from "../lib/types";
import { inviteLink, watchStore } from "../lib/watch";

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
export default function Together(props: {
  /** Inside the player, over the episode (which keeps playing): no page title, a Back button. */
  embedded?: boolean;
  onClose?: () => void;
}) {
  const w = watchStore;
  const navigate = useNavigate();
  const [name, setName] = createSignal(w.defaultName());
  const [code, setCode] = createSignal("");

  const isHost = () => !!w.room()?.host;
  const others = () => w.members().filter((m) => m.id !== w.room()?.me);
  /** Ready only once this device's episode has loaded (or there's nothing picked yet). */
  const canReady = () => !w.nowPlaying() || (w.me()?.playable === true && sameCopy());
  /** This device's copy of the episode is the host's (the same length), or it isn't known yet. */
  const sameCopy = () => {
    const me = w.me();
    return !me || w.lengthGap(me) === null;
  };
  // A change (another source, another episode) that doesn't play takes Ready back.
  createEffect(() => {
    if (!w.started() && w.me()?.ready && w.nowPlaying() && (w.me()?.playable === false || !sameCopy())) w.setReady(false);
  });
  const allReady = () => w.members().length > 1 && w.members().every((m) => m.ready && m.playable !== false);
  const cantPlay = () => w.members().filter((m) => m.playable === false);
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

  const copy = async (what: "link" | "code") => {
    const c = w.room()?.code;
    if (!c) return;
    try {
      await navigator.clipboard.writeText(what === "link" ? inviteLink(c) : c);
      homeStore.notify(
        what === "link"
          ? "Invite link copied. Opening it brings people straight into the room."
          : "Room code copied. Send it to whoever's joining.",
      );
    } catch {
      homeStore.notify("Couldn't copy. Select it and copy it instead.");
    }
  };
  // Rejoin, from the launch notice.
  createEffect(
    on(w.pendingRejoin, (r) => {
      if (!r || w.busy() || w.room()) return;
      w.setPendingRejoin(null);
      void w.rejoin(r, name().trim());
    }),
  );
  // An invite link was opened: join it (once a name is known, which it usually is).
  createEffect(
    on(w.invite, (inv) => {
      if (!inv || w.busy()) return;
      w.setInvite(null);
      if (w.room()?.code === inv) return;
      setCode(inv);
      void w.join(inv, name().trim());
    }),
  );
  const [leaving, setLeaving] = createSignal(false);
  // This device's permanent room code (shown before a room is open), and replacing it.
  const [myCode, { mutate: setMyCode }] = createResource(() => watchMyCode().catch(() => ""));
  const copyText = async (text: string, done: string) => {
    try {
      await navigator.clipboard.writeText(text);
      homeStore.notify(done);
    } catch {
      homeStore.notify("Couldn't copy. Select it and copy it instead.");
    }
  };
  const newLink = async () => {
    if (!window.confirm("Make a new invite link? Your current link will stop working.")) return;
    try {
      setMyCode(await watchNewLink());
      homeStore.notify("New invite link made. The old one no longer works.");
    } catch (e) {
      homeStore.notify(errorText(e));
    }
  };
  /** Renaming the room (host) or yourself, inline. */
  const [editing, setEditing] = createSignal<"room" | "me" | null>(null);
  /** The host moves the room to another episode of the same show (everyone opens it, paused). */
  const setEpisode = (n: number) => {
    const p = w.nowPlaying();
    if (!p || !Number.isFinite(n) || n < 1 || n === p.episode) return;
    if (w.pick({ ...p, episode: n })) navigate(w.roomPath()!);
  };
  const [epDraft, setEpDraft] = createSignal("");
  // The room's audio and subtitles (the host sets them; everyone's player follows).
  const [prefs] = createResource(getPlayerPrefs);
  const roomSubs = () => w.nowPlaying()?.subs ?? prefs()?.subtitles ?? "en";
  const setRoom = (change: { dub?: boolean; subs?: string }) => {
    const p = w.nowPlaying();
    if (!p) return;
    w.send({ ...p, ...change, subs: change.subs ?? roomSubs(), position: change.dub === undefined ? p.position : 0, playing: false });
  };
  const start = () => {
    w.start();
    const path = watchPath();
    if (path) navigate(path);
    props.onClose?.();
  };

  return (
    <div class="together-page" classList={{ page: !props.embedded, "together-embedded": !!props.embedded }}>
      <Show
        when={props.embedded}
        fallback={
          <>
            <h1 class="page-title">Watch together</h1>
            <p class="page-lede">
              Watch an episode in step with friends. Everyone plays it from their own source; only play, pause and
              the position are shared, device to device.
            </p>
          </>
        }
      >
        <div class="drawer-head">
          <div class="drawer-title">
            <p class="drawer-show">Watch together</p>
            <p class="drawer-meta">The episode keeps playing behind this.</p>
          </div>
          <button type="button" class="btn btn-sm" onClick={() => props.onClose?.()}>
            Back to the episode
          </button>
        </div>
      </Show>

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
              {/* This device always hosts under the same code, so its invite link can be kept. */}
              <Show when={myCode()}>
                {(c) => (
                  <div class="together-mylink">
                    <span class="watch-label">Your room's invite link, the same every time. It works while your room is open.</span>
                    <div class="watch-code-row">
                      <code class="watch-code-text" title={inviteLink(c())}>{inviteLink(c())}</code>
                      <button type="button" class="btn btn-sm" onClick={() => void copyText(inviteLink(c()), "Invite link copied.")}>
                        <Copy size={14} stroke-width={1.75} aria-hidden="true" /> Copy
                      </button>
                      <button type="button" class="btn btn-sm btn-quiet" onClick={() => void newLink()}>
                        New link
                      </button>
                    </div>
                  </div>
                )}
              </Show>
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
            <RecentRooms name={name().trim()} />
          </section>
        }
      >
        {(room) => (
          <>
            <section class="section" aria-labelledby="together-room-h">
              {/* The room's name (the host renames it), and where it's at. */}
              <div class="together-title-row">
                <Show
                  when={editing() === "room"}
                  fallback={
                    <h2 id="together-room-h" class="together-room-name">
                      {w.roomName() || "Watch together"}
                    </h2>
                  }
                >
                  <NameField
                    value={w.roomName()}
                    label="Room name"
                    onDone={(v) => {
                      if (v !== null) w.nameRoom(v);
                      setEditing(null);
                    }}
                  />
                </Show>
                <span class="together-phase">{w.started() ? "Watching" : "Lobby"}</span>
                <Show when={room().host && editing() !== "room"}>
                  <button type="button" class="link-btn together-rename" onClick={() => setEditing("room")}>
                    Rename
                  </button>
                </Show>
              </div>

              <Show when={w.reconnecting()}>
                <p class="together-banner" role="status">
                  <i class="spin-dot" aria-hidden="true" /> Connection to the room dropped. Getting back in…
                </p>
              </Show>

              <div class="watch-code">
                <span class="watch-label">Invite people: the link opens B-Anime straight into this room</span>
                <div class="watch-code-row">
                  <code class="watch-code-text" title={inviteLink(room().code)}>{inviteLink(room().code)}</code>
                  <button type="button" class="btn btn-sm btn-primary" onClick={() => void copy("link")}>
                    <Copy size={14} stroke-width={1.75} aria-hidden="true" /> Copy link
                  </button>
                  <button type="button" class="btn btn-sm btn-quiet" onClick={() => void copy("code")}>
                    Copy code
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
                        <p class="together-now-ep num">
                          Episode {p().episode} · {p().dub ? "Dub" : "Sub"} ·{" "}
                          {roomSubs() === "off" ? "No subtitles" : `${subLangName(roomSubs())} subtitles`}
                        </p>
                      </div>
                      <A class="btn btn-sm" href={watchPath()!}>Go to the episode</A>
                    </>
                  )}
                </Show>
              </div>

              <Show when={w.nowPlaying()?.mediaId} keyed>
                {(mediaId) => <RoomCheck mediaId={mediaId} />}
              </Show>

              {/* The host changes the pick: another episode of this show, or another show. */}
              <Show when={room().host}>
                <Show when={w.nowPlaying() && !w.started()}>
                  <div class="together-av">
                    <div class="segmented segmented-sm" role="group" aria-label="Audio">
                      <button type="button" aria-pressed={!w.nowPlaying()!.dub} onClick={() => setRoom({ dub: false })}>
                        Sub
                      </button>
                      <button type="button" aria-pressed={!!w.nowPlaying()!.dub} onClick={() => setRoom({ dub: true })}>
                        Dub
                      </button>
                    </div>
                    <label class="select select-sm">
                      <span class="select-label">Subtitles</span>
                      <select value={roomSubs()} onChange={(e) => setRoom({ subs: e.currentTarget.value })}>
                        <For each={SUB_LANGS}>{([code, label]) => <option value={code}>{label}</option>}</For>
                      </select>
                    </label>
                  </div>
                </Show>
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
                                onClick={() => {
                                  const moved = w.pick({
                                    mediaId: e.mediaId,
                                    episode: ep(),
                                    title: e.title,
                                    dub: !!recallDub(e.mediaId),
                                  });
                                  if (moved) navigate(w.roomPath()!);
                                }}
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
                <p class="together-hint">Nobody else yet. Send them the invite link.</p>
              </Show>
              <ul class="watch-list">
                <For each={w.members()}>
                  {(m) => (
                    <li class="watch-member">
                      <Show
                        when={m.id === room().me && editing() === "me"}
                      >
                        <NameField
                          value={m.name}
                          label="Your name"
                          onDone={(v) => {
                            if (v !== null) {
                              w.rename(v);
                              setName(v);
                            }
                            setEditing(null);
                          }}
                        />
                      </Show>
                      <span class="watch-member-name" classList={{ "is-hidden": m.id === room().me && editing() === "me" }}>
                        {m.name}
                        <Show when={m.id === room().me}>
                          {" "}
                          <span class="watch-you">(you)</span>
                        </Show>
                      </span>
                      <Show when={m.id === room().me && editing() !== "me"}>
                        <button type="button" class="link-btn together-rename" onClick={() => setEditing("me")}>
                          Rename
                        </button>
                      </Show>
                      <Show when={m.host}><span class="ext-tag watch-host-tag">Host</span></Show>
                      <span class="watch-member-meta">
                        <Show when={m.playable !== null || w.nowPlaying()}>
                          <span class="watch-member-check" data-bad={m.playable === false ? "" : undefined}>
                            {m.playable === null ? "Checking…" : m.playable ? m.source ?? "Can play" : "Can't play this"}
                          </span>
                        </Show>
                        {/* Their ping to the host (measured, the median of the last few). */}
                        {/* A different cut of the episode (a Blu-ray file against a TV stream). */}
                        <Show when={w.lengthGap(m)}>
                          {(gap) => (
                            <span
                              class="watch-member-length num"
                              title="A different cut of this episode: it drifts out of step with the host's"
                            >
                              {w.gapText(gap())}
                            </span>
                          )}
                        </Show>
                        <Show when={m.rttMs !== null}>
                          <span
                            class="watch-member-link num"
                            data-slow={(m.rttMs ?? 0) > 250 ? "" : undefined}
                            title="Ping to the host"
                          >
                            {m.rttMs} ms
                          </span>
                        </Show>
                          {/* Host: the room pauses while someone loads; not for someone on a slow
                              connection it shouldn't keep stopping for. */}
                          <Show when={room().host && w.started() && m.id !== room().me}>
                            <button
                              type="button"
                              class="link-btn watch-wait"
                              aria-pressed={!w.notWaiting().has(m.id)}
                              title={
                                w.notWaiting().has(m.id)
                                  ? `The room plays on while ${m.name} loads`
                                  : `The room pauses while ${m.name} loads`
                              }
                              onClick={() => w.waitFor(m.id, w.notWaiting().has(m.id))}
                            >
                              {w.notWaiting().has(m.id) ? "Wait for them" : "Don't wait"}
                            </button>
                          </Show>
                      </span>
                      <span class="watch-member-state num" data-ready={!w.started() && m.ready ? "" : undefined}>
                        {m.connected === false
                          ? "Reconnecting…"
                          : w.started()
                            ? m.buffering
                              ? "Loading…"
                              : clock(m.position)
                            : m.ready
                              ? "Ready"
                              : "Not ready"}
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
                    disabled={!w.me()?.ready && !canReady()}
                    title={!w.me()?.ready && !canReady() ? "Once your episode loads" : undefined}
                    onClick={() => w.setReady(!w.me()?.ready)}
                  >
                    {w.me()?.ready ? "Not ready" : canReady() ? "I'm ready" : w.me()?.playable === false ? "Can't play it yet" : !sameCopy() ? "Not the host's copy" : "Loading your episode…"}
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
                      {allReady() || (!others().length && w.me()?.playable !== false) ? "Start" : "Start anyway"}
                    </button>
                  </Show>
                </Show>
                <Show
                  when={room().host && others().length}
                  fallback={
                    <button
                      type="button"
                      class="btn"
                      classList={{ "btn-danger": !!room().host }}
                      onClick={() => void w.leave(true)}
                    >
                      {room().host ? "End the room" : "Leave the room"}
                    </button>
                  }
                >
                  {/* With others in it, a host can leave the room to them, or end it for everyone. */}
                  <button
                    type="button"
                    class="btn"
                    disabled={leaving()}
                    title={`${others()[0]?.name ?? "The next person"} hosts from here`}
                    onClick={() => {
                      setLeaving(true);
                      void w.leave(false).finally(() => setLeaving(false));
                    }}
                  >
                    Leave (hand over)
                  </button>
                  <button type="button" class="btn btn-danger" disabled={leaving()} onClick={() => void w.leave(true)}>
                    End for everyone
                  </button>
                </Show>
              </div>
              <Show when={!w.started()}>
                <p class="together-hint">
                  {isHost()
                    ? allReady()
                      ? "Everyone's ready."
                      : cantPlay().length
                        ? `${cantPlay().map((m) => m.name).join(", ")} can't play this episode yet.`
                        : "Start lights up when everyone's ready. You can start anyway."
                    : "The host starts the episode once everyone's ready."}
                </p>
              </Show>
            </section>
            <RecentRooms name={name().trim()} />
          </>
        )}
      </Show>
    </div>
  );
}

/**
 * Can this device play the room's episode? From the library, or through a source match that
 * lists the episode. Tells the room (the others see it), and when it can't: change source,
 * fix the match, or install the host's source.
 */
/** Whether an episode loaded from a source (by source and episode id), remembered while the app runs. */
type Probe = { ok: boolean; why: string; url?: string; kind?: string };
const probed = new Map<string, Probe>();

function RoomCheck(props: { mediaId: number }) {
  const w = watchStore;
  const [page] = createResource(() => props.mediaId, getMedia);
  const [localEps] = createResource(() => props.mediaId, (id) => localEpisodes(id).catch(() => [] as number[]));
  const episode = () => w.nowPlaying()?.episode ?? 1;
  const dub = () => !!w.nowPlaying()?.dub;
  const source = createSource(() => page()?.media, undefined, dub);
  const extName = () => source.usable().find((e) => e.id === source.ext())?.name ?? null;
  const hostSource = () => w.nowPlaying()?.source ?? null;
  const hasHostSource = () => !!hostSource() && source.usable().some((e) => e.id === hostSource()!.id);
  const [installing, setInstalling] = createSignal(false);

  /** The library has the room's episode. */
  const hasLocal = () => !!localEps()?.includes(episode());
  /** Not picked: the host's source, when the host plays from one and this device has it (the same
   *  copy as the host's, so it lines up); else the library when it has the episode. */
  const hostDefault = () => {
    const c = w.playFrom();
    return !(c && c.mediaId === props.mediaId) && !!w.hostSourceFor(props.mediaId) && hasHostSource();
  };
  // Shown as picked: the host's source (by default), else whichever this source list has.
  const shownExt = () => (hostDefault() ? hostSource()!.id : source.ext());
  /** Playing from the library: picked, or (not picked) by default (above). */
  const useLibrary = () => {
    const c = w.playFrom();
    if (c && c.mediaId === props.mediaId) return hasLocal() && c.library;
    return hasLocal() && !hostDefault();
  };

  const pickFrom = (value: string) => {
    if (value === "library") return w.setPlayFrom({ mediaId: props.mediaId, library: true });
    // Which source too: the player follows this pick exactly (not a remembered one).
    w.setPlayFrom({ mediaId: props.mediaId, library: false, ext: value });
    source.setExt(value);
  };
  // The episode really loading from the source (not just being listed), before Ready: asked
  // once per source and episode (remembered), and only in the lobby. Once the room plays, the
  // player itself shows whether it does. The library's files are on this device already.
  const probeKey = createMemo(() => {
    if (w.started() || useLibrary() || localEps.loading) return null;
    const item = source.byNumber().get(episode());
    const ext = source.ext();
    return item && ext ? `${ext}\u0000${item.id}` : null;
  });
  const [probe] = createResource(probeKey, (key): Probe | Promise<Probe> => {
    const known = probed.get(key);
    if (known) return known;
    const [ext, id] = key.split("\u0000");
    return getStream(ext, id).then(
      (stream) => {
        const first = stream.sources[0];
        const r: Probe = { ok: true, why: "", url: first?.url, kind: first?.kind };
        probed.set(key, r);
        return r;
      },
      (e) => ({ ok: false, why: errorText(e) }),
    );
  });
  // How long this device's copy is, in the waiting room (before anyone plays it): the library
  // file's own length, or the stream's from its playlist. Compared with the host's, it shows a
  // different cut (a Blu-ray file against a TV stream) before Start.
  const lengthKey = createMemo(() => {
    if (w.started()) return null;
    if (useLibrary()) return `L\u0000${props.mediaId}\u0000${episode()}`;
    const p = probe();
    return p?.ok && p.url && p.kind ? `S\u0000${p.kind}\u0000${p.url}` : null;
  });
  const [copyLength] = createResource(lengthKey, (key) => {
    const [kind, a, b] = key.split("\u0000");
    const got = kind === "L" ? localLength(Number(a), Number(b)) : streamLength(b, a);
    return got.catch(() => null);
  });

  /** true / false, or null while checking. */
  const result = createMemo<{ playable: boolean | null; via: string | null; why?: string }>(() => {
    if (useLibrary()) return { playable: true, via: "Library" };
    if (page.loading || localEps.loading || source.exts.loading) return { playable: null, via: null };
    if (!source.usable().length) return { playable: false, via: null, why: "No anime source installed." };
    const m = source.match();
    if (source.match.loading || (!m && !source.match.error)) return { playable: null, via: extName() };
    if (source.match.error) return { playable: false, via: extName(), why: `${extName()} isn't answering.` };
    if (m?.status !== "matched") return { playable: false, via: extName(), why: `${extName()} didn't find this show.` };
    if (source.items.loading) return { playable: null, via: extName() };
    if (!source.byNumber().has(episode()))
      return { playable: false, via: extName(), why: `${extName()} doesn't have episode ${episode()}.` };
    if (w.started()) return { playable: true, via: extName() };
    const p = probe();
    if (probe.loading || !p) return { playable: null, via: extName() };
    if (!p.ok) return { playable: false, via: extName(), why: `${extName()} couldn't load episode ${episode()}. ${p.why}` };
    return { playable: true, via: extName() };
  });
  /** Where this device can play from: the library (when it has the episode) and each source. */
  const options = () => [
    ...(hasLocal() ? [{ id: "library", name: "Your library" }] : []),
    ...source.usable().map((e) => ({ id: e.id, name: e.name })),
  ];
  createEffect(() => {
    const r = result();
    w.check(r.playable, r.via, copyLength.loading ? null : (copyLength() ?? null));
  });
  /** This device's copy against the host's (+ longer, − shorter), when it's a different cut. */
  const myGap = () => {
    const me = w.me();
    return me ? w.lengthGap(me) : null;
  };
  // The host's working source goes with the room's pick (in the lobby), so guests who can't
  // play it can install the same one.
  createEffect(() => {
    const r = result();
    const p = w.nowPlaying();
    const e = source.usable().find((x) => x.id === source.ext());
    if (!w.room()?.host || w.started() || !p || r.playable !== true || r.via === "Library" || !e) return;
    if (!/^https:\/\//.test(e.sourceUrl) || p.source?.id === e.id) return;
    w.send({ ...p, source: { id: e.id, name: e.name, url: e.sourceUrl } });
  });

  const installHost = async () => {
    const h = hostSource();
    if (!h) return;
    setInstalling(true);
    try {
      await installExtension(h.url);
      await source.reloadExts();
      source.setExt(h.id);
      homeStore.notify(`Installed ${h.name}.`);
    } catch (e) {
      homeStore.notify(errorText(e));
    } finally {
      setInstalling(false);
    }
  };

  return (
    <div class="together-check" data-state={result().playable === null ? "checking" : result().playable ? "ok" : "bad"}>
      <Show when={options().length > 1}>
        <label class="select select-sm together-from">
          <span class="select-label">Playing from</span>
          <select value={useLibrary() ? "library" : (shownExt() ?? "")} onChange={(e) => pickFrom(e.currentTarget.value)}>
            <For each={options()}>{(o) => <option value={o.id}>{o.name}</option>}</For>
          </select>
        </label>
      </Show>
      <Show when={myGap()}>
        {(gap) => (
          <div class="together-check-fix together-copy">
            <p class="together-check-line">
              Your copy is {w.gapText(gap())} than the host's, so it wouldn't stay in step.
              {hasHostSource() ? "" : " Play it from the same source as the host."}
            </p>
            <Show when={hasHostSource() && !w.room()?.host}>
              <button type="button" class="btn btn-sm btn-primary" onClick={() => pickFrom(hostSource()!.id)}>
                Play from {hostSource()!.name}
              </button>
            </Show>
          </div>
        )}
      </Show>
      <p class="together-check-line">
        {result().playable === null
          ? `Loading the episode${result().via ? ` from ${result().via === "Library" ? "your library" : result().via}` : ""} to check it plays…`
          : result().playable
            ? `You can play it${result().via ? ` from ${result().via === "Library" ? "your library" : result().via}` : ""}.`
            : `You can't play it yet. ${result().why ?? ""}`}
      </p>
      <Show when={result().playable === false}>
        <div class="together-check-fix">
          <Show when={hostSource() && !hasHostSource()}>
            <button type="button" class="btn btn-sm btn-primary" disabled={installing()} onClick={() => void installHost()}>
              {installing() ? "Installing…" : `Install ${hostSource()!.name} (the host's source)`}
            </button>
          </Show>
          <Show when={source.usable().length}>
            <A class="btn btn-sm btn-quiet" href={`/show/${props.mediaId}`}>Fix the match</A>
          </Show>
          <Show when={!source.usable().length && !hostSource()}>
            <A class="btn btn-sm btn-quiet" href="/extensions">Get a source</A>
          </Show>
        </div>
      </Show>
    </div>
  );
}

/** Rooms this device was in lately, to rejoin (the one it's in now is left out). */
function RecentRooms(props: { name: string }) {
  const w = watchStore;
  const others = () => w.recent().filter((r) => !w.room() || (r.key !== w.room()!.code && !r.codes.includes(w.room()!.code)));
  return (
    <Show when={others().length}>
      <div class="together-recent">
        <h3 class="together-subhead">Your last room</h3>
        <ul class="watch-list">
          <For each={others()}>
            {(r) => (
              <li class="watch-member together-recent-row">
                <span class="together-recent-text">
                  <span class="watch-member-name">{r.title || "A room"}</span>
                  <span class="together-recent-meta">
                    {r.names.length ? `with ${r.names.join(", ")} · ` : ""}
                    {fmtAgo(new Date(r.at).toISOString(), homeStore.now())}
                  </span>
                </span>
                <button
                  type="button"
                  class="btn btn-sm"
                  disabled={!!w.busy()}
                  onClick={() => void w.rejoin(r, props.name)}
                >
                  {w.busy() === "join" ? "Rejoining…" : "Rejoin"}
                </button>
                <button type="button" class="icon-btn" aria-label={`Forget ${r.title || "this room"}`} onClick={() => w.forget(r.key)}>
                  <X size={16} stroke-width={1.75} aria-hidden="true" />
                </button>
              </li>
            )}
          </For>
        </ul>
      </div>
    </Show>
  );
}

/** An inline name field: Enter or leaving it saves, Esc cancels (onDone gets null). */
function NameField(props: { value: string; label: string; onDone: (value: string | null) => void }) {
  let done = false;
  const finish = (v: string | null) => {
    if (done) return;
    done = true;
    props.onDone(v);
  };
  return (
    <input
      class="watch-input together-name-input"
      aria-label={props.label}
      value={props.value}
      maxLength={32}
      ref={(el) => queueMicrotask(() => (el.focus(), el.select()))}
      onKeyDown={(e) => {
        if (e.key === "Enter") finish(e.currentTarget.value);
        if (e.key === "Escape") finish(null);
      }}
      onBlur={(e) => finish(e.currentTarget.value)}
    />
  );
}
