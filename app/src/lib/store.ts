import { createResource, createRoot, createSignal } from "solid-js";
import { getHome } from "./api";

// App-wide state. One resource for Home data; the rail reads counts and
// sync evidence from it too, so there's a single fetch.
export const homeStore = createRoot(() => {
  const [home, { refetch }] = createResource(getHome);
  /** Show under the pointer/focus; links Up next cards and listings slots. */
  const [hoverMedia, setHoverMedia] = createSignal<number | null>(null);
  const [toast, setToast] = createSignal<string | null>(null);
  let toastTimer: number | undefined;
  const notify = (msg: string) => {
    setToast(msg);
    clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => setToast(null), 3200);
  };
  return { home, refetch, hoverMedia, setHoverMedia, toast, notify };
});
