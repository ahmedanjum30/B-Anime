// Browser-preview data for working on the UI without the Tauri shell.
// Mirrors banime_core::home::sample(); loaded lazily, only outside Tauri.
import type { AiringSlot, HomeData, LibraryEntry, LibraryKind, ListStatus, UpNext } from "./types";

/** A large synthetic list (2,400 anime / 400 manga) for checking the grid at archivist scale. */
export function browserPreviewLibrary(kind: LibraryKind): LibraryEntry[] {
  const syl = ["ka", "mi", "to", "ra", "shi", "no", "yu", "ki", "ha", "ru", "se", "n", "ai", "ko", "re", "zu"];
  const words = ["Sky", "Blade", "Night", "Garden", "Signal", "Tide", "Ember", "Orbit", "Lantern", "Echo"];
  const statuses: ListStatus[] = ["COMPLETED", "COMPLETED", "COMPLETED", "PLANNING", "PLANNING", "CURRENT", "PAUSED", "DROPPED", "REPEATING"];
  const formats = kind === "ANIME" ? ["TV", "TV", "TV", "MOVIE", "OVA", "ONA", "SPECIAL", "TV_SHORT"] : ["MANGA", "MANGA", "NOVEL", "ONE_SHOT"];
  const colors = ["#3d5a6e", "#6e3d3d", "#4a4a6e", "#3d6e55", "#6e5d3d", "#5a3d6e", "#3d6a6e", "#6e4a3d"];
  let seed = kind === "ANIME" ? 7 : 11;
  const rnd = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
  const pick = <T,>(a: T[]) => a[Math.floor(rnd() * a.length)];
  const n = kind === "ANIME" ? 2400 : 400;

  return Array.from({ length: n }, (_, i) => {
    const name = Array.from({ length: 2 + Math.floor(rnd() * 3) }, () => pick(syl)).join("");
    const title = `${name[0].toUpperCase()}${name.slice(1)} ${rnd() > 0.5 ? pick(words) : ""}`.trim();
    const total = kind === "ANIME" ? pick([12, 12, 13, 24, 25, 1, 6]) : pick([40, 120, 250, 8]);
    const status = pick(statuses);
    const airing = status === "CURRENT" && rnd() > 0.5;
    const released = airing ? Math.max(1, Math.floor(total * rnd())) : null;
    const progress =
      status === "COMPLETED" ? total : status === "PLANNING" ? 0 : Math.floor((released ?? total) * rnd());
    return {
      mediaId: 100000 + i,
      title,
      titleEnglish: null,
      coverUrl: null,
      color: pick(colors),
      format: pick(formats),
      mediaStatus: airing ? "RELEASING" : "FINISHED",
      total,
      released,
      status,
      progress,
      score: rnd() > 0.4 ? Math.round(50 + rnd() * 50) : null,
      updatedAt: new Date(Date.now() - i * 3_600_000).toISOString(),
    };
  });
}

export function browserPreviewHome(now = new Date()): HomeData {
  // Authored in local time, like real listings.
  const monday = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  monday.setDate(monday.getDate() - ((monday.getDay() + 6) % 7));
  const at = (day: number, h: number, m: number) =>
    new Date(monday.getFullYear(), monday.getMonth(), monday.getDate() + day, h, m).toISOString();

  const up = (
    mediaId: number, title: string, color: string, episode: number, totalEpisodes: number,
    resumeS: number | null, mark: UpNext["mark"], source: string,
  ): UpNext => ({
    mediaId, title, color, episode, totalEpisodes, resumeS, mark, source,
    coverUrl: null, bannerUrl: null, durationMin: 24, progress: (episode - 1) / totalEpisodes,
  });
  const slot = (
    mediaId: number, title: string, color: string, episode: number, airsAt: string,
    watched: boolean, mark: AiringSlot["mark"],
  ): AiringSlot => ({ mediaId, title, color, episode, airsAt, watched, mark });

  return {
    upNext: [
      up(1, "Frieren: Beyond Journey's End", "#3d5a6e", 18, 28, null, "new", "Frieren S2 - 18 [1080p].mkv"),
      up(2, "Dandadan", "#6e3d3d", 9, 12, 842, null, "Dandadan S02E09.mkv"),
      up(3, "The Apothecary Diaries", "#4a4a6e", 12, 12, null, "final", "Kusuriya no Hitorigoto - 36.mkv"),
      up(4, "Mushishi", "#3d6e55", 3, 26, null, null, "Mushishi - 03 (BD 1080p).mkv"),
    ],
    week: [
      slot(5, "Kaiju No. 8", "#8a7440", 7, at(0, 22, 30), true, null),
      slot(2, "Dandadan", "#a04a4a", 9, at(1, 17, 0), true, null),
      slot(3, "The Apothecary Diaries", "#6a64a8", 12, at(2, 22, 30), false, "final"),
      slot(1, "Frieren: Beyond Journey's End", "#4f7c99", 19, at(4, 17, 0), false, "new"),
      slot(6, "Orb: On the Movements of the Earth", "#4f8f7a", 6, at(5, 22, 30), false, null),
      slot(7, "Blue Lock", "#4a6aa0", 21, at(3, 23, 30), false, null),
      slot(8, "Sakamoto Days", "#a0784a", 4, at(6, 23, 30), false, null),
    ],
    sync: { anilistConnected: false, lastSynced: null, filesMatched: 0 },
    animeCount: 0,
    mangaCount: 0,
    isSample: true,
  };
}
