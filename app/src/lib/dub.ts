// Sub or Dub, remembered per show: what was last watched or picked for it is where it starts
// next time (Home's Up Next, the show page, the player). Shows never chosen use the general
// audio preference.

const key = (mediaId: number) => `banime.dub.${mediaId}`;

/** The remembered choice for this show, or null when there's none. */
export function recallDub(mediaId: number): boolean | null {
  try {
    const v = localStorage.getItem(key(mediaId));
    return v === null ? null : v === "1";
  } catch {
    return null;
  }
}

export function rememberDub(mediaId: number, dub: boolean) {
  try {
    localStorage.setItem(key(mediaId), dub ? "1" : "0");
  } catch {
    /* storage unavailable: just don't remember */
  }
}
