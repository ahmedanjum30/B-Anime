import { For, Match, Show, Switch, createResource, createSignal } from "solid-js";

import {
  checkExtensionUpdates, errorText, getExtensionRepos, inTauri, installExtension, listExtensions, previewExtension,
  readExtensionRepo, removeExtension, setExtensionEnabled, setExtensionRepos, tryExtension, updateExtension,
} from "../lib/api";
import { homeStore } from "../lib/store";
import type { ExtCallOutput, ExtManifest, InstalledExtension } from "../lib/types";

const KIND_LABEL = { anime: "Anime", manga: "Manga" } as const;

export default function Extensions() {
  const { notify } = homeStore;
  const [installed, { refetch, mutate }] = createResource(listExtensions);
  const [updates, { refetch: recheck }] = createResource(checkExtensionUpdates);
  const updateFor = (id: string) => updates()?.find((u) => u.id === id)?.version;

  // ── Install from URL: check first, then confirm.
  const [url, setUrl] = createSignal("");
  const [checking, setChecking] = createSignal(false);
  const [preview, setPreview] = createSignal<ExtManifest | null>(null);
  const [installError, setInstallError] = createSignal<string | null>(null);
  const [installing, setInstalling] = createSignal(false);

  const check = async (e: Event) => {
    e.preventDefault();
    if (!url().trim()) return;
    setChecking(true);
    setInstallError(null);
    setPreview(null);
    try {
      setPreview(await previewExtension(url().trim()));
    } catch (err) {
      setInstallError(errorText(err));
    } finally {
      setChecking(false);
    }
  };
  const install = async (from: string) => {
    setInstalling(true);
    setInstallError(null);
    try {
      const ext = await installExtension(from);
      notify(`Installed ${ext.name}.`);
      setPreview(null);
      setUrl("");
      await refetch();
    } catch (err) {
      setInstallError(errorText(err));
    } finally {
      setInstalling(false);
    }
  };
  const alreadyInstalled = (id: string | null | undefined) => !!id && !!installed()?.some((x) => x.id === id);

  return (
    <div class="page">
      <h1 class="page-title">Extensions</h1>
      <p class="page-lede">
        Extensions add sources for watching and reading online. B-Anime comes with none. Install only extensions
        you trust: they can reach the sites they're written for, but never your files.
      </p>

      <Show when={inTauri} fallback={<p class="empty">Open the desktop app to manage extensions.</p>}>
        <section class="section" aria-labelledby="install-h">
          <h2 id="install-h" class="section-head">Install from URL</h2>
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
                  setInstallError(null);
                }}
                spellcheck={false}
              />
            </label>
            <button type="submit" class="btn" disabled={checking() || !url().trim()}>
              {checking() ? "Checking…" : "Check"}
            </button>
          </form>
          <Show when={installError()}>
            <p class="inline-error" role="alert">{installError()}</p>
          </Show>
          <Show when={preview()}>
            {(m) => (
              <div class="ext-preview">
                <ExtIdentity ext={m()} />
                <p class="setting-desc">{m().description || "No description."}</p>
                <div class="setting-actions">
                  <button type="button" class="btn btn-primary" disabled={installing()} onClick={() => void install(url().trim())}>
                    {installing() ? "Installing…" : alreadyInstalled(m().id) ? `Reinstall ${m().version}` : `Install ${m().name}`}
                  </button>
                  <button type="button" class="btn btn-quiet" onClick={() => setPreview(null)}>Cancel</button>
                </div>
              </div>
            )}
          </Show>
        </section>

        <section class="section" aria-labelledby="installed-h">
          <h2 id="installed-h" class="section-head">
            Installed
            <small class="num">{installed()?.length ?? 0}</small>
          </h2>
          <Switch>
            <Match when={installed.error}>
              <p class="inline-error">Couldn't list extensions: {errorText(installed.error)}</p>
            </Match>
            <Match when={installed() && installed()!.length === 0}>
              <p class="empty">No extensions yet. Paste a manifest URL above, or add a repository below.</p>
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
                      onToggle={(enabled) =>
                        mutate((list) => list?.map((x) => (x.id === ext.id ? { ...x, enabled } : x)))
                      }
                    />
                  )}
                </For>
              </ul>
            </Match>
          </Switch>
        </section>

        <Repositories installedIds={() => installed()?.map((x) => x.id) ?? []} onInstall={install} installing={installing()} />
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

