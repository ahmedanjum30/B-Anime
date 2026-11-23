// Which of a stream's sources (servers) the player starts on, and which one an audio switch moves
// to. No Solid or storage here, so it can be checked on its own.

export type Pickable = { i: number; label: string; kind: string; audio: string | null };

/** What the player remembers a server by: its label (or its kind when it has none). */
export const serverName = (s: { label: string; kind: string }) => s.label.trim() || s.kind;

/** The source at the same place in `audio`'s list as `from` is in its own: the same server in the
 *  other language when an extension lists its servers once per language (HD-1 sub → HD-1 dub). */
export function sameServerIn<T extends Pickable>(all: T[], from: T, audio: string | null): T | undefined {
  const at = all.filter((s) => s.audio === from.audio).findIndex((s) => s.i === from.i);
  const to = all.filter((s) => s.audio === audio);
  return to[Math.max(0, Math.min(at, to.length - 1))];
}

/**
 * The source to start on.
 *
 * When some source is in `picked` (the Sub/Dub this episode was opened with: "ja" / "en"), it plays
 * in that language: the server last picked for this show (`remembered`, a label) if it's in it or
 * names no language, else that same server in `picked` (HD-1 sub → HD-1 dub), else the first
 * source in `picked`.
 *
 * Otherwise (sources that don't name their audio, or none in that language) it's what the player
 * always did: the remembered server, else the first in the `preferred` audio, else the first.
 */
export function startSource<T extends Pickable>(
  all: T[],
  opts: { picked: string | null; preferred: string | null | undefined; remembered: string | null },
): T | undefined {
  const { picked, preferred, remembered } = opts;
  const last = remembered ? all.find((x) => serverName(x) === remembered) : undefined;
  if (picked && all.some((x) => x.audio === picked)) {
    if (last && (!last.audio || last.audio === picked)) return last;
    return (last && sameServerIn(all, last, picked)) || all.find((x) => x.audio === picked);
  }
  return last ?? (preferred ? all.find((x) => x.audio === preferred) : undefined) ?? all[0];
}
