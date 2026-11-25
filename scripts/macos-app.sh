#!/bin/sh
# Builds B-Anime.app on a Mac, for that Mac's kind (Apple Silicon on GitHub's runners), and zips
# it as dist/B-Anime-<version>-macos.zip: the file Homebrew installs and B-Anime updates from.
#
#   sh scripts/macos-app.sh      (after `npm ci` in app/)
#
# The app is signed ad hoc (not with an Apple Developer ID): macOS runs it, but the first time
# only after a right-click > Open, or when installed by Homebrew, which takes care of that.

set -eu
cd "$(dirname "$0")/.."

version=$(node -p "require('./app/src-tauri/tauri.conf.json').version")
(cd app && npx tauri build --bundles app)

app=$(find target/release/bundle/macos -maxdepth 1 -name "*.app" | head -n 1)
[ -n "$app" ] || { echo "No app in target/release/bundle/macos" >&2; exit 1; }
[ "$(basename "$app")" = "B-Anime.app" ] || { echo "Expected B-Anime.app, got $app" >&2; exit 1; }
codesign --force --sign - "$app"
codesign --verify --strict "$app"

mkdir -p dist
zip="dist/B-Anime-$version-macos.zip"
rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"
echo "$app"
echo "$zip ($(du -h "$zip" | cut -f1))"
