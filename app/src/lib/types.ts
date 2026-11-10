// Mirrors banime_core::home (serde camelCase).

export type Mark = "new" | "final";

export interface UpNext {
  mediaId: number;
  title: string;
  coverUrl: string | null;
  color: string | null;
  episode: number;
  totalEpisodes: number | null;
  durationMin: number | null;
  resumeS: number | null;
  progress: number;
  mark: Mark | null;
  source: string;
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

export interface HomeData {
  upNext: UpNext[];
  week: AiringSlot[];
  sync: SyncStatus;
  animeCount: number;
  mangaCount: number;
  isSample: boolean;
}
