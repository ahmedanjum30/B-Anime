# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

(Rendered inside a Tauri 2 desktop window on Windows first; macOS/Linux later. Design language is web, not native-OS.)

## Stack

- Desktop shell: Tauri 2.
- Backend: Rust (axum + tokio, sqlx/SQLite in WAL mode), embedded in the app; optional LAN server mode, off by default.
- Frontend: SolidJS + Vite (user choice, picked for runtime performance).
- Written from scratch; other apps are feature references only, never a code source. License: undecided.

## Users

Anime and manga fans who keep a local library of episodes/chapters and track what they watch/read on AniList. They open the app in a lean-back or evening setting to pick up where they left off, browse what's next, and manage their lists. Public release: users range from casual fans to power users with large libraries (thousands of files).

## Product Purpose

A fast, beautiful desktop home for a personal anime/manga collection: scans local files and matches them to AniList without strict naming, plays them (external MPV/VLC/MPC-HC or in-app via transcoding/direct play), syncs progress back to AniList automatically, and adds online streaming and manga sources through extensions. Success: users never think about file names, tracking, or sync. They pick a show and it plays, and progress is recorded.

## Positioning

Full-featured anime and manga library features in a native-weight package: Rust backend + Tauri instead of Go + Electron, so lower RAM, a small installer, instant startup, and smooth scrolling through large libraries.

## Operating Context

- AniList is the tracker and source of truth for lists/progress (OAuth login).
- Local folders of video files and manga chapters; external players (MPV, VLC, MPC-HC).
- JS extensions for online streaming and manga sources, installed from a marketplace.
- User data lives in the OS app-data folder (e.g. %APPDATA%\B-Anime): config, SQLite db, cache, logs, extensions. AniList token in the OS credential store.

## Capabilities and Constraints

- In scope: AniList sync and discovery, local library scan/match, playback with progress sync, browser/in-app transcoding and direct play, extension runtime, online streaming, manga reader, release schedule, Discord Rich Presence, offline mode, playlists, themes.
- Out of scope (explicit): torrents of any kind, auto-downloader, debrid services. Other trackers (MAL, etc.) not planned.
- The app does not host or distribute media.
- Performance is a primary requirement: large grids must stay smooth, startup must be fast, and memory must stay low.

## Brand Commitments

Name: B-Anime (working name). No logo or other brand assets yet.

## Evidence on Hand

None yet: no users, screenshots, benchmarks, or testimonials. Do not fabricate any.

## Product Principles

1. Speed is a feature: every interaction should feel instant; never block the UI on the network or a scan.
2. Zero bookkeeping: matching, tracking, and sync happen automatically; manual fixes are the exception and easy.
3. The artwork leads: the user's collection (cover art, banners) is the visual content; chrome recedes.
4. Respect the library: never move, rename, or delete user files without explicit action.
5. Works offline: the local library stays usable without network access.
