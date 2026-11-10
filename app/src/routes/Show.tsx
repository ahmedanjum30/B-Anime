import { For, Match, Show as When, Switch, createEffect, createMemo, createResource, createSignal, on } from "solid-js";
import { A, useNavigate, useParams } from "@solidjs/router";

import ListEditor from "../components/ListEditor";
import EpisodeList from "../components/EpisodeList";
import ChapterList from "../components/ChapterList";
import SourcePanel from "../components/SourcePanel";
import { createSource } from "../lib/source";
import { FORMAT_LABEL } from "../components/CoverCard";
import { ArrowLeft, ExternalLink } from "../lib/icons";
import { errorText, getMedia, inTauri, openAniList } from "../lib/api";
import { fmtAgo, fmtCount } from "../lib/format";
import { homeStore } from "../lib/store";
import { createEntrySaver } from "../lib/entry";
import type { Media, MediaRef } from "../lib/types";

const STATUS_TEXT: Record<string, string> = {
  FINISHED: "Finished",
  RELEASING: "Airing",
  NOT_YET_RELEASED: "Not yet released",
  CANCELLED: "Cancelled",
  HIATUS: "On hiatus",
};
const SEASON: Record<string, string> = { WINTER: "Winter", SPRING: "Spring", SUMMER: "Summer", FALL: "Fall" };
const SOURCE: Record<string, string> = {
  ORIGINAL: "Original",
  MANGA: "Manga",
  LIGHT_NOVEL: "Light novel",
  VISUAL_NOVEL: "Visual novel",
  VIDEO_GAME: "Video game",
  NOVEL: "Novel",
  WEB_NOVEL: "Web novel",
  OTHER: "Other",
};
const RELATION: Record<string, string> = {
  ADAPTATION: "Adaptation",
  PREQUEL: "Prequel",
  SEQUEL: "Sequel",
  PARENT: "Parent story",
  SIDE_STORY: "Side story",
  CHARACTER: "Shares characters",
  SUMMARY: "Summary",
  ALTERNATIVE: "Alternative",
  SPIN_OFF: "Spin-off",
  OTHER: "Related",
  SOURCE: "Source",
  COMPILATION: "Compilation",
  CONTAINS: "Contains",
};
const RELATION_ORDER = ["PREQUEL", "SEQUEL", "PARENT", "SIDE_STORY", "SPIN_OFF", "ALTERNATIVE", "SOURCE", "ADAPTATION"];

/** AniList descriptions carry <br>, <i>, entities; render as plain text with line breaks. */
function plainText(html: string | null): string {
  if (!html) return "";
  const withBreaks = html.replace(/<br\s*\/?>/gi, "\n");
  const text = new DOMParser().parseFromString(withBreaks, "text/html").body.textContent ?? "";
  return text.replace(/\n{3,}/g, "\n\n").trim();
}

