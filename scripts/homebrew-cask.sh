#!/bin/sh
# Prints the Homebrew cask for a release, for the tap (ahmedanjum30/homebrew-tap,
# Casks/b-anime.rb). The release workflow runs it for each new stable version.
#
#   sh scripts/homebrew-cask.sh <version> <zip>   # sha256 from the zip
#   sh scripts/homebrew-cask.sh <version> <sha256>
#   URL=file:///path/to.zip sh scripts/homebrew-cask.sh ...   # local testing
#
# The app is signed ad hoc, not notarized, so Gatekeeper refuses it while it carries the
# quarantine flag Homebrew sets on downloads. The postflight removes the flag, which is why this
# lives in our own tap: the official homebrew/cask only takes notarized apps.

set -eu

version=$1
if [ -f "$2" ]; then
    sha256=$(shasum -a 256 "$2" | cut -d' ' -f1)
else
    sha256=$2
fi
url=${URL:-'https://github.com/ahmedanjum30/B-Anime/releases/download/v#{version}/B-Anime-#{version}-macos.zip'}

cat <<EOF
cask "b-anime" do
  version "$version"
  sha256 "$sha256"

  url "$url"
  name "B-Anime"
  desc "Anime and manga library that tracks what you watch on AniList"
  homepage "https://ahmedanjum30.github.io/B-Anime/"

  livecheck do
    url :url
    strategy :github_latest
  end

  # B-Anime updates itself; brew upgrade leaves it alone unless asked to (--greedy).
  auto_updates true
  depends_on arch: :arm64
  depends_on macos: ">= :big_sur"

  app "B-Anime.app"

  # Signed ad hoc, not notarized: without this Gatekeeper won't open it.
  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-dr", "com.apple.quarantine", "#{appdir}/B-Anime.app"]
  end

  uninstall quit: "app.banime.desktop"

  zap trash: [
    "~/Library/Application Support/B-Anime",
    "~/Library/Caches/app.banime.desktop",
    "~/Library/WebKit/app.banime.desktop",
  ]
end
EOF
