import { createResource, createRoot, createSignal } from "solid-js";
import { getAccount, getHome, onLibraryChanged } from "./api";

// App-wide state. Home data and the AniList account are fetched once and refreshed whenever
// the backend says lists or library changed (sign-in, sync, sign-out).
export const homeStore = createRoot(() => {
  const [home, { refetch }] = createResource(getHome);
  const [account, { refetch: refetchAccount, mutate: setAccount }] = createResource(getAccount);

  void onLibraryChanged(() => {
    void refetch();
    void refetchAccount();
  });

  /** Show under the pointer/focus; links Up next cards and listings slots. */
  const [hoverMedia, setHoverMedia] = createSignal<number | null>(null);
  const [toast, setToast] = createSignal<string | null>(null);
  let toastTimer: number | undefined;
  const notify = (msg: string) => {
    setToast(msg);
    clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => setToast(null), 3600);
  };
  return { home, refetch, account, refetchAccount, setAccount, hoverMedia, setHoverMedia, toast, notify };
});
