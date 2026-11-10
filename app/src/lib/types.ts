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
  /** False for shows not on the user's list (Schedule, "everything airing"). */
  onList?: boolean;
}

export interface SyncStatus {
  anilistConnected: boolean;
  lastSynced: string | null;
  filesMatched: number;
}

export type LibraryKind = "ANIME" | "MANGA";
export type ListStatus = "CURRENT" | "PLANNING" | "COMPLETED" | "DROPPED" | "PAUSED" | "REPEATING";

export interface LibraryEntry {
  mediaId: number;
  title: string;
  titleEnglish: string | null;
  coverUrl: string | null;
  color: string | null;
  format: string | null;
  mediaStatus: string | null;
  /** Episodes or chapters, when known. */
  total: number | null;
  /** Episodes out so far for a show still airing. */
  released: number | null;
  status: ListStatus;
  progress: number;
  /** 0–100 */
  score: number | null;
  updatedAt: string;
}

export interface MediaRef {
  id: number;
  type: LibraryKind;
  format: string | null;
  status?: string | null;
  title: { userPreferred: string };
  coverImage: { large: string | null; color: string | null };
}

/** AniList's Media object as the show page query returns it. */
export interface Media {
  id: number;
  idMal: number | null;
  type: LibraryKind;
  format: string | null;
  status: string | null;
  episodes: number | null;
  chapters: number | null;
  volumes: number | null;
  duration: number | null;
  source: string | null;
  season: string | null;
  seasonYear: number | null;
  startDate: { year: number | null; month: number | null; day: number | null };
  endDate: { year: number | null; month: number | null; day: number | null };
  title: { userPreferred: string; romaji: string | null; english: string | null; native: string | null };
  description: string | null;
  coverImage: { extraLarge: string | null; large: string | null; color: string | null };
  bannerImage: string | null;
  genres: string[];
  averageScore: number | null;
  popularity: number | null;
  siteUrl: string;
  studios: { nodes: { id: number; name: string }[] };
  nextAiringEpisode: { episode: number; airingAt: number } | null;
  streamingEpisodes: { title: string | null; thumbnail: string | null }[];
  relations: { edges: { relationType: string; node: MediaRef }[] };
  recommendations: { nodes: { mediaRecommendation: MediaRef | null }[] };
}

export interface ListEntry {
  status: ListStatus;
  progress: number;
  score: number | null;
  updatedAt: string;
}

export interface MediaPage {
  media: Media;
  listEntry: ListEntry | null;
  localFiles: { episode: number; fileName: string }[];
  fromCache: boolean;
  fetchedAt: string;
}

export interface EntryChange {
  status?: ListStatus;
  progress?: number;
  /** 0–100 */
  score?: number;
}

export interface MediaCard {
  id: number;
  kind: LibraryKind;
  title: string;
  coverUrl: string | null;
  color: string | null;
  format: string | null;
  mediaStatus: string | null;
  total: number | null;
  seasonYear: number | null;
  averageScore: number | null;
  genres: string[];
  nextEpisode: number | null;
  nextAiringAt: number | null;
  listStatus: ListStatus | null;
  progress: number | null;
}

export type DiscoverSection = "trending" | "this_season" | "next_season" | "all_time";

export interface BrowseFilter {
  search?: string;
  section?: DiscoverSection;
  kind?: LibraryKind;
  genre?: string;
  format?: string;
  year?: number;
  page: number;
}

export interface BrowsePage {
  items: MediaCard[];
  hasNext: boolean;
}

export type ScheduleScope = "mine" | "all";

export interface ScheduleItem {
  mediaId: number;
  title: string;
  coverUrl: string | null;
  color: string | null;
  format: string | null;
  episode: number;
  totalEpisodes: number | null;
  airsAt: string;
  listStatus: ListStatus | null;
  progress: number | null;
}

export type ExtKind = "anime" | "manga";

export interface ExtManifest {
  id: string;
  name: string;
  version: string;
  kind: ExtKind;
  author: string;
  description: string;
  language: string | null;
  icon: string | null;
  codeUrl: string;
  apiVersion: number;
}

export interface InstalledExtension extends ExtManifest {
  sourceUrl: string;
  enabled: boolean;
  installedAt: string;
  updatedAt: string;
}

export interface ExtCallOutput {
  value: unknown;
  logs: string[];
}

export interface RepoEntry {
  manifestUrl: string;
  id: string | null;
  name: string;
  version: string | null;
  kind: ExtKind | null;
  author: string | null;
  description: string | null;
  icon: string | null;
  language: string | null;
}

export interface Repo {
  url: string;
  name: string | null;
  description: string | null;
  extensions: RepoEntry[];
}

// ── Streaming & reading ──
export interface SourceResult {
  id: string;
  title: string;
  image: string | null;
  year: number | null;
}

export interface SourceItem {
  id: string;
  number: number;
  title: string | null;
  volume: number | null;
}

export type MatchState =
  | { status: "matched"; sourceId: string; title: string; auto: boolean }
  | { status: "unmatched"; candidates: SourceResult[] };

export interface Stream {
  /** "file" is a library file served by the local proxy. `audio` is "ja"/"en" when known. */
  sources: { url: string; kind: "hls" | "mp4" | "file"; label: string; audio: string | null }[];
  subtitles: { url: string; lang: string; label: string }[];
}

export interface SkipTime {
  kind: string;
  start: number;
  end: number;
}

// ── Library folders, playback, updates ──
export interface ScanReport {
  files: number;
  matched: number;
  unmatched: number;
  added: number;
  removed: number;
  millis: number;
  finishedAt: string | null;
}

export interface ScanStatus {
  running: boolean;
  phase: "walking" | "matching" | "saving" | null;
  done: number;
  total: number;
}

export interface LibraryStatus {
  roots: string[];
  scan: ScanStatus;
  last: ScanReport | null;
}

export interface UnmatchedGroup {
  title: string;
  season: number | null;
  files: { path: string; fileName: string; episode: number | null }[];
}

export type PlayerChoice = "builtin" | "mpv" | "vlc";

export interface PlayerPrefs {
  player: PlayerChoice;
  available: string[];
  /** "ja" (original) or "en" (English dub). */
  audio: string;
  /** A language code ("en", "es", …) or "off". */
  subtitles: string;
}

export interface UpdateInfo {
  version: string;
  notes: string;
  publishedAt: string | null;
  pageUrl: string;
}

export interface UpdateStatus {
  current: string;
  enabled: boolean;
  available: UpdateInfo | null;
  ready: boolean;
  error: string | null;
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
  /** Set while automatic syncing is failing; it keeps retrying. */
  syncError: string | null;
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
