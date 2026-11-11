import { Show } from "solid-js";
import { Play } from "../lib/icons";

import Mark from "./Mark";
import type { UpNext } from "../lib/types";
import { fmtClock } from "../lib/format";
import { homeStore } from "../lib/store";

export default function UpNextCard(props: { item: UpNext; onPlay: (item: UpNext) => void }) {
  const { hoverMedia, setHoverMedia } = homeStore;
  const it = () => props.item;
  const linked = () => hoverMedia() === it().mediaId;

  const meta = () => {
    const parts = [`Ep ${it().episode}${it().totalEpisodes ? ` of ${it().totalEpisodes}` : ""}`];
    if (it().resumeS) parts.push(`resume at ${fmtClock(it().resumeS!)}`);
    else if (it().durationMin) parts.push(`${it().durationMin} min`);
    return parts.join(" · ");
  };

  return (
    <button
      type="button"
      class="upnext-card"
      data-linked={linked() ? "" : undefined}
      onClick={() => props.onPlay(it())}
      onPointerEnter={() => setHoverMedia(it().mediaId)}
      onPointerLeave={() => setHoverMedia(null)}
      onFocus={() => setHoverMedia(it().mediaId)}
      onBlur={() => setHoverMedia(null)}
      aria-label={`Play ${it().title}, ${meta()}`}
    >
      <div class="upnext-art" style={{ "background-color": it().color ?? "var(--z3)" }}>
        <Show when={it().bannerUrl ?? it().coverUrl}>
          {(src) => <img src={src()} alt="" loading="lazy" decoding="async" />}
        </Show>
        <span class="ep-chip num">EP {it().episode}</span>
        <span class="play-badge" aria-hidden="true">
          <Play size={18} fill="currentColor" stroke-width={0} />
        </span>
      </div>
      <div class="upnext-meta">
        <h3 class="upnext-title">
          <Show when={it().mark}>{(m) => <Mark kind={m()} />}</Show>
          <span class="upnext-name">{it().title}</span>
        </h3>
        <p class="upnext-line num">{meta()}</p>
        <p class="upnext-source" data-missing={it().source ? undefined : ""} title={it().source ?? undefined}>
          {it().source ?? "Not in your library"}
        </p>
        <div class="progress" aria-hidden="true" title={it().episodeProgress > 0 ? `${Math.round(it().episodeProgress * 100)}% of this episode watched` : undefined}>
          <i style={{ "--p": it().episodeProgress }} />
        </div>
      </div>
    </button>
  );
}
