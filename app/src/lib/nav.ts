import { createEffect, on } from "solid-js";
import { useLocation, type Navigator } from "@solidjs/router";

// The page shown before the current one (path only), so a Back button can step back through
// history instead of pushing a new entry. Pushing one made Back loop: the player's Back opened
// the show page anew, and the show page's Back (history back) returned to the player.
let previous: string | null = null;

/** Records page changes; call once inside the router (the Shell). */
export function trackPages() {
  const location = useLocation();
  createEffect(
    on(
      () => location.pathname,
      (_now, before) => {
        if (before !== undefined) previous = before;
      },
    ),
  );
}

/** Leaves for `path`: back through history when that's where we came from, else replacing this page. */
export function leaveTo(navigate: Navigator, path: string) {
  if (previous === path && history.length > 1) history.back();
  else navigate(path, { replace: true });
}
