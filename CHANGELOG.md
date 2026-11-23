# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

The release workflow puts the section for the version you tag into the GitHub
release notes, above the install instructions and the auto-generated list of
commits. Add a `## [x.y.z] - YYYY-MM-DD` section below before you tag a release;
without one, the release still gets the install notes and the commit list.

## [Unreleased]

## [0.4.2] - 2026-11-24

### Added
- **AnikotoTv** extension: subbed and dubbed anime in English.

### Changed
- AniPm and AnimeVillage extensions updated.
- The player starts in the Sub or Dub you opened the episode with, on the server you last picked
  for the show (or the same server in that language), and switching audio keeps it as the show's
  choice.

### Fixed
- Videos from hosts that hide each piece of video behind a small image now play: B-Anime removes
  the image before the player sees it.

## [0.4.1] - 2026-11-24

### Fixed
- Connecting AniList on a phone: the first time, when you had to log in to AniList, it failed
  with "couldn't reach AniList". Android blocks the network for apps in the background, and
  B-Anime asked AniList who you are while the browser was still in front. It now keeps trying
  until you're back in B-Anime.
- Connecting AniList again after coming back from the browser failed with "couldn't listen on
  port 48731: address already in use". A new attempt now replaces the one still waiting.
- Watching without an AniList account no longer shows "Couldn't update AniList" at 80% of an
  episode.

## [0.4.0] - 2026-11-23

### Fixed
- **This week** showed none of the week's episodes in the first hours of Monday east of
  Greenwich (on phones only "Today" was listed). The week now follows your own time zone.
- Double-tap to seek: "+10 s" stays on one line near the edge of the screen.

## [0.3.9] - 2026-11-23

### Added
- Touch screens: double-tap the right side of the video to jump 10 seconds ahead, the left
  side to go 10 seconds back. Keep tapping to jump further.
- Android: **Picture in picture** works, from the player's settings menu (the gear). The video
  keeps playing in a small window over your other apps; closing that window stops it.

### Fixed
- Phones held upright: the player's controls stay on one line however long the time gets
  (they split over two uneven lines once it reached 10:00). Previous episode is in the
  episode list there.

## [0.3.8] - 2026-11-23

### Added
- Android: **Full screen** really is full screen. The status and navigation bars hide and
  the screen turns to landscape, and both come back when you leave full screen.
- The player remembers the server and the audio language you pick. Your last server
  becomes the default for shows you haven't picked one for.
- Sponsor button (Patreon) on the GitHub page.

### Fixed
- The next episode starts playing by itself (on Android, and whenever AniSkip was slow).
- Playback no longer stops partway through an episode on a slow connection. Network
  retries also wait longer each time, so a dropped connection has time to come back.
- Switching servers mid-episode keeps your place.
- Phones: subtitle **Position** (Low, Normal, High) changes the height again.
- Phones held upright: the **Full screen** button shows (the controls wrap to two lines).
- Phones held sideways: the subtitles menu scrolls, so all its settings can be reached.

### Removed
- Android: the picture-in-picture button, which did nothing there.

## [0.3.7] - 2026-11-20

### Changed
- Streams: each server sends its own headers, and the player keeps your place
  and your choices across a server change.

## [0.3.6] - 2026-11-20

### Fixed
- Brought back HTTP/2 in reqwest, which AniPm's streams need.

## [0.3.5] - 2026-11-20

### Changed
- Extensions: a **Recommended** tag on the card, instead of a separate section.

## [0.3.4] - 2026-11-20

### Added
- Extensions and Support. Extensions auto-update on launch, from the manifest.

## [0.3.3] - 2026-11-20

### Fixed
- Android: in-app update install now works, and install errors are shown.

### Changed
- Release builds use thin LTO (16 codegen units); Rust pinned to 1.100.0 so
  release caches match across runner images.

## [0.3.2] - 2026-11-20

### Fixed
- Keep bottom subtitles at least 56 px above the bottom edge.

### Changed
- Pre-compile the Android release build on main, and retry the Android NDK
  download up to three times.

## [0.3.1] - 2026-11-20

### Changed
- The tag is the version: releases no longer need a version-bump commit.

### Fixed
- Stop racing animations when Windows "Animation effects" is off.

## [0.3.0] - 2026-11-20

### Added
- Subtitle language as a page of its menu, with Sub/Dub remembered per show.

### Fixed
- Keep subtitles drawn over a hardware video overlay.
