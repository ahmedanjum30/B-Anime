import { errorText, playLocal } from "./api";

/**
 * Plays an episode from the library with the user's preferred player: the in-app player opens
 * as a route; MPV and VLC launch outside (MPV reports progress back).
 */
export async function playLibraryEpisode(
  mediaId: number,
  episode: number,
  navigate: (to: string) => void,
  notify: (msg: string) => void,
) {
  try {
    const { mode } = await playLocal(mediaId, episode);
    if (mode === "builtin") navigate(`/watch/${mediaId}/${episode}?local=1`);
    else if (mode === "mpv") notify("Playing in MPV. Progress syncs to AniList when you're 80% through.");
    else notify("Playing in VLC. Progress isn't tracked there; mark it watched here when you're done.");
  } catch (e) {
    notify(errorText(e));
  }
}
