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

// The audio track picked in the player (a stream or file with several audio languages), per
// show, by its language (or name when it has none): track numbers differ between servers and
// episodes. Shows never chosen use the general audio preference.
const audioKey = (mediaId: number) => `banime.audio.${mediaId}`;
/** What identifies an audio track across episodes and servers. */
export const audioId = (t: { name: string; lang: string }) => (t.lang || t.name).trim().toLowerCase();

export function recallAudio(mediaId: number): string | null {
  try {
    return localStorage.getItem(audioKey(mediaId));
  } catch {
    return null;
  }
}

export function rememberAudio(mediaId: number, id: string) {
  try {
    localStorage.setItem(audioKey(mediaId), id);
  } catch {
    /* storage unavailable: just don't remember */
  }
}
