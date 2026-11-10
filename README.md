# B-Anime

A fast desktop home for your anime and manga: it scans your local library, matches it to AniList without strict file naming, plays episodes, and syncs your progress automatically.

Built with a Rust backend and a Tauri 2 shell, so it starts instantly, uses a fraction of the memory of Electron apps, and stays smooth with libraries of thousands of shows.

## Features

- **AniList:** sign in once; lists, progress and the airing schedule sync automatically.
- **Library:** pick folders; files are found, matched to AniList (release-group names, SxxEyy,
  season folders, split seasons and absolute numbering), and new files are picked up as they
  arrive. Wrong or missing matches are fixed in Settings.
- **Watch:** library episodes in the built-in player, MPV (progress tracked) or VLC; online
  episodes through extensions, with subtitles, skip intro/outro, resume and auto-next.
- **Read:** manga through extensions, in single page, two pages or long strip.
- **Browse:** Home (Up next, this week), Library, show pages with list editing, Discover and
  search, Schedule.
- **Updates:** installed copies update themselves from GitHub Releases (verified downloads).

Still to come: Discord Rich Presence, themes, offline mode.

Out of scope: torrents, auto-downloaders, debrid services.

Online sources come from extensions, which users install themselves; B-Anime ships with none.
To write one, see [docs/extensions.md](docs/extensions.md).

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
npm run tauri dev      # day-to-day: the desktop app with instant UI reload, no installer needed
npm run dev            # UI only, in a browser, with preview data
```

Run backend tests with `cargo test -p banime-core`.

### AniList sign-in

B-Anime's AniList API client (id 56545, redirect URL `http://127.0.0.1:48731/callback`) is built
in; nothing to configure. A fork can use its own client by setting `BANIME_ANILIST_CLIENT_ID`.
No client secret is used or needed.

### Installers

Built with [B-Installer](https://github.com/ahmedanjum30/B-installer); download `bi.exe` and
`bi-setup.exe` from its releases.

```powershell
installer\build.ps1 -Bi C:\tools\bi.exe -Quick   # local testing: ~25 s after the first build
installer\build.ps1 -Bi C:\tools\bi.exe          # shipping: fully optimised (LTO), ~5 min
```

Output goes to `dist\` (`B-Anime-Setup-<version>-quick.exe` for quick builds).

To publish a release (which installed copies then update to), and for code signing, see
[docs/releasing.md](docs/releasing.md).
