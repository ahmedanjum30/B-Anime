import { For, Match, Show, Switch, createMemo, createResource, createSignal } from "solid-js";

import {
  checkExtensionUpdates, errorText, getExtensionRepos, inTauri, installExtension, listExtensions, previewExtension,
  readExtensionRepo, removeExtension, setExtensionEnabled, setExtensionRepos, tryExtension, updateExtension,
  installLocalExtension, isLocalExtension, officialExtensionRepo,
} from "../lib/api";
import { Plus, Search } from "../lib/icons";
import { homeStore } from "../lib/store";
import type { ExtCallOutput, ExtKind, ExtManifest, InstalledExtension, Repo, RepoEntry } from "../lib/types";

const KIND_LABEL = { anime: "Anime", manga: "Manga" } as const;

/** Numeric-aware: "1.10.0" is newer than "1.9.2". */
const newer = (a: string, b: string) => {
  const p = (v: string) => v.split(/\D+/).filter(Boolean).map(Number);
  const [x, y] = [p(a), p(b)];
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0);
  }
  return false;
};

type Tab = "browse" | "installed" | "repos";
// The tab survives navigating away and back.
const [tab, setTab] = createSignal<Tab>("browse");

export default function Extensions() {
  const { notify } = homeStore;
  const [installed, { refetch, mutate }] = createResource(listExtensions);
  const [updates, { refetch: recheck }] = createResource(checkExtensionUpdates);
  const updateFor = (id: string) => updates()?.find((u) => u.id === id)?.version;

  // Repositories and their listings, shared by Browse and Repositories.
  const [repoUrls, { mutate: setRepoUrls }] = createResource(getExtensionRepos);
  const [repos, { refetch: reloadRepos }] = createResource(
    () => repoUrls(),
    async (urls) =>
      Promise.all(
        urls.map((url) =>
          readExtensionRepo(url).then(
            (repo): { url: string; repo: Repo | null; error: string | null } => ({ url, repo, error: null }),
            (e): { url: string; repo: Repo | null; error: string | null } => ({ url, repo: null, error: errorText(e) }),
          ),
        ),
      ),
  );

  // B-Anime's own repository, listed in its own panel and never removable.
  type RepoState = { url: string; repo: Repo | null; error: string | null };
  const [official, { refetch: reloadOfficial }] = createResource(async (): Promise<RepoState | null> => {
    if (!inTauri) return null;
    const url = await officialExtensionRepo();
    return readExtensionRepo(url).then(
      (repo) => ({ url, repo, error: null }),
      (e) => ({ url, repo: null, error: errorText(e) }),
    );
  });

  const [installing, setInstalling] = createSignal<string | null>(null);
  const install = async (manifestUrl: string, name?: string) => {
    setInstalling(manifestUrl);
    try {
      const ext = await installExtension(manifestUrl);
      notify(`Installed ${ext.name}.`);
      await refetch();
      void recheck();
      return true;
    } catch (e) {
      notify(`Couldn't install ${name ?? "the extension"}: ${errorText(e)}`);
      return false;
    } finally {
      setInstalling(null);
    }
  };

  const saveRepos = async (next: string[]) => {
    const prev = repoUrls() ?? [];
    setRepoUrls(next);
    try {
      await setExtensionRepos(next);
    } catch (e) {
      setRepoUrls(prev);
      notify(errorText(e));
    }
  };

  const installedCount = () => installed()?.length ?? 0;
  const browseCount = () => (repos() ?? []).reduce((n, r) => n + (r.repo?.extensions.length ?? 0), 0);

  return (
    <div class="page page-wide">
      <h1 class="page-title">Extensions</h1>
      <p class="page-lede">
        Extensions add sources for watching and reading online. Install only extensions you trust: they can reach
        the sites they're written for, but never your files.
      </p>

      <Show when={inTauri} fallback={<p class="empty">Open the desktop app to manage extensions.</p>}>
        <div class="tabs ext-tabs" role="tablist" aria-label="Extensions">
          <button type="button" role="tab" class="tab" aria-selected={tab() === "browse"} onClick={() => setTab("browse")}>
            Browse <span class="tab-count num">{browseCount()}</span>
          </button>
          <button type="button" role="tab" class="tab" aria-selected={tab() === "installed"} onClick={() => setTab("installed")}>
            Installed <span class="tab-count num">{installedCount()}</span>
            <Show when={updates()?.length}>
              <span class="tab-badge num">{updates()!.length} update{updates()!.length === 1 ? "" : "s"}</span>
            </Show>
          </button>
          <button type="button" role="tab" class="tab" aria-selected={tab() === "repos"} onClick={() => setTab("repos")}>
            Repositories <span class="tab-count num">{(repoUrls() ?? []).filter((u) => u !== official()?.url).length + 1}</span>
          </button>
        </div>

        <Switch>
          <Match when={tab() === "browse"}>
            <Browse
              official={official() ?? null}
              officialLoading={official.loading}
              onRetryOfficial={() => void reloadOfficial()}
              repos={(repos() ?? []).filter((r) => r.url !== official()?.url)}
              loading={repos.loading}
              installed={installed() ?? []}
              installing={installing()}
              onInstall={install}
              onUpdate={async (id) => {
                setInstalling(id);
                try {
                  const ext = await updateExtension(id);
                  notify(`Updated ${ext.name} to ${ext.version}.`);
                  await refetch();
                  void recheck();
                } catch (e) {
                  notify(errorText(e));
                } finally {
                  setInstalling(null);
                }
              }}
              onAddRepos={() => setTab("repos")}
            />
          </Match>

          <Match when={tab() === "installed"}>
            <Switch>
              <Match when={installed.error}>
                <p class="inline-error">Couldn't list extensions: {errorText(installed.error)}</p>
              </Match>
              <Match when={installed() && installed()!.length === 0}>
                <div class="empty-block">
                  <p>No extensions installed yet.</p>
                  <button type="button" class="btn btn-primary" onClick={() => setTab("browse")}>Browse extensions</button>
                </div>
              </Match>
              <Match when={installed()}>
                <ul class="ext-list">
                  <For each={installed()}>
                    {(ext) => (
                      <ExtensionRow
                        ext={ext}
                        update={updateFor(ext.id)}
                        onChanged={async () => {
                          await refetch();
                          void recheck();
                        }}
                        onToggle={(enabled) => mutate((list) => list?.map((x) => (x.id === ext.id ? { ...x, enabled } : x)))}
                      />
                    )}
                  </For>
                </ul>
              </Match>
            </Switch>
            <InstallFromUrl installedIds={() => installed()?.map((x) => x.id) ?? []} onInstall={install} />
            <Show when={import.meta.env.DEV}>
              <InstallFromFile
                onInstalled={async (ext) => {
                  notify(`Installed ${ext.name} from a file.`);
                  await refetch();
                }}
              />
            </Show>
          </Match>

          <Match when={tab() === "repos"}>
            <Repositories
              official={official() ?? null}
              urls={(repoUrls() ?? []).filter((u) => u !== official()?.url)}
              repos={repos() ?? []}
              onSave={(next) => void saveRepos(next)}
              onReload={() => void reloadRepos()}
            />
          </Match>
        </Switch>
      </Show>
    </div>
  );
}

