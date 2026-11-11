import { For, Match, Show, Switch, createSignal } from "solid-js";
import { A } from "@solidjs/router";

import { Minus, Plus } from "../lib/icons";
import { inTauri } from "../lib/api";
import { fmtAgo } from "../lib/format";
import { homeStore } from "../lib/store";
import type { EntrySaver } from "../lib/entry";
import type { ListStatus, Media } from "../lib/types";

const STATUS_LABEL: Record<"ANIME" | "MANGA", Record<ListStatus, string>> = {
  ANIME: { CURRENT: "Watching", PLANNING: "Planning", COMPLETED: "Completed", PAUSED: "Paused", DROPPED: "Dropped", REPEATING: "Rewatching" },
  MANGA: { CURRENT: "Reading", PLANNING: "Planning", COMPLETED: "Completed", PAUSED: "Paused", DROPPED: "Dropped", REPEATING: "Rereading" },
};
const ORDER: ListStatus[] = ["CURRENT", "PLANNING", "COMPLETED", "PAUSED", "DROPPED", "REPEATING"];

/** Your list entry for one show: status, progress, score. Every change saves to AniList. */
export default function ListEditor(props: { media: Media; saver: EntrySaver; total: number | null }) {
  const { account, now } = homeStore;
  const s = () => props.saver;
  const e = () => s().entry();
  const anime = () => props.media.type === "ANIME";
  const labels = () => STATUS_LABEL[props.media.type];
  const unit = () => (anime() ? "Episodes" : "Chapters");
  const connected = () => !inTauri || !!account()?.viewer;

  // Score is typed as 0–10 and stored by AniList as 0–100.
  const [scoreDraft, setScoreDraft] = createSignal<string | null>(null);
  const scoreText = () => scoreDraft() ?? (e()?.score ? String(e()!.score! / 10) : "");
  const commitScore = () => {
    const raw = scoreDraft();
    setScoreDraft(null);
    if (raw === null) return;
    const v = raw.trim() === "" ? 0 : Number(raw);
    if (Number.isNaN(v)) return;
    const score = Math.round(Math.min(10, Math.max(0, v)) * 10);
    if (score !== (e()?.score ?? 0)) s().save({ score });
  };

  const [progressDraft, setProgressDraft] = createSignal<string | null>(null);
  const commitProgress = () => {
    const raw = progressDraft();
    setProgressDraft(null);
    if (raw === null) return;
    const v = Math.floor(Number(raw));
    if (!Number.isNaN(v) && v !== e()?.progress) s().save({ progress: v });
  };
  const step = (d: number) => s().save({ progress: (e()?.progress ?? 0) + d }, 600);
  const [confirming, setConfirming] = createSignal(false);

  return (
    <section class="list-editor" aria-label="Your list">
      <Switch>
        <Match when={!connected()}>
          <p class="editor-note">
            <A href="/settings">Connect AniList</A> to track this {anime() ? "show" : "series"}.
          </p>
        </Match>

        <Match when={!e()}>
          <div class="editor-add">
            <button type="button" class="btn btn-primary" onClick={() => s().save({ status: "CURRENT" })}>
              {anime() ? "Start watching" : "Start reading"}
            </button>
            <button type="button" class="btn" onClick={() => s().save({ status: "PLANNING" })}>
              Add to Planning
            </button>
          </div>
        </Match>

        <Match when={e()}>
          {(entry) => (
            <>
              <label class="select select-full">
                <span class="select-label">Status</span>
                <select value={entry().status} onChange={(ev) => s().save({ status: ev.currentTarget.value as ListStatus })}>
                  <For each={ORDER}>{(st) => <option value={st}>{labels()[st]}</option>}</For>
                </select>
              </label>

              <div class="editor-row">
                <span class="editor-label">{unit()}</span>
                <div class="stepper">
                  <button type="button" class="step" aria-label={`One ${anime() ? "episode" : "chapter"} less`} disabled={entry().progress <= 0} onClick={() => step(-1)}>
                    <Minus size={14} stroke-width={2.25} aria-hidden="true" />
                  </button>
                  <input
                    class="step-input num"
                    inputmode="numeric"
                    aria-label={`${unit()} ${anime() ? "watched" : "read"}`}
                    value={progressDraft() ?? String(entry().progress)}
                    onInput={(ev) => setProgressDraft(ev.currentTarget.value.replace(/\D/g, ""))}
                    onBlur={commitProgress}
                    onKeyDown={(ev) => ev.key === "Enter" && ev.currentTarget.blur()}
                  />
                  <span class="step-of num">/ {props.total ?? "?"}</span>
                  <button type="button" class="step" aria-label={`One more ${anime() ? "episode" : "chapter"}`} disabled={!!props.total && entry().progress >= props.total} onClick={() => step(1)}>
                    <Plus size={14} stroke-width={2.25} aria-hidden="true" />
                  </button>
                </div>
              </div>

              <div class="editor-row">
                <label class="editor-label" for="score-input">Score</label>
                <div class="score-field">
                  <input
                    id="score-input"
                    class="step-input num"
                    inputmode="decimal"
                    placeholder="–"
                    value={scoreText()}
                    onInput={(ev) => setScoreDraft(ev.currentTarget.value.replace(/[^\d.]/g, ""))}
                    onBlur={commitScore}
                    onKeyDown={(ev) => ev.key === "Enter" && ev.currentTarget.blur()}
                  />
                  <span class="step-of">/ 10</span>
                </div>
              </div>

              <Show
                when={confirming()}
                fallback={
                  <button type="button" class="editor-remove" onClick={() => setConfirming(true)}>
                    Remove from list
                  </button>
                }
              >
                <div class="editor-confirm" role="group" aria-label="Remove from list">
                  <p>
                    Remove <strong>{props.media.title.userPreferred}</strong> from your AniList list? Its status,{" "}
                    {anime() ? "episode" : "chapter"} progress and score there are deleted.
                  </p>
                  <div class="editor-confirm-actions">
                    <button
                      type="button"
                      class="btn btn-danger btn-sm"
                      disabled={s().state() === "saving"}
                      onClick={async () => {
                        if (await s().remove()) setConfirming(false);
                      }}
                    >
                      {s().state() === "saving" ? "Removing…" : "Remove"}
                    </button>
                    <button type="button" class="btn btn-quiet btn-sm" onClick={() => setConfirming(false)}>
                      Cancel
                    </button>
                  </div>
                </div>
              </Show>
            </>
          )}
        </Match>
      </Switch>

      <p class="save-state" role="status" data-state={s().state()} hidden={s().state() === "idle"}>
        <Switch>
          <Match when={s().state() === "saving" && confirming()}>Removing from AniList…</Match>
          <Match when={s().state() === "saving"}>Saving to AniList…</Match>
          <Match when={s().state() === "error"}>{s().error()}</Match>
          <Match when={s().state() === "saved" && s().savedAt()}>Saved to AniList {fmtAgo(s().savedAt()!, now())}</Match>
        </Switch>
      </p>
    </section>
  );
}

export { STATUS_LABEL as LIST_STATUS_LABEL };
