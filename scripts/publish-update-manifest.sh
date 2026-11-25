#!/usr/bin/env bash
# Publishes a release's manifest (stable.json or beta.json, made by the release workflow) to the
# website, at https://ahmedanjum30.github.io/B-Anime/updates/<channel>.json. Installed copies of
# B-Anime read it to find updates (crates/core/src/updater.rs), which isn't limited like GitHub's
# API. Only updates/ changes; the site and its downloads stay as they are.
#
#   scripts/publish-update-manifest.sh <dir-with-json>
#
# Needs GH_TOKEN (contents: write) and GITHUB_REPOSITORY; run from the repository root.
set -euo pipefail

src="$1"
remote="https://x-access-token:${GH_TOKEN}@github.com/${GITHUB_REPOSITORY}.git"
work="$(mktemp -d)"

if ! git clone --quiet --depth 1 --branch gh-pages "$remote" "$work/live" 2>/dev/null; then
  echo "::warning::No gh-pages branch yet; the manifest isn't published (installed copies use GitHub's API)."
  exit 0
fi

mkdir -p "$work/live/updates"
cp "$src"/*.json "$work/live/updates/"
cd "$work/live"
git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git add updates
if git diff --cached --quiet; then
  echo "The manifest is already up to date."
  exit 0
fi
git commit --quiet -m "Update manifest ${GITHUB_REF_NAME:-} from ${GITHUB_SHA::7}"
git push --quiet "$remote" gh-pages
ls updates
echo "Published: https://${GITHUB_REPOSITORY%%/*}.github.io/${GITHUB_REPOSITORY#*/}/updates/"
