# B-Anime

## Changelog entries are one-liners

`CHANGELOG.md`'s section for a version is what users see: the app shows it in the "What's new"
window the first time it opens after an update (the release pipeline builds in just that section; `app/src-tauri/src/whats_new.rs`),
and the release workflow puts it in the GitHub release notes. Betas of a version show that
version's section.

So every entry is **one bullet point** (`- …`) of **one line**, written for users, so the list is
easy to scan:

- What changed for them, not how: "Invite links open B-Anime straight into a room", not
  "Added deep-link plugin and banime:// scheme".
- Short enough to read at a glance; no wrapped second line, no implementation detail.
- Grouped under `### Added`, `### Changed` and `### Fixed`, newest version first.
- Add the line when the change lands (in the `## [x.y.z]` section of the version being
  worked on), not at release time.

## Testing on a phone

Local test APKs (`scriptsndroid-build.ps1 -Quick`) keep the repository's version: don't stamp
a version into `Cargo.toml`/`tauri.conf.json` to install over a newer release. If Android
refuses the APK as a downgrade, uninstall the app and install it fresh
(`adb uninstall app.banime.desktop`, then `adb install`); losing the phone's app data is fine.