// ── Browse: every extension from every repository ──────────────────────────────

type RepoCard = RepoEntry & { repoName: string };

function Browse(props: {
  official: { url: string; repo: Repo | null; error: string | null } | null;
  officialLoading: boolean;
  onRetryOfficial: () => void;
  repos: { url: string; repo: Repo | null; error: string | null }[];
  loading: boolean;
  installed: InstalledExtension[];
  installing: string | null;
  onInstall: (manifestUrl: string, name: string) => Promise<boolean>;
  onUpdate: (id: string) => Promise<void>;
  onAddRepos: () => void;
}) {
  const [kind, setKind] = createSignal<"all" | ExtKind>("all");
  const [query, setQuery] = createSignal("");

  const officialCards = createMemo<RepoCard[]>(() =>
    (props.official?.repo?.extensions ?? []).map((e) => ({ ...e, repoName: "B-Anime" })),
  );
  // Everything from the user's repositories, minus what B-Anime's own panel already lists.
  const cards = createMemo<RepoCard[]>(() => {
    const seen = new Set<string>(officialCards().map((c) => c.id ?? c.manifestUrl));
    const out: RepoCard[] = [];
    for (const { url, repo } of props.repos) {
      for (const e of repo?.extensions ?? []) {
        const key = e.id ?? e.manifestUrl;
        if (seen.has(key)) continue;
        seen.add(key);
        out.push({ ...e, repoName: repo?.name ?? new URL(url).host });
      }
    }
    return out;
  });
  const matches = (c: RepoCard) => {
    const q = query().trim().toLowerCase();
    return (
      (kind() === "all" || c.kind === kind()) &&
      (!q || c.name.toLowerCase().includes(q) || (c.description ?? "").toLowerCase().includes(q) || (c.author ?? "").toLowerCase().includes(q))
    );
  };
  const officialShown = createMemo(() => officialCards().filter(matches));
  const shown = createMemo(() => cards().filter(matches));
  const failed = () => props.repos.filter((r) => r.error);
  const cardProps = () => ({
    installed: props.installed,
    installing: props.installing,
    onInstall: props.onInstall,
    onUpdate: props.onUpdate,
  });

  return (
    <>
      <div class="toolbar">
        <label class="search search-sm">
          <Search size={16} stroke-width={1.75} aria-hidden="true" />
          <span class="sr-only">Search extensions</span>
          <input type="search" placeholder="Search extensions" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} />
        </label>
        <div class="segmented" role="group" aria-label="Type">
          <button type="button" aria-pressed={kind() === "all"} onClick={() => setKind("all")}>All</button>
          <button type="button" aria-pressed={kind() === "anime"} onClick={() => setKind("anime")}>Anime</button>
          <button type="button" aria-pressed={kind() === "manga"} onClick={() => setKind("manga")}>Manga</button>
        </div>
        <p class="toolbar-count num">{officialShown().length + shown().length} shown</p>
      </div>

      <section class="ext-panel ext-panel-official" aria-labelledby="official-h">
        <div class="ext-panel-head">
          <h2 id="official-h" class="subhead subhead-plain">B-Anime</h2>
          <p class="setting-desc">
            Extensions made by the B-Anime project.
          </p>
        </div>
        <Switch>
          <Match when={props.official?.error}>
            <p class="inline-error">
              Couldn't read B-Anime's repository: {props.official!.error}{" "}
              <button type="button" class="link-btn" onClick={props.onRetryOfficial}>Try again</button>
            </p>
          </Match>
          <Match when={props.officialLoading && !officialCards().length}>
            <div class="ext-grid" aria-busy="true">
              <For each={[0, 1]}>{() => <div class="ext-card sk-card" />}</For>
            </div>
          </Match>
          <Match when={!officialShown().length}>
            <p class="empty">
              {query().trim() || kind() !== "all" ? "No B-Anime extensions match." : "B-Anime hasn't published extensions yet."}
            </p>
          </Match>
          <Match when={officialShown().length}>
            <ul class="ext-grid">
              <For each={officialShown()}>{(c) => <ExtCard card={c} {...cardProps()} />}</For>
            </ul>
          </Match>
        </Switch>
      </section>

      <section class="ext-panel" aria-labelledby="yours-h">
        <div class="ext-panel-head">
          <h2 id="yours-h" class="subhead subhead-plain">From your repositories</h2>
          <p class="setting-desc">Made by other people. Install only what you trust.</p>
        </div>

      <For each={failed()}>
        {(r) => <p class="inline-error">Couldn't read {r.url}: {r.error}</p>}
      </For>

      <Switch>
        <Match when={props.loading && !cards().length}>
          <div class="ext-grid" aria-busy="true">
            <For each={[0, 1, 2]}>{() => <div class="ext-card sk-card" />}</For>
          </div>
        </Match>
        <Match when={!props.repos.length}>
          <div class="empty-block">
            <p>You haven't added any repositories. Add one to see the extensions it offers.</p>
            <button type="button" class="btn" onClick={props.onAddRepos}>Add a repository</button>
          </div>
        </Match>
        <Match when={!shown().length}>
          <p class="empty">{query().trim() ? `No extensions match “${query().trim()}”.` : "These repositories list no extensions of this type."}</p>
        </Match>
        <Match when={shown().length}>
          <ul class="ext-grid">
            <For each={shown()}>{(c) => <ExtCard card={c} {...cardProps()} />}</For>
          </ul>
        </Match>
      </Switch>
      </section>
    </>
  );
}