export default function Show() {
  const params = useParams();
  const id = () => Number(params.id);
  const { notify, now } = homeStore;
  const [page, { refetch }] = createResource(id, getMedia);

  // One saver for the page, shared by the editor and the episode list.
  const total = () => {
    const m = page()?.media;
    return m ? (m.type === "ANIME" ? m.episodes : m.chapters) : null;
  };
  const navigate = useNavigate();
  const source = createSource(() => page()?.media);

  const saver = createEntrySaver({
    mediaId: id,
    total,
    initial: () => page()?.listEntry ?? null,
    notify,
  });

  // Related links reuse this page: start each show collapsed and at the top.
  const [expanded, setExpanded] = createSignal(false);
  createEffect(
    on(id, () => {
      setExpanded(false);
      document.querySelector(".main")?.scrollTo({ top: 0 });
    }, { defer: true }),
  );

  return (
    <div class="show-page">
      <Switch>
        <Match when={page.error}>
          <div class="page">
            <BackLink />
            <div class="notice is-error" role="alert">
              <p><strong>Couldn't load this show.</strong> {errorText(page.error)}</p>
              <button type="button" class="btn" onClick={() => void refetch()}>Try again</button>
            </div>
          </div>
        </Match>
        <Match when={page()}>
          {(p) => {
            const m = () => p().media;
            const description = createMemo(() => plainText(m().description));
            return (
              <>
                <header class="show-hero" style={{ "--tint": m().coverImage.color ?? "var(--z3)" }}>
                  <When when={m().bannerImage}>
                    {(src) => <img class="show-banner" src={src()} alt="" decoding="async" />}
                  </When>
                  <div class="show-hero-scrim" />
                  <div class="show-hero-inner">
                    <BackLink />
                  </div>
                </header>

                <div class="show-body">
                  <aside class="show-side">
                    <div class="show-cover" style={{ "background-color": m().coverImage.color ?? "var(--z3)" }}>
                      <When when={m().coverImage.extraLarge ?? m().coverImage.large}>
                        {(src) => <img src={src()} alt={`${m().title.userPreferred} cover`} decoding="async" />}
                      </When>
                    </div>
                    <ListEditor media={m()} saver={saver} total={total()} />
                    <Facts media={m()} />
                  </aside>

                  <main class="show-main">
                    <div class="show-titles">
                      <h1 class="show-title">{m().title.userPreferred}</h1>
                      <When when={altTitle(m())}>{(t) => <p class="show-alt">{t()}</p>}</When>
                      <p class="show-meta num">{metaLine(m())}</p>
                      <When when={m().genres.length}>
                        <p class="show-genres">{m().genres.join(" · ")}</p>
                      </When>
                    </div>

                    <When when={p().fromCache}>
                      <p class="cache-note">
                        Offline copy from {fmtAgo(p().fetchedAt, now())}. AniList couldn't be reached.
                      </p>
                    </When>

                    <When when={description()}>
                      <section class="section" aria-labelledby="about-h">
                        <h2 id="about-h" class="section-head">About</h2>
                        <p class="synopsis" data-expanded={expanded() ? "" : undefined} data-selectable>
                          {description()}
                        </p>
                        <When when={description().length > 420}>
                          <button type="button" class="link-btn" onClick={() => setExpanded(!expanded())}>
                            {expanded() ? "Show less" : "Read more"}
                          </button>
                        </When>
                      </section>
                    </When>

                    <When
                      when={m().type === "ANIME"}
                      fallback={
                        <When when={inTauri && source.usable().length > 0} fallback={<SourcePanel media={m()} source={source} />}>
                          <ChapterList
                            items={source.items()}
                            loading={source.items.loading || source.match.loading}
                            saver={saver}
                            total={total()}
                            panel={<SourcePanel media={m()} source={source} />}
                            onRead={(c) => navigate(`/read/${m().id}/${c.number}?ext=${encodeURIComponent(source.ext()!)}`)}
                          />
                        </When>
                      }
                    >
                      <EpisodeList
                        media={m()}
                        saver={saver}
                        localFiles={p().localFiles}
                        panel={<SourcePanel media={m()} source={source} />}
                        playable={(ep) => source.byNumber().has(ep)}
                        onPlay={(ep) => navigate(`/watch/${m().id}/${ep}?ext=${encodeURIComponent(source.ext()!)}`)}
                      />
                    </When>

                    <Related title="Related" id="rel-h" items={relations(m())} />
                    <Related
                      title="If you like this"
                      id="rec-h"
                      items={m()
                        .recommendations.nodes.map((n) => n.mediaRecommendation)
                        .filter((r): r is MediaRef => !!r)
                        .map((node) => ({ node, label: FORMAT_LABEL[node.format ?? ""] ?? null }))}
                    />

                    <When when={inTauri}>
                      <button type="button" class="link-btn ext" onClick={() => void openAniList(m().id, m().type)}>
                        Open on AniList <ExternalLink size={14} stroke-width={2} aria-hidden="true" />
                      </button>
                    </When>
                  </main>
                </div>
              </>
            );
          }}
        </Match>
        <Match when={page.loading}>
          <div class="show-hero sk-hero" />
          <div class="show-body" aria-busy="true" aria-label="Loading show">
            <div class="show-side"><div class="show-cover sk-cover" /></div>
            <div class="show-main"><div class="sk-line wide" /><div class="sk-line" /></div>
          </div>
        </Match>
      </Switch>
    </div>
  );
}

