import { createEffect, onCleanup } from "solid-js";
import { useLocation } from "@solidjs/router";
import { playerSlot, sameEpisode, targetFromPath } from "../lib/playerSlot";

/**
 * The /watch page: says which episode to play. The player itself lives in the Shell, so in a
 * Watch together room it can keep going after this page is left.
 */
export default function WatchPage() {
  const location = useLocation();
  createEffect(() => {
    const t = targetFromPath(location.pathname + location.search);
    // Already playing it (behind the app, say, from the library or a picked source): as it is.
    if (t && !sameEpisode(playerSlot.target(), t)) playerSlot.setTarget(t);
  });
  playerSlot.setOnPage(true);
  onCleanup(() => playerSlot.setOnPage(false));
  return null;
}
