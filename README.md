# B-Anime

A fast desktop home for your anime and manga: it scans your local library, matches it to AniList without strict file naming, plays episodes, and syncs your progress automatically.

Built with a Rust backend and a Tauri 2 shell, so it starts instantly, uses a fraction of the memory of Electron apps, and stays smooth with libraries of thousands of shows.

> Status: early development (Phase 1). The Home screen runs on labelled sample data until AniList login and the library scanner land.

## Features (planned)

| Phase | Scope |
|---|---|
| 1 | AniList login and sync, local library scan and match, Home, playback in MPV/VLC/MPC-HC with progress sync |
| 2 | In-app playback: ffmpeg transcoding (HLS) and direct play |
| 3 | Extension runtime (JS), online streaming sources |
| 4 | Manga reader (local and extension sources) |
| 5 | Schedule, Discord Rich Presence, offline mode, playlists, themes, optional LAN server |

Out of scope: torrents, auto-downloaders, debrid services.

## Stack

- **Desktop shell:** Tauri 2
- **Backend:** Rust (`crates/core`): tokio, sqlx + SQLite (WAL)
- **Frontend:** SolidJS + Vite (`app/`)

## Layout

```
crates/core/        Backend library: data dir, database, home data (no UI)
  migrations/       SQLite schema
app/                SolidJS frontend
  src-tauri/        Tauri shell (thin layer over banime-core)
PRODUCT.md          Product brief
.impeccable/        Design direction and decision records
```

## User data

Everything lives in one folder: `%APPDATA%\B-Anime` on Windows, `~/Library/Application Support/B-Anime` on macOS, `~/.config/B-Anime` on Linux. Override it with `B_ANIME_DATA_DIR` (absolute path).

```
b-anime.db   SQLite database
cache/       Metadata and image cache (transcode/ for video segments)
logs/  extensions/  manga/  offline/
```

## Development

Requirements: Rust (MSVC toolchain on Windows; pinned by `rust-toolchain.toml`), Node.js 22+, and the [Tauri prerequisites](https://tauri.app/start/prerequisites/).

```sh
cd app
npm install
npm run tauri dev      # desktop app with hot reload
npm run dev            # UI only, in a browser, with preview data
npm run tauri build    # release installer
```

Run backend tests with `cargo test -p banime-core`.
