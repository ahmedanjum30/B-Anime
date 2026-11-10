// Mirrors banime_core::home (serde camelCase).

export type Mark = "new" | "final";

export interface UpNext {
  mediaId: number;
  title: string;
  coverUrl: string | null;
  bannerUrl: string | null;
  color: string | null;
  episode: number;
  totalEpisodes: number | null;
  durationMin: number | null;
  resumeS: number | null;
  progress: number;
  mark: Mark | null;
  /** File name in the library, or null when the episode isn't in the library. */
  source: string | null;
}

export interface AiringSlot {
  mediaId: number;
  title: string;
  color: string | null;
  episode: number;
  airsAt: string;
  watched: boolean;
  mark: Mark | null;
}

export interface SyncStatus {
  anilistConnected: boolean;
  lastSynced: string | null;
  filesMatched: number;
}

export interface Viewer {
  id: number;
  name: string;
  avatar: { medium: string | null } | null;
}

export interface Account {
  clientConfigured: boolean;
  viewer: Viewer | null;
  lastSynced: string | null;
}

export interface SyncReport {
  anime: number;
  manga: number;
  airing: number;
  syncedAt: string;
}

export interface HomeData {
  upNext: UpNext[];
  week: AiringSlot[];
  sync: SyncStatus;
  animeCount: number;
  mangaCount: number;
  isSample: boolean;
}
