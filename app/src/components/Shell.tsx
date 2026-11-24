import type { ParentProps } from "solid-js";
import { For, Show, createEffect, createSignal, on, onCleanup } from "solid-js";
import { errorText, inTauri, installUpdate, onAndroid, openAndroidDownload, openSupport, UPDATE_ACTION, UPDATE_BUSY } from "../lib/api";
import { A, useLocation, useNavigate } from "@solidjs/router";
import { House, Heart, Library, CalendarDays, BookOpen, Compass, Ellipsis, Puzzle, RefreshCw, Settings, Smartphone, Users } from "../lib/icons";

import { homeStore } from "../lib/store";
import { trackPages } from "../lib/nav";
import { watchStore } from "../lib/watch";
import { fmtAgo, fmtCount } from "../lib/format";

// `bar`: kept in the phone's bottom bar; the rest move into the More sheet.
const NAV = [
  { href: "/", label: "Home", icon: House, end: true, bar: true },
  { href: "/library", label: "Library", icon: Library, count: "anime" as const, bar: true },
  { href: "/schedule", label: "Schedule", icon: CalendarDays },
  { href: "/manga", label: "Manga", icon: BookOpen, count: "manga" as const, bar: true },
  { href: "/discover", label: "Discover", icon: Compass, bar: true },
  { href: "/together", label: "Together", icon: Users, count: "room" as const },
  { href: "/extensions", label: "Extensions", icon: Puzzle },
  { href: "/settings", label: "Settings", icon: Settings },
];
const MORE = NAV.filter((n) => !n.bar);

export default function Shell(props: ParentProps) {
  const { home, toast } = homeStore;
  trackPages();
  const location = useLocation();
  const navigate = useNavigate();

  // Watch together: when the room moves to another episode (someone picked it), this device
  // opens it too, from wherever it is in the app.
  // Only a change of episode moves it: between changes, browsing elsewhere in the app is fine.
  let followed = "";
  createEffect(
    on(watchStore.remote, (r) => {
      // In the lobby nobody's app opens the episode yet: Start does that (below).
      if (!r || !watchStore.room() || !watchStore.started() || watchStore.staleForSwitch(r)) return;
      const path = `/watch/${r.mediaId}/${r.episode}`;
      if (path === followed) return;
      followed = path;
      if (location.pathname !== path) navigate(`${path}${r.dub ? "?dub=1" : ""}`);
    }),
  );
  createEffect(on(watchStore.room, () => (followed = "")));
  // The host pressed Start: everyone not on the episode yet goes to it (it plays when loaded).
  createEffect(
    on(watchStore.started, (started, was) => {
      const p = watchStore.nowPlaying();
      if (!started || was || !p || !watchStore.room()) return;
      const path = `/watch/${p.mediaId}/${p.episode}`;
      followed = path;
      if (location.pathname !== path) navigate(`${path}${p.dub ? "?dub=1" : ""}`);
    }),
  );

  const count = (k: "anime" | "manga" | "room") => {
    // Together: how many are in the room this device is in.
    if (k === "room") return watchStore.room() ? String(watchStore.members().length) : null;
    const h = home();
    if (!h || h.isSample) return null;
    return fmtCount(k === "anime" ? h.animeCount : h.mangaCount);
  };

  // Phone only: the More sheet. Closes on navigation, Esc and a tap outside.
  const [moreOpen, setMoreOpen] = createSignal(false);
  createEffect(on(() => location.pathname, () => setMoreOpen(false), { defer: true }));
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && moreOpen()) setMoreOpen(false);
  };
  window.addEventListener("keydown", onKey);
  onCleanup(() => window.removeEventListener("keydown", onKey));
  const moreActive = () => MORE.some((n) => location.pathname.startsWith(n.href));

  return (
    <div class="shell">
      <nav class="rail" aria-label="Main">
        <div class="wordmark" aria-label="B-Anime">
          B<span aria-hidden="true">·</span>ANIME
        </div>
        <ul class="rail-nav">
          {NAV.map((item) => (
            <li data-bar={item.bar ? "" : undefined}>
              <A href={item.href} end={item.end} activeClass="is-active" class="rail-link" aria-label={item.label} title={item.label}>
                <item.icon size={18} stroke-width={1.75} aria-hidden="true" />
                <span class="rail-label">{item.label}</span>
                <Show when={item.count && count(item.count)}>
                  {(c) => <span class="rail-count num">{c()}</span>}
                </Show>
              </A>
            </li>
          ))}
          <li class="rail-more-item">
            <button
              type="button"
              class="rail-link rail-more"
              classList={{ "is-active": moreActive() || moreOpen() }}
              aria-expanded={moreOpen()}
              aria-controls="more-sheet"
              onClick={() => setMoreOpen(!moreOpen())}
            >
              <Ellipsis size={18} stroke-width={1.75} aria-hidden="true" />
              <span class="rail-label">More</span>
              {/* The update notice lives in the More sheet: flag it on the button. */}
              <Show when={homeStore.update()?.ready}>
                <i class="more-badge" aria-label="Update ready" />
              </Show>
            </button>
          </li>
        </ul>
        {/* Android app: desktop only, just above Support. Opens the latest release (with the APK)
            in the browser. */}
        <Show when={inTauri && !onAndroid}>
          <button
            type="button"
            class="rail-link rail-support rail-android"
            aria-label="Get B-Anime for Android (opens your browser)"
            title="Get B-Anime for Android"
            onClick={() => void openAndroidDownload().catch(() => {})}
          >
            <Smartphone size={18} stroke-width={1.75} aria-hidden="true" />
            <span class="rail-label">Android app</span>
          </button>
        </Show>
        {/* Support: looks like the menu's items, kept at the foot of the rail. Opens the
            Patreon page in the browser. */}
        <Show when={inTauri}>
          <button
            type="button"
            class="rail-link rail-support"
            aria-label="Support B-Anime on Patreon (opens your browser)"
            title="Support B-Anime on Patreon"
            onClick={() => void openSupport().catch(() => {})}
          >
            <Heart size={18} stroke-width={1.75} aria-hidden="true" />
            <span class="rail-label">Support</span>
          </button>
        </Show>
        <UpdateNotice />
        <SyncFoot />
      </nav>
      <main class="main">{props.children}</main>
      <Show when={moreOpen()}>
        <div class="more-scrim" aria-hidden="true" onClick={() => setMoreOpen(false)} />
        <div class="more-sheet" id="more-sheet" role="dialog" aria-label="More">
          <ul class="more-nav">
            <For each={MORE}>
              {(item) => (
                <li>
                  <A href={item.href} activeClass="is-active" class="rail-link">
                    <item.icon size={18} stroke-width={1.75} aria-hidden="true" />
                    <span class="rail-label">{item.label}</span>
                  </A>
                </li>
              )}
            </For>
            <Show when={inTauri}>
              <li>
                <button
                  type="button"
                  class="rail-link"
                  aria-label="Support B-Anime on Patreon (opens your browser)"
                  onClick={() => void openSupport().catch(() => {})}
                >
                  <Heart size={18} stroke-width={1.75} aria-hidden="true" />
                  <span class="rail-label">Support</span>
                </button>
              </li>
            </Show>
          </ul>
          <UpdateNotice />
          <SyncFoot />
        </div>
      </Show>
      <div class="toast" role="status" aria-live="polite" data-open={toast() ? "" : undefined}>
        {toast()}
      </div>
    </div>
  );
}

