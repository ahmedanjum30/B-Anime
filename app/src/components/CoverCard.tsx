import { Show } from "solid-js";
import { A } from "@solidjs/router";

import Mark from "./Mark";
import type { LibraryEntry, LibraryKind } from "../lib/types";

export const FORMAT_LABEL: Record<string, string> = {
  TV: "TV",
  TV_SHORT: "TV short",
  MOVIE: "Movie",
  SPECIAL: "Special",
  OVA: "OVA",
  ONA: "ONA",
  MUSIC: "Music",
  MANGA: "Manga",
  NOVEL: "Light novel",
  ONE_SHOT: "One-shot",
};

const IN_PROGRESS = new Set(["CURRENT", "REPEATING", "PAUSED", "DROPPED"]);

export default function CoverCard(props: { entry: LibraryEntry; kind: LibraryKind }) {
  const e = () => props.entry;
  const anime = () => props.kind === "ANIME";
  const format = () => (e().format ? FORMAT_LABEL[e().format!] ?? e().format : null);

  // Episodes out that the user hasn't watched yet (airing shows on Watching).
  const behind = () => e().status === "CURRENT" && e().released != null && e().released! > e().progress;
  const inProgress = () => IN_PROGRESS.has(e().status);
  const ratio = () => {
    const of = e().released ?? e().total;
    return of ? Math.min(1, e().progress / of) : 0;
  };

  const meta = () => {
    const unit = anime() ? "Ep" : "Ch";
    const plural = anime() ? "eps" : "ch";
    const parts: string[] = [];
    if (inProgress()) parts.push(`${unit} ${e().progress}${e().total ? ` / ${e().total}` : ""}`);
    else if (e().total) parts.push(`${e().total} ${e().total === 1 && anime() ? "ep" : plural}`);
    if (format()) parts.push(format()!);
    return parts.join(" · ");
  };

  return (
    <A href={`/show/${e().mediaId}`} class="cover-card" title={e().title}>
      <div class="cover" style={{ "background-color": e().color ?? "var(--z3)" }}>
        <Show when={e().coverUrl}>
          {(src) => <img src={src()} alt="" loading="lazy" decoding="async" draggable={false} />}
        </Show>
        <Show when={inProgress() && ratio() > 0}>
          <div class="cover-progress" aria-hidden="true">
            <i style={{ "--p": ratio() }} />
          </div>
        </Show>
      </div>
      <p class="cover-title">
        <Show when={behind()}>
          <Mark kind="new" />
        </Show>
        {e().title}
      </p>
      <p class="cover-meta num">
        <span class="cover-meta-main">{meta()}</span>
        <Show when={e().score}>
          {(s) => (
            <span class="cover-score" title="Your score">
              <span class="sr-only">Your score </span>
              {(s() / 10).toFixed(1)}
            </span>
          )}
        </Show>
      </p>
    </A>
  );
}
