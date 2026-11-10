import type { HomeData } from "./types";

/** True inside the Tauri webview; false when the UI runs in a plain browser (vite dev). */
export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function getHome(): Promise<HomeData> {
  if (!inTauri) {
    // Browser preview only: the real data comes from Rust.
    const { browserPreviewHome } = await import("./preview");
    return browserPreviewHome();
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<HomeData>("get_home");
}
