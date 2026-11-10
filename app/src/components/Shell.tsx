import type { ParentProps } from "solid-js";
import { Show } from "solid-js";
import { A } from "@solidjs/router";
import { House, Library, CalendarDays, BookOpen, Compass, Puzzle, Settings } from "../lib/icons";

import { homeStore } from "../lib/store";
import { fmtAgo, fmtCount } from "../lib/format";

const NAV = [
  { href: "/", label: "Home", icon: House, end: true },
  { href: "/library", label: "Library", icon: Library, count: "anime" as const },
  { href: "/schedule", label: "Schedule", icon: CalendarDays },
  { href: "/manga", label: "Manga", icon: BookOpen, count: "manga" as const },
  { href: "/discover", label: "Discover", icon: Compass },
  { href: "/extensions", label: "Extensions", icon: Puzzle },
  { href: "/settings", label: "Settings", icon: Settings },
];

export default function Shell(props: ParentProps) {
  const { home, toast } = homeStore;

  const count = (k: "anime" | "manga") => {
    const h = home();
    if (!h || h.isSample) return null;
    return fmtCount(k === "anime" ? h.animeCount : h.mangaCount);
  };

  return (
    <div class="shell">
      <nav class="rail" aria-label="Main">
        <div class="wordmark" aria-label="B-Anime">
          B<span aria-hidden="true">·</span>ANIME
        </div>
        <ul class="rail-nav">
          {NAV.map((item) => (
            <li>
              <A href={item.href} end={item.end} activeClass="is-active" class="rail-link" aria-label={item.label} title={item.label}>
                <item.icon size={18} stroke-width={1.75} aria-hidden="true" />
                <span class="rail-label">{item.label}</span>
                <Show when={item.count && count(item.count)}>
                  {(c) => <span class="rail-count num">{c()}</span>}
                </Show>
              </A>
            </li>
          ))}
        </ul>
        <SyncFoot />
      </nav>
      <main class="main">{props.children}</main>
      <div class="toast" role="status" aria-live="polite" data-open={toast() ? "" : undefined}>
        {toast()}
      </div>
    </div>
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
