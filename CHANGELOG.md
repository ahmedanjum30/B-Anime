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

## [0.4.6]

### Added
- B-Anime for Mac (Apple Silicon): install it with Homebrew, and it updates itself.
- Rejoin your last room, even if someone else took over hosting; closing the app mid-session offers it at the next launch.
- Rooms have a name (the host's name by default), which the host can change; anyone can rename themselves.
- A notice says who paused, resumed or jumped.
- Your room code and invite link stay the same every time; New link replaces them.
- Your name and your room's name are remembered for next time.
- Pick where you play the room's episode from: your library or any of your sources.
- A show with episodes in your library can play from a source instead: pick it under Play from on the show's page, and Home and rooms follow.
- Guests play from the host's source by default, so everyone has the same copy of the episode.
- In a room, guests on the host's source play the show the host matched there (the right season), even if their own match differs.
- The waiting room checks everyone has the same copy of the episode as the host (the same length), since a different cut won't stay in step; Ready waits for it.
- A room keeps playing, sound and picture, with the app in the background on PC and phone.
- Leaving the player during a room keeps the episode going: on PC it pops out into a small window, on phone you keep hearing it.
- The room bar can pop the episode out into its small window when it didn't pop out by itself.

### Changed
- When someone loses connection mid-episode, the room pauses until they're back (up to a minute), unless the host chose not to wait for them.
- In the player, the room opens over the episode, which keeps playing.
- In a room, episodes start where the room is, not from your own saved spot.
- Start waits until everyone's player has loaded the episode, so everyone begins together; only the host can start without waiting.
- Everyone's episode starts at the same moment, instead of the others a little behind the host.
- When the host jumps during play, the room waits until everyone is there and loaded, then plays for all at once.
- The host switches the room to another show or episode in one click; it plays once everyone has loaded it.
- Leaving the player no longer pauses the room, and a bar takes you back to the episode.
- Your last room stays listed on the Together page while you're in a room.
- Settings → About (desktop) can record detailed network logs, for troubleshooting connections.
- You can say you're ready only once your episode has loaded, so a room never starts on one that won't play.
- When someone pauses, anyone a moment behind keeps watching up to that point instead of skipping ahead.
- In a room, only the host skips openings, endings and recaps (by hand or automatically), and everyone follows.

### Fixed
- On the Mac, videos show their picture instead of a black screen.
- On the Mac, B-Anime no longer asks for your password to reach the Keychain (sign in to AniList once more).
- Watch together on a show's page no longer stops what the room is watching; it takes you back to the episode.
- The People list shows each person's real ping.
- Someone left alone hosting a room after everyone else left now ends it, and rejoining finds a room's host in seconds.
- Someone who closes the app is noticed as gone in 10 seconds instead of 30.
- A guest's pause no longer gets lost, or undone a moment later, when it lands just after a room update.
- Coming back to the app as host no longer takes everyone back to where you left.
- A host who reopens their room carries on from where everyone else is.
- A new room starts in its lobby even when someone comes back to it from an earlier room.
- When the host leaves, the room stays paused with its new host until someone presses play.
- Someone whose connection drops gets back into the room instead of ending up alone in a room of their own.
- A host whose connection dropped while a guest took over gets back into the room as a guest.
- Closing the app no longer tells the room you paused.
- Leaving a room, or the room ending, pauses your player instead of letting it play on out of step.
- Closing the pop-out window no longer pauses the room for everyone.
- When the room moves to another episode while the player is popped out, it switches there and you stay on your page.
- Rejoining a room you hosted opens it again where it was, when nobody else is hosting it.
- Coming back to a room makes you the same person in it, so it stops waiting for you at once.
- An episode in your library plays from it in a room, instead of from a source.
- The player's header keeps its layout with long episode titles and on phones.
- On Android, online episodes show their skip intro and ending markers and buttons again.
- On Android, the screen stays on while an episode plays.
- On Android, full screen respects your phone's rotation lock.
- A paused phone no longer makes the room wait for it to load.
- A guest whose player lags or reloads catches up with the room, instead of jumping back to the start and getting stuck loading.

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