function ExtCard(props: {
  card: RepoCard;
  installed: InstalledExtension[];
  installing: string | null;
  onInstall: (manifestUrl: string, name: string) => Promise<boolean>;
  onUpdate: (id: string) => Promise<void>;
}) {
  const c = props.card;
  const mine = () => props.installed.find((x) => x.id === c.id);
  const canUpdate = () => !!mine() && !!c.version && newer(c.version, mine()!.version);
  const busy = () => props.installing === c.manifestUrl || props.installing === c.id;
  return (
                  <li class="ext-card">
                    <div class="ext-card-head">
                      <Show
                        when={c.icon}
                        fallback={<span class="ext-icon ext-icon-empty" aria-hidden="true">{c.name.slice(0, 1)}</span>}
                      >
                        {(src) => <img class="ext-icon" src={src()} alt="" width="40" height="40" loading="lazy" />}
                      </Show>
                      <div class="setting-text">
                        <p class="setting-title">
                          {c.name} <Show when={c.version}><span class="ext-version num">{c.version}</span></Show>
                        </p>
                        <p class="ext-meta">
                          {[
                            c.kind ? KIND_LABEL[c.kind] : null,
                            c.language?.toUpperCase(),
                            c.author ? `by ${c.author}` : null,
                            c.origin === "converted" ? "Converted on install" : null,
                          ]
                            .filter(Boolean)
                            .join(" · ")}
                        </p>
                      </div>
                    </div>
                    <p class="ext-card-desc">{c.description || "No description."}</p>
                    <div class="ext-card-foot">
                      <span class="ext-card-repo" title={c.manifestUrl}>{c.repoName}</span>
                      <Switch>
                        <Match when={canUpdate()}>
                          <button type="button" class="btn btn-primary btn-sm" disabled={busy()} onClick={() => void props.onUpdate(c.id!)}>
                            {busy() ? "Updating…" : `Update to ${c.version}`}
                          </button>
                        </Match>
                        <Match when={mine()}>
                          <span class="slot-stamp">Installed</span>
                        </Match>
                        <Match when={true}>
                          <button
                            type="button"
                            class="btn btn-primary btn-sm"
                            disabled={!!props.installing}
                            onClick={() => void props.onInstall(c.manifestUrl, c.name)}
                          >
                            <Plus size={14} stroke-width={2.25} aria-hidden="true" /> {busy() ? "Installing…" : "Install"}
                          </button>
                        </Match>
                      </Switch>
                    </div>
                  </li>
  );
}

