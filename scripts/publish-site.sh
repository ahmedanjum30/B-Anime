#!/usr/bin/env bash
# Publishes the website (site/) to the gh-pages branch, served by GitHub Pages at
# https://ahmedanjum30.github.io/B-Anime/.
#
#   scripts/publish-site.sh                      # the site only; downloads stay as they were
#   scripts/publish-site.sh <version> <files-dir>  # also this release's downloads
#
# With a version, the newest downloads sit at fixed links (the website's buttons):
#   download/B-Anime-Setup.exe, download/B-Anime-arm64.apk and download/B-Anime-macos.zip
# Only the latest downloads are kept, and gh-pages is a single fresh commit each time, so the
# branch doesn't grow by ~30 MB with every release.
#
# Needs GH_TOKEN (contents: write) and GITHUB_REPOSITORY; run from the repository root.
set -euo pipefail

version="${1:-}"
files="${2:-}"
remote="https://x-access-token:${GH_TOKEN}@github.com/${GITHUB_REPOSITORY}.git"
work="$(mktemp -d)"

# What's live now, to keep its downloads (absent on the very first publish).
if git clone --quiet --depth 1 --branch gh-pages "$remote" "$work/live" 2>/dev/null; then
  echo "Keeping the current downloads."
else
  echo "No gh-pages branch yet; creating it."
  mkdir -p "$work/live"
fi

mkdir -p "$work/out"
cp -r site/. "$work/out/"
if [ -d "$work/live/download" ]; then cp -r "$work/live/download" "$work/out/"; fi
# The release manifests installed copies read for updates (scripts/publish-update-manifest.sh).
if [ -d "$work/live/updates" ]; then cp -r "$work/live/updates" "$work/out/"; fi

if [ -n "$version" ]; then
  mkdir -p "$work/out/download"
  cp "$files/B-Anime-Setup-$version.exe" "$work/out/download/B-Anime-Setup.exe"
  if [ -f "$files/B-Anime-$version-arm64.apk" ]; then
    cp "$files/B-Anime-$version-arm64.apk" "$work/out/download/B-Anime-arm64.apk"
  fi
  if [ -f "$files/B-Anime-$version-macos.zip" ]; then
    cp "$files/B-Anime-$version-macos.zip" "$work/out/download/B-Anime-macos.zip"
  fi
  echo "$version" > "$work/out/download/VERSION"
fi

cd "$work/out"
git init --quiet -b gh-pages
git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git add -A
git commit --quiet -m "Site${version:+ and downloads $version} from ${GITHUB_SHA::7}"
git push --quiet --force "$remote" gh-pages
echo "Published: https://${GITHUB_REPOSITORY%%/*}.github.io/${GITHUB_REPOSITORY#*/}/"
