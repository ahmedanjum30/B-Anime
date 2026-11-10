import { For, Match, Show, Switch, createSignal } from "solid-js";
import { A } from "@solidjs/router";

import { clearSourceMatch, errorText, inTauri, searchSource, setSourceMatch } from "../lib/api";
import { searchTitles, type SourceState } from "../lib/source";
import type { Media, SourceResult } from "../lib/types";

/** Which online source this show uses and what it matched to, with a way to fix the match. */
export default function SourcePanel(props: { media: Media; source: SourceState }) {
  const s = () => props.source;
  const verb = () => (s().kind() === "anime" ? "watch" : "read");
  const extName = () => s().usable().find((e) => e.id === s().ext())?.name ?? "";

  const [picking, setPicking] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [results, setResults] = createSignal<SourceResult[] | null>(null);
  const [searching, setSearching] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const openPicker = () => {
    setQuery(searchTitles(props.media)[0] ?? "");
    setResults(null);
    setError(null);
    setPicking(true);
  };
  const runSearch = async (e?: Event) => {
    e?.preventDefault();
    const ext = s().ext();
    if (!ext || !query().trim()) return;
    setSearching(true);
    setError(null);
    try {
      setResults(await searchSource(ext, query().trim()));
    } catch (err) {
      setError(errorText(err));
    } finally {
      setSearching(false);
    }
  };
  const choose = async (r: SourceResult) => {
    const ext = s().ext();
    if (!ext) return;
    try {
      await setSourceMatch(props.media.id, ext, r.id, r.title);
      s().setMatch({ status: "matched", sourceId: r.id, title: r.title, auto: false });
      setPicking(false);
    } catch (err) {
      setError(errorText(err));
    }
  };
  const retry = async () => {
    const ext = s().ext();
    if (!ext) return;
    await clearSourceMatch(props.media.id, ext).catch(() => {});
    void s().rematch();
  };

  const candidates = () => {
    const m = s().match();
    return results() ?? (m?.status === "unmatched" ? m.candidates : []);
  };

  return (
    <Show when={inTauri}>
      <div class="source-panel">
        <Switch>
          <Match when={s().exts() && s().usable().length === 0}>
            <p class="source-line">
              To {verb()} here, <A href="/extensions">install a {s().kind()} extension</A>.
            </p>
          </Match>
          <Match when={s().ext()}>
            <div class="source-line">
              <Show
                when={s().usable().length > 1}
                fallback={<span class="source-name">{extName()}</span>}
              >
                <label class="select select-sm">
                  <span class="select-label">Source</span>
                  <select value={s().ext() ?? ""} onChange={(e) => s().setExt(e.currentTarget.value)}>
                    <For each={s().usable()}>{(e) => <option value={e.id}>{e.name}</option>}</For>
                  </select>
                </label>
              </Show>
              <Switch>
                <Match when={s().match.loading}>
                  <span class="source-status">Finding this {s().kind() === "anime" ? "show" : "series"}…</span>
                </Match>
                <Match when={s().match.error}>
                  <span class="source-status is-error">{errorText(s().match.error)}</span>
                  <button type="button" class="link-btn" onClick={() => void retry()}>Try again</button>
                </Match>
                <Match when={s().match()?.status === "matched" && s().match()}>
                  {(m) => (
                    <>
                      <span class="source-status">
                        Matched to <strong>{(m() as { title: string }).title}</strong>
                      </span>
                      <button type="button" class="link-btn" onClick={openPicker}>Wrong match?</button>
                    </>
                  )}
                </Match>
                <Match when={s().match()?.status === "unmatched"}>
                  <span class="source-status is-warn">No confident match. Pick the right one:</span>
                </Match>
              </Switch>
            </div>

            <Show when={picking() || s().match()?.status === "unmatched"}>
              <div class="source-picker">
                <form class="install-row" onSubmit={runSearch}>
                  <label class="search search-grow">
                    <span class="sr-only">Search {extName()}</span>
                    <input
                      type="search"
                      value={query()}
                      placeholder={`Search ${extName()}`}
                      onInput={(e) => setQuery(e.currentTarget.value)}
                    />
                  </label>
                  <button type="submit" class="btn" disabled={searching()}>{searching() ? "Searching…" : "Search"}</button>
                  <Show when={picking()}>
                    <button type="button" class="btn btn-quiet" onClick={() => setPicking(false)}>Cancel</button>
                  </Show>
                </form>
                <Show when={error()}>
                  <p class="inline-error">{error()}</p>
                </Show>
                <Show when={candidates().length} fallback={<Show when={results()}><p class="empty">No results.</p></Show>}>
                  <ul class="candidates">
                    <For each={candidates()}>
                      {(r) => (
                        <li>
                          <button type="button" class="candidate" onClick={() => void choose(r)}>
                            <span class="candidate-title">{r.title}</span>
                            <Show when={r.year}><span class="candidate-year num">{r.year}</span></Show>
                          </button>
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>
              </div>
            </Show>
          </Match>
        </Switch>
      </div>
    </Show>
  );
}