// ── Repositories ───────────────────────────────────────────────────────────────

function Repositories(props: {
  official: { url: string; repo: Repo | null; error: string | null } | null;
  urls: string[];
  repos: { url: string; repo: Repo | null; error: string | null }[];
  onSave: (next: string[]) => void;
  onReload: () => void;
}) {
  const [draft, setDraft] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [checking, setChecking] = createSignal(false);

  const add = async (e: Event) => {
    e.preventDefault();
    const u = draft().trim();
    if (!u) return;
    if (u === props.official?.url) return setError("That's B-Anime's own repository, which is always included.");
    if (props.urls.includes(u)) return setError("That repository is already added.");
    setChecking(true);
    setError(null);
    try {
      // Read it first, so a typo doesn't get saved.
      await readExtensionRepo(u);
      props.onSave([...props.urls, u]);
      setDraft("");
    } catch (err) {
      setError(errorText(err));
    } finally {
      setChecking(false);
    }
  };
  const info = (url: string) => props.repos.find((r) => r.url === url);

  return (
    <section aria-label="Repositories">
      <p class="setting-desc repo-intro">
        A repository is a list of extensions someone publishes, shared as one link. Extensions from every repository
        here appear under Browse.
      </p>
      <form class="install-row" onSubmit={add}>
        <label class="search search-grow">
          <span class="sr-only">Repository URL</span>
          <input type="url" placeholder="https://…/repository.json" value={draft()} onInput={(e) => setDraft(e.currentTarget.value)} spellcheck={false} />
        </label>
        <button type="submit" class="btn btn-primary" disabled={!draft().trim() || checking()}>
          {checking() ? "Checking…" : "Add repository"}
        </button>
      </form>
      <Show when={error()}>
        <p class="inline-error" role="alert">{error()}</p>
      </Show>

      <ul class="repo-rows">
        <li class="repo-row">
          <div class="setting-text">
            <p class="setting-title">B-Anime</p>
            <p class="setting-desc">
              <Show when={props.official?.repo} fallback={props.official?.error ? `Couldn't read it: ${props.official.error}` : "Loading…"}>
                {(repo) => (
                  <>
                    <span class="num">{repo().extensions.length}</span> extension{repo().extensions.length === 1 ? "" : "s"} · made
                    by the B-Anime project.
                  </>
                )}
              </Show>
            </p>
          </div>
          <div class="setting-actions">
            <span class="slot-stamp">Built in</span>
          </div>
        </li>
      </ul>

      <Show when={props.urls.length} fallback={<p class="empty">You haven't added any repositories. Add one above.</p>}>
        <ul class="repo-rows">
          <For each={props.urls}>
            {(url) => {
              const r = () => info(url);
              return (
                <li class="repo-row">
                  <div class="setting-text">
                    <p class="setting-title">{r()?.repo?.name ?? new URL(url).host}</p>
                    <p class="repo-url" data-selectable>{url}</p>
                    <Show
                      when={r()?.error}
                      fallback={
                        <p class="setting-desc">
                          <Show when={r()?.repo} fallback="Loading…">
                            {(repo) => (
                              <>
                                <span class="num">{repo().extensions.length}</span> extension{repo().extensions.length === 1 ? "" : "s"}
                                <Show when={repo().description}> · {repo().description}</Show>
                              </>
                            )}
                          </Show>
                        </p>
                      }
                    >
                      <p class="inline-error">Couldn't read it: {r()!.error}</p>
                    </Show>
                  </div>
                  <div class="setting-actions">
                    <Show when={r()?.error}>
                      <button type="button" class="btn btn-sm" onClick={props.onReload}>Try again</button>
                    </Show>
                    <button type="button" class="btn btn-quiet btn-sm" onClick={() => props.onSave(props.urls.filter((x) => x !== url))}>
                      Remove
                    </button>
                  </div>
                </li>
              );
            }}
          </For>
        </ul>
      </Show>
    </section>
  );
}

// ── Install from URL (for extensions not in any repository) ───────────────────

function InstallFromUrl(props: { installedIds: () => string[]; onInstall: (url: string, name?: string) => Promise<boolean> }) {
  const [open, setOpen] = createSignal(false);
  const [url, setUrl] = createSignal("");
  const [checking, setChecking] = createSignal(false);
  const [preview, setPreview] = createSignal<ExtManifest | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [installing, setInstalling] = createSignal(false);

  const check = async (e: Event) => {
    e.preventDefault();
    if (!url().trim()) return;
    setChecking(true);
    setError(null);
    setPreview(null);
    try {
      setPreview(await previewExtension(url().trim()));
    } catch (err) {
      setError(errorText(err));
    } finally {
      setChecking(false);
    }
  };
  const install = async () => {
    setInstalling(true);
    if (await props.onInstall(url().trim(), preview()?.name)) {
      setPreview(null);
      setUrl("");
      setOpen(false);
    }
    setInstalling(false);
  };

  return (
    <div class="install-url">
      <Show
        when={open()}
        fallback={
          <button type="button" class="link-btn" onClick={() => setOpen(true)}>
            Install from a link instead
          </button>
        }
      >
        <h3 class="subhead subhead-plain">Install from a link</h3>
        <form class="install-row" onSubmit={check}>
          <label class="search search-grow">
            <span class="sr-only">Extension manifest URL</span>
            <input
              type="url"
              placeholder="https://…/manifest.json"
              value={url()}
              onInput={(e) => {
                setUrl(e.currentTarget.value);
                setPreview(null);
                setError(null);
              }}
              spellcheck={false}
            />
          </label>
          <button type="submit" class="btn" disabled={checking() || !url().trim()}>{checking() ? "Checking…" : "Check"}</button>
        </form>
        <Show when={error()}>
          <p class="inline-error" role="alert">{error()}</p>
        </Show>
        <Show when={preview()}>
          {(m) => (
            <div class="ext-preview">
              <ExtIdentity ext={m()} />
              <p class="setting-desc">{m().description || "No description."}</p>
              <div class="setting-actions">
                <button type="button" class="btn btn-primary" disabled={installing()} onClick={() => void install()}>
                  {installing() ? "Installing…" : props.installedIds().includes(m().id) ? `Reinstall ${m().version}` : `Install ${m().name}`}
                </button>
                <button type="button" class="btn btn-quiet" onClick={() => setPreview(null)}>Cancel</button>
              </div>
            </div>
          )}
        </Show>
      </Show>
    </div>
  );
}

// ── Install from a file (development builds only) ─────────────────────────────

function InstallFromFile(props: { onInstalled: (ext: InstalledExtension) => Promise<void> }) {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const pick = async () => {
    setBusy(true);
    setError(null);
    try {
      const ext = await installLocalExtension();
      if (ext) await props.onInstalled(ext);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div class="install-url">
      <button type="button" class="link-btn" disabled={busy()} onClick={() => void pick()}>
        {busy() ? "Installing…" : "Install from a file (development build)"}
      </button>
      <p class="setting-desc">
        Pick a manifest <code>.json</code>; its code is read from the same folder. Use <strong>Reload</strong> after
        editing the files.
      </p>
      <Show when={error()}>
        <p class="inline-error" role="alert">{error()}</p>
      </Show>
    </div>
  );
}

function ExtIdentity(props: { ext: ExtManifest }) {
  return (
    <div class="ext-identity">
      <Show
        when={props.ext.icon}
        fallback={<span class="ext-icon ext-icon-empty" aria-hidden="true">{props.ext.name.slice(0, 1)}</span>}
      >
        {(src) => <img class="ext-icon" src={src()} alt="" width="40" height="40" />}
      </Show>
      <div class="setting-text">
        <p class="setting-title">
          {props.ext.name} <span class="ext-version num">{props.ext.version}</span>
        </p>
        <p class="ext-meta">
          {KIND_LABEL[props.ext.kind]}
          {props.ext.language ? ` · ${props.ext.language.toUpperCase()}` : ""}
          {props.ext.author ? ` · by ${props.ext.author}` : ""}
          {props.ext.origin === "converted" ? " · Converted extension" : ""}
          {"sourceUrl" in props.ext && isLocalExtension(props.ext as InstalledExtension) ? " · Local file" : ""}
        </p>
      </div>
    </div>
  );
}

function ExtensionRow(props: {
  ext: InstalledExtension;
  update: string | undefined;
  onChanged: () => Promise<void>;
  onToggle: (enabled: boolean) => void;
}) {
  const { notify } = homeStore;
  const [busy, setBusy] = createSignal<null | "update" | "remove">(null);
  const [confirmRemove, setConfirmRemove] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [trying, setTrying] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [running, setRunning] = createSignal(false);
  const [result, setResult] = createSignal<ExtCallOutput | null>(null);
  const [tryError, setTryError] = createSignal<string | null>(null);

  const toggle = async (enabled: boolean) => {
    props.onToggle(enabled);
    try {
      await setExtensionEnabled(props.ext.id, enabled);
    } catch (e) {
      props.onToggle(!enabled);
      setError(errorText(e));
    }
  };
  const update = async () => {
    setBusy("update");
    setError(null);
    try {
      const ext = await updateExtension(props.ext.id);
      notify(`Updated ${ext.name} to ${ext.version}.`);
      await props.onChanged();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  };
  const reload = async () => {
    setBusy("update");
    setError(null);
    try {
      const ext = await updateExtension(props.ext.id);
      notify(`Reloaded ${ext.name} ${ext.version} from disk.`);
      await props.onChanged();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  };
  const remove = async () => {
    if (!confirmRemove()) {
      setConfirmRemove(true);
      return;
    }
    setBusy("remove");
    try {
      await removeExtension(props.ext.id);
      notify(`Removed ${props.ext.name}.`);
      await props.onChanged();
    } catch (e) {
      setError(errorText(e));
      setBusy(null);
    }
  };
  const run = async (e: Event) => {
    e.preventDefault();
    setRunning(true);
    setTryError(null);
    setResult(null);
    try {
      setResult(await tryExtension(props.ext.id, query().trim()));
    } catch (err) {
      setTryError(errorText(err));
    } finally {
      setRunning(false);
    }
  };
  const titles = () => {
    const v = result()?.value;
    return Array.isArray(v) ? v.map((x) => (x && typeof x === "object" && "title" in x ? String((x as { title: unknown }).title) : JSON.stringify(x))) : null;
  };

  return (
    <li class="ext-row" data-disabled={props.ext.enabled ? undefined : ""}>
      <div class="ext-main">
        <ExtIdentity ext={props.ext} />
        <p class="setting-desc">{props.ext.description}</p>
        <Show when={error()}>
          <p class="inline-error" role="alert">{error()}</p>
        </Show>
      </div>
      <div class="ext-actions">
        <Show when={isLocalExtension(props.ext)}>
          <button type="button" class="btn btn-sm" disabled={busy() !== null} onClick={reload} title="Read the manifest and code from disk again">
            {busy() === "update" ? "Reloading…" : "Reload"}
          </button>
        </Show>
        <Show when={props.update}>
          <button type="button" class="btn btn-primary btn-sm" disabled={busy() !== null} onClick={update}>
            {busy() === "update" ? "Updating…" : `Update to ${props.update}`}
          </button>
        </Show>
        <button type="button" class="btn btn-sm" aria-expanded={trying()} disabled={!props.ext.enabled} onClick={() => setTrying(!trying())}>
          Try it
        </button>
        <button
          type="button"
          class="btn btn-sm"
          classList={{ "btn-quiet": !confirmRemove(), "btn-danger": confirmRemove() }}
          disabled={busy() !== null}
          onClick={remove}
          onBlur={() => setConfirmRemove(false)}
        >
          {busy() === "remove" ? "Removing…" : confirmRemove() ? "Confirm remove" : "Remove"}
        </button>
        <label class="switch" title={props.ext.enabled ? "On" : "Off"}>
          <input
            type="checkbox"
            role="switch"
            checked={props.ext.enabled}
            onChange={(e) => void toggle(e.currentTarget.checked)}
            aria-label={`${props.ext.name} enabled`}
          />
          <span aria-hidden="true" />
        </label>
      </div>

      <Show when={trying() && props.ext.enabled}>
        <div class="ext-try">
          <form class="install-row" onSubmit={run}>
            <label class="search search-grow">
              <span class="sr-only">Search with {props.ext.name}</span>
              <input
                type="search"
                placeholder={`Search ${props.ext.name}, e.g. a show title`}
                value={query()}
                onInput={(e) => setQuery(e.currentTarget.value)}
              />
            </label>
            <button type="submit" class="btn" disabled={running()}>{running() ? "Running…" : "Search"}</button>
          </form>
          <Show when={tryError()}>
            <p class="inline-error">{tryError()}</p>
          </Show>
          <Show when={result()}>
            {(r) => (
              <div class="try-result">
                <Show
                  when={titles()}
                  fallback={<pre class="try-pre" data-selectable>{JSON.stringify(r().value, null, 2)}</pre>}
                >
                  {(list) => (
                    <>
                      <p class="try-count num">{list().length} {list().length === 1 ? "result" : "results"}</p>
                      <ol class="try-list" data-selectable>
                        <For each={list().slice(0, 20)}>{(t) => <li>{t}</li>}</For>
                      </ol>
                    </>
                  )}
                </Show>
                <Show when={r().logs.length}>
                  <details class="try-logs">
                    <summary>Console ({r().logs.length})</summary>
                    <pre class="try-pre" data-selectable>{r().logs.join("\n")}</pre>
                  </details>
                </Show>
              </div>
            )}
          </Show>
        </div>
      </Show>
    </li>
  );
}

