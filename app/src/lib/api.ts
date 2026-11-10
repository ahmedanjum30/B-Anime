import type { Account, HomeData, SyncReport } from "./types";

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

export async function getAccount(): Promise<Account> {
  if (!inTauri) return { clientConfigured: true, viewer: null, lastSynced: null };
  return call<Account>("get_account");
}

/** Opens AniList in the browser; resolves once the user approves (or rejects on cancel/timeout). */
export const login = () => call<Account>("anilist_login");
export const cancelLogin = () => call<void>("anilist_cancel_login");
export const logout = () => call<Account>("anilist_logout");
export const syncNow = () => call<SyncReport>("anilist_sync");

/** Runs `fn` whenever the backend reports changed lists or library. Returns an unsubscribe. */
export async function onLibraryChanged(fn: () => void): Promise<() => void> {
  if (!inTauri) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen("library-changed", fn);
}

/** Tauri rejects with the Rust error string; normalise for display. */
export const errorText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
