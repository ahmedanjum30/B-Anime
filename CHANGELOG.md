# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

The release workflow puts the section for the version you tag into the GitHub
release notes, above the install instructions and the auto-generated list of
commits. Add a `## [x.y.z] - YYYY-MM-DD` section below before you tag a release;
without one, the release still gets the install notes and the commit list. Betas of x.y.z
(x.y.z-beta.N) use the same section, so keep it up to date as betas go out.

The app shows a version's section in its "What's new" window after an update, so entries are
one line each, written for users (see CLAUDE.md).

## [Unreleased]

## [0.5.0] - 2026-11-27

### Fixed
- The timeline jumps at once when you skip forward or back, instead of once the video has loaded.
- On phones, letting go of the timeline jumps the video there again.

## [0.4.9] - 2026-11-27

### Added
- On phones, drag along the timeline to see the frame at that spot.

### Fixed
- On the Mac, subtitles and the sidebar update again instead of staying stuck on what they first showed.

## [0.4.8]

### Added
- Hover over the timeline to see the frame at that spot (PC and Mac; online episodes load just that bit).
- Sign in with Google to pick up each show where you left it on any of your devices.
- A small note at the top shows while B-Anime syncs with AniList or Google Drive.
- Home offers to keep your place on every device under Up next; Not now puts it away for 3 days.

### Changed
- An episode counts as watched at 85% (was 80%).

### Fixed
- The player's controls stay up while the mouse is on them.
- Dragging the timeline moves the video only when you let go, instead of loading every spot on the way.

## [0.4.7] - 2026-11-26

### Added
- B-Anime keeps a log file; Settings → About shows it, for reporting problems.
- On the Mac, pick the video quality in the player (Auto picks it for you, as before).

### Changed
- B-Anime looks for updates when it starts (or when you check in Settings), no longer every 6 hours.

### Fixed
- On the Mac, videos keep their picture when you come back to B-Anime after a while in another app.
- Home, Library, Schedule and Extensions no longer blink when B-Anime starts or syncs with AniList.
- A background sync no longer clears a match search you're typing in Settings.
- Checking for updates no longer fails when several devices share your internet connection.

## [0.4.6] - 2026-11-25

### Added
- B-Anime for Mac (Apple Silicon): install it with Homebrew, and it updates itself.
- Watch together keeps playing in the background, and pops out into a small window when you leave the player.
- Rejoin your last room; your room code and invite link stay the same every time.
- Name your room and yourself; a notice says who paused, resumed or jumped.
- Play from: pick your library or a source for each show, in rooms and everywhere else.

### Changed
- Rooms stay in step: everyone starts together, and jumps or dropped connections wait for everyone.
- Guests play the host's source by default, and the waiting room checks everyone has the same copy.
- In a room, only the host skips openings, endings and recaps, and everyone follows.

### Fixed
- Many Watch together fixes for dropped connections, hosts leaving, rejoining and lagging players.
- On the Mac, videos show their picture, and no more Keychain password prompts (sign in to AniList once more).
- On Android, the screen stays on while playing, full screen respects the rotation lock, and skip markers are back.

## [0.4.5] - 2026-11-24

### Added
- Watch together: watch an episode in step with friends, each from your own source.
- Invite links open B-Anime straight into a room, and room codes are short (like K7QM-4X2P-9FTE).
- The host picks the show, episode, Sub or Dub and subtitles; everyone gets ready, then the host starts.
- A room waits for anyone still loading, rides out dropped connections, and finds a new host if the host leaves.
- See who can play the episode, and install the host's source in one tap if you can't.
- Watch together from any show's page.
- Don't wait: carry on without someone who's still loading, from the notice or People.
- When an update is ready, its notice installs it in one click or tap.
- This "What's new" window, once after each update.
- Numbered betas (0.4.5 Beta 1, Beta 2…) on their own releases page.

### Changed
- Phones: tapping the video shows the controls instead of pausing; play and pause from the middle.
- Old update downloads are cleaned up, keeping only the newest one waiting to install.
- Discover leaves out adult and ecchi titles, and show pages no longer suggest adult ones.
- Watch together: only the host moves the timeline; play and pause stay shared.
- Watch together copes better with slow connections: fewer jumps and no speeding up while loading.
- Phones: notices sit above the bottom bar.

### Fixed
- Phones: pages no longer show through above the Library, Manga and Discover headers.

## [0.4.4] - 2026-11-24

### Added
- **Beta updates.** Settings has a new choice, Stable or Beta. On Beta, B-Anime also installs
  test versions before they're released to everyone. Leaving Beta never downgrades: the beta
  you have stays until a stable version passes it.

## [0.4.3] - 2026-11-24

### Added
- **Remove match** on a show's source bar, for a match you picked by hand: it forgets that
  match and finds the show automatically again.

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