function Repositories(props: { installedIds: () => string[]; onInstall: (url: string) => Promise<void>; installing: boolean }) {
  const [repos, { mutate }] = createResource(getExtensionRepos);
  const [draft, setDraft] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);

  const save = async (next: string[]) => {
    const prev = repos() ?? [];
    mutate(next);
    try {
      await setExtensionRepos(next);
    } catch (e) {
      mutate(prev);
      setError(errorText(e));
    }
  };
  const add = (e: Event) => {
    e.preventDefault();
    const u = draft().trim();
    if (!u || repos()?.includes(u)) return;
    setError(null);
    setDraft("");
    void save([...(repos() ?? []), u]);
  };

  return (
    <section class="section" aria-labelledby="repos-h">
      <h2 id="repos-h" class="section-head">
        Repositories
        <small>Lists of extensions published by others. None are added by default.</small>
      </h2>
      <form class="install-row" onSubmit={add}>
        <label class="search search-grow">
          <span class="sr-only">Repository URL</span>
          <input type="url" placeholder="https://…/repository.json" value={draft()} onInput={(e) => setDraft(e.currentTarget.value)} spellcheck={false} />
        </label>
        <button type="submit" class="btn" disabled={!draft().trim()}>Add repository</button>
      </form>
      <Show when={error()}>
        <p class="inline-error">{error()}</p>
      </Show>
      <For each={repos() ?? []}>
        {(repo) => <RepoListing url={repo} onRemove={() => void save((repos() ?? []).filter((r) => r !== repo))} {...props} />}
      </For>
    </section>
  );
}

function RepoListing(props: {
  url: string;
  onRemove: () => void;
  installedIds: () => string[];
  onInstall: (url: string) => Promise<void>;
  installing: boolean;
}) {
  const [entries, { refetch }] = createResource(() => props.url, readExtensionRepo);
  return (
    <div class="repo">
      <div class="repo-head">
        <p class="repo-url" data-selectable>{props.url}</p>
        <button type="button" class="btn btn-quiet btn-sm" onClick={props.onRemove}>Remove</button>
      </div>
      <Switch>
        <Match when={entries.error}>
          <div class="empty-block">
            <p class="inline-error">Couldn't read this repository: {errorText(entries.error)}</p>
            <button type="button" class="btn btn-sm" onClick={() => void refetch()}>Try again</button>
          </div>
        </Match>
        <Match when={entries.loading}>
          <p class="empty">Loading…</p>
        </Match>
        <Match when={entries()?.length === 0}>
          <p class="empty">This repository lists no extensions.</p>
        </Match>
        <Match when={entries()}>
          <ul class="repo-list">
            <For each={entries()}>
              {(e) => {
                const isInstalled = () => !!e.id && props.installedIds().includes(e.id);
                return (
                  <li class="repo-item">
                    <div class="setting-text">
                      <p class="setting-title">
                        {e.name} <Show when={e.version}><span class="ext-version num">{e.version}</span></Show>
                      </p>
                      <p class="ext-meta">
                        {[e.kind ? KIND_LABEL[e.kind] : null, e.author ? `by ${e.author}` : null].filter(Boolean).join(" · ")}
                      </p>
                      <Show when={e.description}><p class="setting-desc">{e.description}</p></Show>
                    </div>
                    <button
                      type="button"
                      class="btn btn-sm"
                      disabled={isInstalled() || props.installing}
                      onClick={() => void props.onInstall(e.manifestUrl)}
                    >
                      {isInstalled() ? "Installed" : "Install"}
                    </button>
                  </li>
                );
              }}
            </For>
          </ul>
        </Match>
      </Switch>
    </div>
  );
}
