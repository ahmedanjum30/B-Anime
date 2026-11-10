import { createResource, createRoot, createSignal } from "solid-js";
import { getAccount, getHome, onEvent, onLibraryChanged, syncIfStale, updateStatus } from "./api";
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
  let toastTimer: number | undefined;
  const notify = (msg: string) => {
    setToast(msg);
    clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => setToast(null), 3600);
  };

  /** Library scan progress, pushed by the backend. */
  const [scan, setScan] = createSignal<ScanStatus>({ running: false, phase: null, done: 0, total: 0 });
  void onEvent<ScanStatus>("scan-progress", setScan);
  void onEvent("update-ready", () => void refetchUpdate());
  void onEvent<{ episode: number; title: string }>("player-watched", (w) =>
    notify(`Episode ${w.episode} of ${w.title} marked watched on AniList.`),
  );

  return {
    home, refetch, account, refetchAccount, setAccount, update, refetchUpdate, scan, now,
    hoverMedia, setHoverMedia, toast, notify,
  };
});
