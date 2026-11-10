// Browser-preview data for working on the UI without the Tauri shell.
// Mirrors banime_core::home::sample(); loaded lazily, only outside Tauri.
import type { AiringSlot, HomeData, UpNext } from "./types";

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
    coverUrl: null, durationMin: 24, progress: (episode - 1) / totalEpisodes,
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
