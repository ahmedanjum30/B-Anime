import { createResource, createRoot, createSignal } from "solid-js";
import { getAccount, getHome, googleSync, inTauri, onEvent, syncsRunning, onLibraryChanged, syncIfStale, updateStatus, installUpdate, errorText, UPDATE_ACTION } from "./api";
import { fmtVersion } from "./format";
import { keepSame } from "./keep";
import type { ScanStatus } from "./types";

/** What a running sync is with. */
export type SyncWith = "anilist" | "google";

// App-wide state. Home data and the AniList account are fetched once and refreshed whenever
// the backend says lists or library changed (sign-in, sync, sign-out, a scan).
export const homeStore = createRoot(() => {
  // Home and the account load once the "library changed" listener is attached: a sync that
  // finished while the UI was still loading emitted its event before anyone was listening, so
  // loading earlier meant loading twice at every launch (and Home drawn twice).
  const [listening, setListening] = createSignal(false);
  const [home, { refetch }] = createResource(listening, getHome, { storage: keepSame });
  const [account, { refetch: refetchAccount, mutate: setAccount }] = createResource(listening, getAccount);
  const [update, { refetch: refetchUpdate }] = createResource(updateStatus);

  const refresh = () => {
    if (!listening()) return;
    void refetch();
    void refetchAccount();
  };
  void onLibraryChanged(refresh).then(
    () => setListening(true),
    () => setListening(true),
  );

  // Automatic sync triggers (launch and a 15-minute timer run in the backend).
  // Coming back to the app syncs if the last sync is over 5 minutes old.
  // Your place in each show (Google): coming back fetches what another device saved meanwhile,
  // and leaving (the phone going to the background) sends this one's while it still can.
  let placesAt = 0;
  const syncPlaces = () => {
    if (!inTauri || Date.now() - placesAt < 30_000) return;
    placesAt = Date.now();
    void googleSync().catch(() => {});
  };
  const onReturn = () => {
    if (document.visibilityState === "visible") void syncIfStale(5 * 60);
    syncPlaces();
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
  /** The notice is about the episode playing (the player, the room): it sits on the video, not
   *  on the screen's centre. */
  const [toastOnVideo, setToastOnVideo] = createSignal(false);
  let toastTimer: number | undefined;
  const show = (msg: string, action: { label: string; run: () => void } | undefined, onVideo: boolean) => {
    setToastOnVideo(onVideo);
    setToast(msg);
    setToastAction(action ?? null);
    clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => {
      setToast(null);
      setToastAction(null);
    }, action ? 12000 : 3600);
  };
  /** A notice for the whole app (an update, an extension, the library): the screen's centre. */
  const notify = (msg: string, action?: { label: string; run: () => void }) => show(msg, action, false);
  /** A notice about what's playing: centred on the video while it shows, else as any notice. */
  const notifyPlayback = (msg: string, action?: { label: string; run: () => void }) => show(msg, action, true);

  /** Syncs running now (AniList, Google), said at the top of the window. Only one that takes a
   *  moment shows: a quick one (most of them) finishes before it would appear, so the note
   *  doesn't flicker on every background sync. */
  const SYNC_SHOW_AFTER = 400;
  const running = new Set<SyncWith>();
  const [syncing, setSyncing] = createSignal<SyncWith[]>([]);
  let syncShowTimer: number | undefined;
  const syncsChanged = () => {
    clearTimeout(syncShowTimer);
    if (!running.size) return setSyncing([]);
    // Already showing: the change shows at once; otherwise after the pause.
    if (syncing().length) setSyncing([...running]);
    else syncShowTimer = window.setTimeout(() => setSyncing([...running]), SYNC_SHOW_AFTER);
  };
  void onEvent<{ with: SyncWith; busy: boolean }>("syncing", (s) => {
    if (s.busy) running.add(s.with);
    else running.delete(s.with);
    syncsChanged();
  }).then(() =>
    // The launch syncs start before this page has loaded: the ones already running.
    syncsRunning().then((now) => {
      for (const w of now) running.add(w);
      if (now.length) syncsChanged();
    }, () => {}),
  );

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
    hoverMedia, setHoverMedia, toast, toastAction, toastOnVideo, notify, notifyPlayback, syncing,
  };
});
