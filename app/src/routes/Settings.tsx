import { For, Match, Show, Switch, createResource, createSignal, onCleanup } from "solid-js";

import {
  addLibraryRoot, assignFiles, browse, cancelLogin, checkForUpdate, errorText, getPlayerPrefs, ignoreFiles, inTauri,
  installUpdate, libraryStatus, login, logout, onLibraryChanged, pickFolder, removeLibraryRoot, setPlayerPref,
  unmatchedFiles,
} from "../lib/api";
import { fmtAgo, fmtCount } from "../lib/format";
import { homeStore } from "../lib/store";
import type { MediaCard, PlayerChoice, UnmatchedGroup } from "../lib/types";

type Busy = null | "login" | "logout";

export default function Settings() {
  const { account, setAccount, notify, now } = homeStore;
  const [busy, setBusy] = createSignal<Busy>(null);
  const [error, setError] = createSignal<string | null>(null);

  const run = async (kind: Exclude<Busy, null>, fn: () => Promise<void>) => {
    setBusy(kind);
    setError(null);
    try {
      await fn();
    } catch (e) {
      const text = errorText(e);
      if (text !== "Sign-in cancelled.") setError(text);
    } finally {
      setBusy(null);
    }
  };

  const connect = () =>
    run("login", async () => {
      const acc = await login();
      setAccount(acc);
      notify(`Connected as ${acc.viewer?.name}. Your lists are synced.`);
    });

  const disconnect = () =>
    run("logout", async () => {
      setAccount(await logout());
      notify("Disconnected from AniList. Your library files are untouched.");
    });

  return (
    <div class="page">
      <h1 class="page-title">Settings</h1>

      <section class="section" aria-labelledby="acct-h">
        <h2 id="acct-h" class="section-head">
          AniList account
          <small>Your lists and progress live on AniList; B-Anime keeps them in sync.</small>
        </h2>

        <Show when={error()}>
          <p class="inline-error" role="alert">{error()}</p>
        </Show>

        <Switch>
          <Match when={!inTauri}>
            <p class="empty">Open the desktop app to connect AniList. This browser preview can't sign in.</p>
          </Match>

          <Match when={account() && !account()!.clientConfigured}>
            <p class="empty">
              This build has no AniList client id, so sign-in is turned off. Set{" "}
              <code>BANIME_ANILIST_CLIENT_ID</code> and rebuild.
            </p>
          </Match>

          <Match when={busy() === "login"}>
            <div class="setting-row">
              <div class="setting-text">
                <p class="setting-title">Waiting for AniList…</p>
                <p class="setting-desc">Approve B-Anime in the browser tab that just opened, then come back here.</p>
              </div>
              <button type="button" class="btn" onClick={() => void cancelLogin()}>Cancel</button>
            </div>
          </Match>

          <Match when={account()?.viewer}>
            {(viewer) => (
              <div class="setting-row">
                <div class="account">
                  <Show
                    when={viewer().avatar?.medium}
                    fallback={<span class="avatar avatar-empty" aria-hidden="true">{viewer().name.slice(0, 1)}</span>}
                  >
                    {(src) => <img class="avatar" src={src()} alt="" width="40" height="40" />}
                  </Show>
                  <div class="setting-text">
                    <p class="setting-title">{viewer().name}</p>
                    <p class="setting-desc">
                      <Show
                        when={!account()?.syncError}
                        fallback={<span class="warn-text">Can't reach AniList right now. B-Anime will keep retrying.</span>}
                      >
                        Syncs automatically
                        <Show when={account()?.lastSynced}>{(t) => <> · last synced {fmtAgo(t(), now())}</>}</Show>
                      </Show>
                    </p>
                  </div>
                </div>
                <div class="setting-actions">
                  <button type="button" class="btn btn-quiet" disabled={busy() !== null} onClick={disconnect}>
                    Disconnect
                  </button>
                </div>
              </div>
            )}
          </Match>

          <Match when={account()}>
            <div class="setting-row">
              <div class="setting-text">
                <p class="setting-title">Not connected</p>
                <p class="setting-desc">Sign in to bring in your Watching list, progress and this week's airing schedule.</p>
              </div>
              <button type="button" class="btn btn-primary" onClick={connect}>Connect AniList</button>
            </div>
          </Match>
        </Switch>
      </section>

      <LibraryFolders />
      <PlaybackPrefs />
      <About />
    </div>
  );
}

const PHASE: Record<string, string> = {
  walking: "Looking for video files…",
  matching: "Matching titles to AniList",
  saving: "Saving…",
};

function LibraryFolders() {
  const { notify, scan, now } = homeStore;
  const [status, { refetch }] = createResource(libraryStatus);
  const [unmatched, { refetch: refetchUnmatched }] = createResource(unmatchedFiles);
  const [error, setError] = createSignal<string | null>(null);

  let unlisten: (() => void) | undefined;
  void onLibraryChanged(() => {
    void refetch();
    void refetchUnmatched();
  }).then((u) => (unlisten = u));
  onCleanup(() => unlisten?.());

  const add = async () => {
    setError(null);
    let path: string | null;
    try {
      path = await pickFolder();
    } catch (e) {
      setError(`Couldn't open the folder picker: ${errorText(e)}`);
      return;
    }
    if (!path) return;
    try {
      await addLibraryRoot(path);
      notify("Folder added. Scanning it now.");
      void refetch();
    } catch (e) {
      setError(errorText(e));
    }
  };
  const remove = async (path: string) => {
    try {
      await removeLibraryRoot(path);
      void refetch();
      void refetchUnmatched();
    } catch (e) {
      setError(errorText(e));
    }
  };

  const running = () => scan().running || status()?.scan.running;
  const last = () => status()?.last;

  return (
    <section class="section" aria-labelledby="lib-h">
      <h2 id="lib-h" class="section-head">
        Library folders
        <small>B-Anime watches these folders and matches new episodes automatically.</small>
      </h2>

      <Show when={inTauri} fallback={<p class="empty">Open the desktop app to add library folders.</p>}>
        <Show when={error()}>
          <p class="inline-error" role="alert">{error()}</p>
        </Show>

        <ul class="roots">
          <For each={status()?.roots ?? []}>
            {(root) => (
              <li class="root-row">
                <span class="root-path" data-selectable>{root}</span>
                <button type="button" class="btn btn-quiet btn-sm" onClick={() => void remove(root)}>Remove</button>
              </li>
            )}
          </For>
        </ul>
        <div class="setting-row">
          <p class="setting-desc" aria-live="polite">
            <Switch
              fallback={
                <Show when={last()} fallback="Add the folders where your anime lives. Nothing is moved or renamed.">
                  {(l) => (
                    <>
                      <span class="num">{fmtCount(l().files)}</span> files · <span class="num">{fmtCount(l().matched)}</span> matched
                      <Show when={l().unmatched}> · <span class="warn-text num">{fmtCount(l().unmatched)} need a look</span></Show>
                      <Show when={l().finishedAt}>{(t) => <> · scanned {fmtAgo(t(), now())}</>}</Show>
                    </>
                  )}
                </Show>
              }
            >
              <Match when={running()}>
                <span class="scan-live">
                  <i class="spin-dot" aria-hidden="true" />
                  {PHASE[scan().phase ?? "walking"]}
                  <Show when={scan().phase === "matching" && scan().total}>
                    {" "}<span class="num">{scan().done} / {scan().total}</span>
                  </Show>
                </span>
              </Match>
            </Switch>
          </p>
          <button type="button" class="btn btn-primary" onClick={() => void add()}>Add folder</button>
        </div>

        <Show when={unmatched()?.length}>
          <Unmatched groups={unmatched()!} onChanged={() => void refetchUnmatched()} />
        </Show>
      </Show>
    </section>
  );
}

function Unmatched(props: { groups: UnmatchedGroup[]; onChanged: () => void }) {
  const [open, setOpen] = createSignal<string | null>(null);
  const key = (g: UnmatchedGroup) => `${g.title}|${g.season ?? ""}`;
  return (
    <div class="unmatched">
      <h3 class="subhead">
        Couldn't match automatically <span class="num">{props.groups.length}</span>
      </h3>
      <ul class="unmatched-list">
        <For each={props.groups}>
          {(g) => (
            <li class="unmatched-group">
              <div class="unmatched-head">
                <div class="setting-text">
                  <p class="setting-title">
                    {g.title}
                    <Show when={g.season && g.season > 1}> · Season {g.season}</Show>
                  </p>
                  <p class="setting-desc num" title={g.files.map((f) => f.fileName).join("\n")}>
                    {g.files.length} {g.files.length === 1 ? "file" : "files"} · {g.files[0].fileName}
                  </p>
                </div>
                <div class="setting-actions">
                  <button type="button" class="btn btn-sm" aria-expanded={open() === key(g)} onClick={() => setOpen(open() === key(g) ? null : key(g))}>
                    Match…
                  </button>
                  <button
                    type="button"
                    class="btn btn-quiet btn-sm"
                    onClick={() => void ignoreFiles(g.files.map((f) => f.path)).then(props.onChanged)}
                  >
                    Ignore
                  </button>
                </div>
              </div>
              <Show when={open() === key(g)}>
                <MatchPicker
                  group={g}
                  onDone={() => {
                    setOpen(null);
                    props.onChanged();
                  }}
                />
              </Show>
            </li>
          )}
        </For>
      </ul>
    </div>
  );
}

function MatchPicker(props: { group: UnmatchedGroup; onDone: () => void }) {
  const { notify } = homeStore;
  const initial = props.group.season && props.group.season > 1 ? `${props.group.title} season ${props.group.season}` : props.group.title;
  const [query, setQuery] = createSignal(initial);
  const [results, setResults] = createSignal<MediaCard[] | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const search = async (e?: Event) => {
    e?.preventDefault();
    if (!query().trim()) return;
    setBusy(true);
    setError(null);
    try {
      setResults((await browse({ search: query().trim(), kind: "ANIME", page: 1 })).items);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };
  void search();

  const choose = async (m: MediaCard) => {
    try {
      await assignFiles(props.group.files.map((f) => f.path), m.id);
      notify(`Matched ${props.group.files.length} ${props.group.files.length === 1 ? "file" : "files"} to ${m.title}.`);
      props.onDone();
    } catch (err) {
      setError(errorText(err));
    }
  };

  return (
    <div class="source-picker">
      <form class="install-row" onSubmit={search}>
        <label class="search search-grow">
          <span class="sr-only">Search AniList</span>
          <input type="search" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} />
        </label>
        <button type="submit" class="btn" disabled={busy()}>{busy() ? "Searching…" : "Search"}</button>
      </form>
      <Show when={error()}>
        <p class="inline-error">{error()}</p>
      </Show>
      <Show when={results()?.length} fallback={<Show when={results()}><p class="empty">No results. Try a different title.</p></Show>}>
        <ul class="candidates">
          <For each={results()}>
            {(m) => (
              <li>
                <button type="button" class="candidate candidate-media" onClick={() => void choose(m)}>
                  <span class="candidate-cover" style={{ "background-color": m.color ?? "var(--z3)" }}>
                    <Show when={m.coverUrl}>{(src) => <img src={src()} alt="" loading="lazy" />}</Show>
                  </span>
                  <span class="candidate-title">{m.title}</span>
                  <span class="candidate-year num">
                    {[m.format, m.seasonYear, m.total ? `${m.total} eps` : null].filter(Boolean).join(" · ")}
                  </span>
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}

const PLAYER_LABEL: Record<PlayerChoice, string> = { builtin: "B-Anime player", mpv: "MPV", vlc: "VLC" };

function PlaybackPrefs() {
  const [prefs, { mutate }] = createResource(getPlayerPrefs);
  const choose = async (p: PlayerChoice) => {
    const prev = prefs();
    mutate((x) => (x ? { ...x, player: p } : x));
    try {
      await setPlayerPref(p);
    } catch {
      mutate(prev);
    }
  };
  const note = () =>
    ({
      builtin: "Plays inside B-Anime. Some MKV files need MPV.",
      mpv: "Plays every format. Progress syncs to AniList when you're 80% through.",
      vlc: "Plays every format. Progress isn't tracked; mark episodes watched yourself.",
    })[prefs()?.player ?? "builtin"];

  return (
    <Show when={inTauri}>
      <section class="section" aria-labelledby="play-h">
        <h2 id="play-h" class="section-head">Playback</h2>
        <div class="setting-row">
          <div class="setting-text">
            <p class="setting-title">Play library episodes in</p>
            <p class="setting-desc">{note()}</p>
          </div>
          <label class="select">
            <span class="select-label">Player</span>
            <select value={prefs()?.player ?? "builtin"} onChange={(e) => void choose(e.currentTarget.value as PlayerChoice)}>
              <For each={["builtin", "mpv", "vlc"] as PlayerChoice[]}>
                {(p) => (
                  <option value={p} disabled={p !== "builtin" && !prefs()?.available.includes(p)}>
                    {PLAYER_LABEL[p]}{p !== "builtin" && !prefs()?.available.includes(p) ? " (not installed)" : ""}
                  </option>
                )}
              </For>
            </select>
          </label>
        </div>
      </section>
    </Show>
  );
}

function About() {
  const { update, refetchUpdate } = homeStore;
  const [checking, setChecking] = createSignal(false);
  const [installing, setInstalling] = createSignal(false);
  const check = async () => {
    setChecking(true);
    try {
      await checkForUpdate();
    } finally {
      await refetchUpdate();
      setChecking(false);
    }
  };
  const install = async () => {
    setInstalling(true);
    try {
      await installUpdate();
    } catch {
      setInstalling(false);
    }
  };

  return (
    <section class="section" aria-labelledby="about-h">
      <h2 id="about-h" class="section-head">About</h2>
      <div class="setting-row">
        <div class="setting-text">
          <p class="setting-title">
            B-Anime <span class="ext-version num">{update()?.current}</span>
          </p>
          <p class="setting-desc">
            <Switch fallback="Updates install automatically when you restart. You're up to date.">
              <Match when={!inTauri || (update() && !update()!.enabled)}>Updates are off in development builds.</Match>
              <Match when={update()?.ready && update()?.available}>
                {(u) => <>Version {(u() as { version: string }).version} is downloaded and ready.</>}
              </Match>
              <Match when={update()?.available}>
                {(u) => <>Downloading version {(u() as { version: string }).version}…</>}
              </Match>
              <Match when={update()?.error}>
                <span class="warn-text">Couldn't check for updates: {update()!.error}</span>
              </Match>
            </Switch>
          </p>
        </div>
        <div class="setting-actions">
          <Show
            when={update()?.ready}
            fallback={
              <Show when={inTauri && update()?.enabled}>
                <button type="button" class="btn" disabled={checking()} onClick={() => void check()}>
                  {checking() ? "Checking…" : "Check now"}
                </button>
              </Show>
            }
          >
            <button type="button" class="btn btn-primary" disabled={installing()} onClick={() => void install()}>
              {installing() ? "Restarting…" : "Restart to update"}
            </button>
          </Show>
        </div>
      </div>
    </section>
  );
}
