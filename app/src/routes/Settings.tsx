import { Match, Show, Switch, createSignal } from "solid-js";

import { cancelLogin, errorText, inTauri, login, logout } from "../lib/api";
import { fmtAgo } from "../lib/format";
import { homeStore } from "../lib/store";

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

      <section class="section" aria-labelledby="lib-h">
        <h2 id="lib-h" class="section-head">
          Library folders
          <small>Where B-Anime looks for your episodes.</small>
        </h2>
        <p class="empty">Folder scanning is coming in a later update.</p>
      </section>
    </div>
  );
}