function BackLink() {
  const back = () => (history.length > 1 ? history.back() : (location.href = "/library"));
  return (
    <button type="button" class="back-link" onClick={back}>
      <ArrowLeft size={16} stroke-width={2} aria-hidden="true" /> Back
    </button>
  );
}

function altTitle(m: Media): string | null {
  const main = m.title.userPreferred;
  const alts = [m.title.english, m.title.native].filter((t): t is string => !!t && t !== main);
  return alts.length ? alts.join(" · ") : null;
}

function metaLine(m: Media): string {
  const parts: string[] = [];
  if (m.format) parts.push(FORMAT_LABEL[m.format] ?? m.format);
  if (m.season && m.seasonYear) parts.push(`${SEASON[m.season]} ${m.seasonYear}`);
  else if (m.startDate.year) parts.push(String(m.startDate.year));
  if (m.type === "ANIME" && m.episodes) parts.push(`${m.episodes} ${m.episodes === 1 ? "episode" : "episodes"}`);
  if (m.type === "MANGA" && m.chapters) parts.push(`${m.chapters} chapters`);
  if (m.status) parts.push(STATUS_TEXT[m.status] ?? m.status);
  return parts.join(" · ");
}

function Facts(props: { media: Media }) {
  const m = () => props.media;
  const date = (d: Media["startDate"]) =>
    d.year ? new Date(d.year, (d.month ?? 1) - 1, d.day ?? 1).toLocaleDateString(undefined, { year: "numeric", month: "short", ...(d.day ? { day: "numeric" } : {}) }) : null;
  const rows = () =>
    [
      ["Studio", m().studios.nodes.map((s) => s.name).join(", ") || null],
      ["Source", m().source ? SOURCE[m().source!] ?? m().source : null],
      ["Episode length", m().duration ? `${m().duration} min` : null],
      ["Started", date(m().startDate)],
      ["Ended", date(m().endDate)],
      ["AniList score", m().averageScore ? `${m().averageScore}%` : null],
      ["On lists", m().popularity ? fmtCount(m().popularity!) : null],
    ].filter((r): r is [string, string] => !!r[1]);
  return (
    <dl class="facts">
      <For each={rows()}>
        {([k, v]) => (
          <div class="fact">
            <dt>{k}</dt>
            <dd class="num">{v}</dd>
          </div>
        )}
      </For>
    </dl>
  );
}

function relations(m: Media) {
  return [...m.relations.edges]
    .filter((e) => e.node.type === "ANIME" || e.relationType === "SOURCE" || e.relationType === "ADAPTATION")
    .sort((a, b) => rank(a.relationType) - rank(b.relationType))
    .map((e) => ({ node: e.node, label: RELATION[e.relationType] ?? e.relationType }));
}
const rank = (t: string) => {
  const i = RELATION_ORDER.indexOf(t);
  return i === -1 ? 99 : i;
};

function Related(props: { title: string; id: string; items: { node: MediaRef; label: string | null }[] }) {
  return (
    <When when={props.items.length}>
      <section class="section" aria-labelledby={props.id}>
        <h2 id={props.id} class="section-head">{props.title}</h2>
        <div class="related">
          <For each={props.items}>
            {(r) => (
              <A href={`/show/${r.node.id}`} class="related-card" title={r.node.title.userPreferred}>
                <div class="cover" style={{ "background-color": r.node.coverImage.color ?? "var(--z3)" }}>
                  <When when={r.node.coverImage.large}>
                    {(src) => <img src={src()} alt="" loading="lazy" decoding="async" />}
                  </When>
                </div>
                <When when={r.label}>{(l) => <p class="related-label">{l()}</p>}</When>
                <p class="related-title">{r.node.title.userPreferred}</p>
              </A>
            )}
          </For>
        </div>
      </section>
    </When>
  );
}
