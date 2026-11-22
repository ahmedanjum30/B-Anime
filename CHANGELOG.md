# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

The release workflow puts the section for the version you tag into the GitHub
release notes, above the install instructions and the auto-generated list of
commits. Add a `## [x.y.z] - YYYY-MM-DD` section below before you tag a release;
without one, the release still gets the install notes and the commit list.

## [Unreleased]

### Added
- Sponsor button (Patreon).

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
