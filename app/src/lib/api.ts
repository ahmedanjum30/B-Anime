import type {
  Account, BrowseFilter, BrowsePage, EntryChange, ExtCallOutput, ExtManifest, HomeData, InstalledExtension,
  LibraryEntry, LibraryKind, ListEntry, MediaPage, RepoEntry, ScheduleItem, ScheduleScope,
} from "./types";

// ── Extensions (desktop only; the browser preview shows an empty list) ──
export const listExtensions = () => (inTauri ? call<InstalledExtension[]>("list_extensions") : Promise.resolve([]));
export const previewExtension = (url: string) => call<ExtManifest>("preview_extension", { url });
export const installExtension = (url: string) => call<InstalledExtension>("install_extension", { url });
export const updateExtension = (id: string) => call<InstalledExtension>("update_extension", { id });
export const checkExtensionUpdates = () =>
  inTauri ? call<{ id: string; version: string }[]>("check_extension_updates") : Promise.resolve([]);
export const setExtensionEnabled = (id: string, enabled: boolean) => call<void>("set_extension_enabled", { id, enabled });
export const removeExtension = (id: string) => call<void>("remove_extension", { id });
export const tryExtension = (id: string, query: string) => call<ExtCallOutput>("try_extension", { id, query });
export const getExtensionRepos = () => (inTauri ? call<string[]>("get_extension_repos") : Promise.resolve([]));
export const setExtensionRepos = (repos: string[]) => call<void>("set_extension_repos", { repos });
export const readExtensionRepo = (url: string) => call<RepoEntry[]>("read_extension_repo", { url });

export async function browse(filter: BrowseFilter): Promise<BrowsePage> {
  if (!inTauri) {
    const { browserPreviewBrowse } = await import("./preview");
    return browserPreviewBrowse(filter);
  }
  return call<BrowsePage>("browse", { filter });
}

/** Airing between two unix timestamps (seconds). */
export async function getSchedule(scope: ScheduleScope, from: number, to: number): Promise<ScheduleItem[]> {
  if (!inTauri) {
    const { browserPreviewSchedule } = await import("./preview");
    return browserPreviewSchedule(scope, from, to);
  }
  return call<ScheduleItem[]>("get_schedule", { scope, from, to });
}

export async function getMedia(id: number): Promise<MediaPage> {
  if (!inTauri) {
    const { browserPreviewMedia } = await import("./preview");
    return browserPreviewMedia(id);
  }
  return call<MediaPage>("get_media", { id });
}

/** Opens the show's AniList page in the default browser. */
export const openAniList = (id: number, type: LibraryKind) => call<void>("open_anilist", { id, kind: type });

/** Saves to AniList; resolves with the entry as AniList now has it. */
export async function saveEntry(mediaId: number, change: EntryChange): Promise<ListEntry> {
  if (!inTauri) {
    await new Promise((r) => setTimeout(r, 300));
    return { status: change.status ?? "CURRENT", progress: change.progress ?? 0, score: change.score ?? null, updatedAt: new Date().toISOString() };
  }
  return call<ListEntry>("save_entry", { mediaId, change });
}

/** True inside the Tauri webview; false when the UI runs in a plain browser (vite dev). */
export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

export async function getHome(): Promise<HomeData> {
  if (!inTauri) {
    // Browser preview only: the real data comes from Rust.
    const { browserPreviewHome } = await import("./preview");
    return browserPreviewHome();
  }
  return call<HomeData>("get_home");
}

export async function getLibrary(kind: LibraryKind): Promise<LibraryEntry[]> {
  if (!inTauri) {
    const { browserPreviewLibrary } = await import("./preview");
    return browserPreviewLibrary(kind);
  }
  return call<LibraryEntry[]>("get_library", { kind });
}

export async function getAccount(): Promise<Account> {
  if (!inTauri) return { clientConfigured: true, viewer: { id: 0, name: "Preview", avatar: null }, lastSynced: null, syncError: null };
  return call<Account>("get_account");
}

/** Opens AniList in the browser; resolves once the user approves (or rejects on cancel/timeout). */
export const login = () => call<Account>("anilist_login");
export const cancelLogin = () => call<void>("anilist_cancel_login");
export const logout = () => call<Account>("anilist_logout");
/** Syncs if the last sync is older than `minAgeSecs` (and none is running). Never throws. */
export async function syncIfStale(minAgeSecs: number): Promise<boolean> {
  if (!inTauri) return false;
  return call<boolean>("sync_if_stale", { minAgeSecs }).catch(() => false);
}

/** Runs `fn` whenever the backend reports changed lists or library. Returns an unsubscribe. */
export async function onLibraryChanged(fn: () => void): Promise<() => void> {
  if (!inTauri) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen("library-changed", fn);
}

/** Tauri rejects with the Rust error string; normalise for display. */
export const errorText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