/** Appears as soon as a new version is found: downloading first, then ready to install. */
function UpdateNotice() {
  const { update } = homeStore;
  const [restarting, setRestarting] = createSignal(false);
  return (
    <Show when={update()?.available}>
      {(u) => (
        <Show
          when={update()?.ready}
          fallback={
            <div class="rail-update" role="status">
              <p class="rail-update-text">
                Version <span class="num">{u().version}</span> is available
              </p>
              <p class="rail-update-text rail-update-sub">
                <Show when={update()?.error} fallback={<><i class="spin-dot" aria-hidden="true" /> Downloading…</>}>
                  Download failed, will retry
                </Show>
              </p>
            </div>
          }
        >
        <div class="rail-update" role="status">
          <p class="rail-update-text">
            Version <span class="num">{u().version}</span> is ready
          </p>
          <button
            type="button"
            class="btn btn-primary btn-sm"
            aria-label={`${UPDATE_ACTION} to version ${u().version}`}
            disabled={restarting()}
            onClick={() => {
              setRestarting(true);
              void installUpdate()
                // Android: the system installer is open and B-Anime stays (the user may cancel).
                .then(() => onAndroid && setRestarting(false))
                .catch((e) => {
                  setRestarting(false);
                  homeStore.notify(errorText(e));
                });
            }}
          >
            <RefreshCw size={14} stroke-width={2} aria-hidden="true" class="rail-update-icon" />
            <span class="rail-update-label">{restarting() ? UPDATE_BUSY : UPDATE_ACTION}</span>
          </button>
        </div>
        </Show>
      )}
    </Show>
  );
}

/** Sync status as evidence. Syncing is automatic; there is nothing to press. */
function SyncFoot() {
  const { home, account, now } = homeStore;
  const synced = () => account()?.lastSynced;
  return (
    <div class="sync">
      <Show
        when={account()?.viewer}
        fallback={
          <>
            <span class="sync-state is-off">
              <i class="dot" aria-hidden="true" /> AniList not connected
            </span>
            <A href="/settings" class="sync-link">Connect account</A>
          </>
        }
      >
        <Show
          when={!account()?.syncError}
          fallback={
            <>
              <span class="sync-state is-warn" title={account()?.syncError ?? undefined}>
                <i class="dot" aria-hidden="true" /> Can't reach AniList
              </span>
              <span class="sync-evidence">
                Retrying automatically{synced() ? <> · last synced {fmtAgo(synced()!, now())}</> : null}
              </span>
            </>
          }
        >
          <span class="sync-state">
            <i class="dot" aria-hidden="true" /> Synced with AniList
          </span>
          <span class="sync-evidence">
            {synced() ? fmtAgo(synced()!, now()) : "Syncing…"}
            {" · "}
            <span class="num">{fmtCount(home()?.sync.filesMatched ?? 0)}</span> files matched
          </span>
        </Show>
      </Show>
    </div>
  );
}
