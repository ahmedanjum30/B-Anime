import { createResource, createRoot, createSignal } from "solid-js";
import { getAccount, getHome, onEvent, onLibraryChanged, syncIfStale, updateStatus, installUpdate, errorText, UPDATE_ACTION } from "./api";
import { fmtVersion } from "./format";
import type { ScanStatus } from "./types";

// App-wide state. Home data and the AniList account are fetched once and refreshed whenever
// the backend says lists or library changed (sign-in, sync, sign-out, a scan).
export const homeStore = createRoot(() => {
  const [home, { refetch }] = createResource(getHome);
  const [account, { refetch: refetchAccount, mutate: setAccount }] = createResource(getAccount);
  const [update, { refetch: refetchUpdate }] = createResource(updateStatus);

  const refresh = () => {
    void refetch();
    void refetchAccount();
  };
  // Refetch once the listener is attached too: a sync that finished while the UI was still
  // loading emitted its event before anyone was listening.
  void onLibraryChanged(refresh).then(refresh);

  // Automatic sync triggers (launch and a 15-minute timer run in the backend).
  // Coming back to the app syncs if the last sync is over 5 minutes old.
  const onReturn = () => {
    if (document.visibilityState === "visible") void syncIfStale(5 * 60);
  };
  window.addEventListener("focus", onReturn);
  document.addEventListener("visibilitychange", onReturn);
  // Network back: sync now (the backend also retries a failing sync on every trigger).
  window.addEventListener("online", () => void syncIfStale(0));

  /** Ticks every 30 s so relative times ("2 minutes ago") stay true. */
  const [now, setNow] = createSignal(Date.now());
  setInterval(() => setNow(Date.now()), 30_000);

  /** Show under the pointer/focus; links Up next cards and listings slots. */
  const [hoverMedia, setHoverMedia] = createSignal<number | null>(null);
  const [toast, setToast] = createSignal<string | null>(null);
  /** A notice can offer one thing to do (clicking the notice does it); it stays up longer. */
  const [toastAction, setToastAction] = createSignal<{ label: string; run: () => void } | null>(null);
  let toastTimer: number | undefined;
  const notify = (msg: string, action?: { label: string; run: () => void }) => {
    setToast(msg);
    setToastAction(action ?? null);
    clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => {
      setToast(null);
      setToastAction(null);
    }, action ? 12000 : 3600);
  };

  /** Library scan progress, pushed by the backend. */
  const [scan, setScan] = createSignal<ScanStatus>({ running: false, phase: null, done: 0, total: 0 });
  void onEvent<ScanStatus>("scan-progress", setScan);
  // A new version: said once when it's found (it then downloads in the background), and again
  // when it's ready to install. The sidebar keeps the notice and the restart button.
  void onEvent<{ version: string }>("update-available", (u) => {
    void refetchUpdate();
    notify(`B-Anime ${fmtVersion(u.version)} is available. Downloading it in the background…`);
  });
  void onEvent("update-failed", () => void refetchUpdate());
  // Installed extensions updated themselves at launch.
  void onEvent<{ id: string; name: string; version: string }[]>("extensions-updated", (list) => {
    notify(
      list.length === 1
        ? `Extension updated: ${list[0].name} ${list[0].version}.`
        : `${list.length} extensions updated: ${list.map((e) => e.name).join(", ")}.`,
    );
  });
  void onEvent<{ version: string }>("update-ready", (u) => {
    void refetchUpdate();
    // Clicking the notice installs it (as the sidebar's and More's buttons do).
    notify(`B-Anime ${fmtVersion(u.version)} is ready.`, {
      label: UPDATE_ACTION,
      run: () => void installUpdate().catch((e) => notify(errorText(e))),
    });
  });
  void onEvent<{ episode: number; title: string }>("player-watched", (w) =>
    notify(`Episode ${w.episode} of ${w.title} marked watched on AniList.`),
  );

  return {
    home, refetch, account, refetchAccount, setAccount, update, refetchUpdate, scan, now,
    hoverMedia, setHoverMedia, toast, toastAction, notify,
  };
});
