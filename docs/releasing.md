# Releasing B-Anime

## Ship a version

From a clean `main`:

```powershell
scripts\release.ps1            # 0.3.0 -> 0.3.1, 0.3.9 -> 0.4.0 (parts stay single digits)
scripts\release.ps1 minor      # 0.3.4 -> 0.4.0
scripts\release.ps1 major      # 0.3.4 -> 1.0.0
scripts\release.ps1 0.5.0      # an exact version
scripts\release.ps1 -DryRun    # only print the next version
```

**The tag is the version.** The script reads the newest `vX.Y.Z` tag, picks the next version,
tags `main` and pushes the tag; there's no version-bump commit. It stops without changing
anything if you're not on `main`, have uncommitted changes, or the tag already exists.

The **Release** workflow (`.github/workflows/release.yml`) writes the tag's version into
`Cargo.toml`, `Cargo.lock`, `tauri.conf.json`, `package.json` and `package-lock.json` on the
build machine (`scripts/set-version.mjs`; nothing is committed), builds the app in release mode,
packs it with [B-Installer](https://github.com/ahmedanjum30/B-installer), and publishes the
release.

So the version in the repository's files isn't kept current: local builds (`npm run tauri dev`,
`scripts\android-build.ps1`) show whatever those files say. To build a specific version locally,
run `node scripts/set-version.mjs 0.3.1` first and `git checkout` the files afterwards.

## What a release contains

The **Release** workflow builds, in parallel:

- the Windows setup, `B-Anime-Setup-<version>.exe`;
- the Android APK, `B-Anime-<version>-arm64.apk`, signed with the release key. This needs the
  repository secrets `ANDROID_KEYSTORE` (the `.jks`, base64-encoded), `ANDROID_KEYSTORE_PASSWORD`
  and `ANDROID_KEY_ALIAS`; without them the release goes out with Windows only.

Then it publishes one GitHub release with both files and a combined `SHA256SUMS.txt`, and updates
the website (below). The Windows release still goes out if the Android build fails.

## Website

`site/` is the website, published to the `gh-pages` branch and served by GitHub Pages at
https://ahmedanjum30.github.io/B-Anime/. Every release also puts its downloads at fixed links
there: `download/B-Anime-Setup.exe` and `download/B-Anime-arm64.apk` (the site's buttons). Only
the latest are kept.

Changes to `site/` on `main` publish on their own (the **Website** workflow, or run it by hand
from Actions). The first time, turn on Pages in the repository's Settings → Pages: deploy from
the `gh-pages` branch, `/ (root)`.

## How installed copies update

- B-Anime checks for a newer release 3 seconds after launch, and when Check for updates is
  clicked in Settings (release builds only; `npm run tauri dev` never updates itself).
- It reads the release manifests on the website, `updates/stable.json` (and `beta.json` on the
  Beta channel) at https://ahmedanjum30.github.io/B-Anime/, which the release workflow writes
  after publishing (`scripts/publish-update-manifest.sh`): the version, each file's link and its
  SHA-256. They aren't limited like GitHub's API (60 requests an hour per connection, shared by
  every device on it). Without manifests it asks GitHub's API instead.
- When the release is newer, it downloads `B-Anime-Setup-<version>.exe`, only from GitHub's own
  hosts, and checks its SHA-256 against the release's `SHA256SUMS.txt`. A release without
  checksums, or a download that doesn't match, is refused.
- The user sees **Restart to update** in the sidebar and in Settings → About. The setup then runs
  silently with `/UPDATE=1`: it closes B-Anime, replaces it, and starts it again
  (`[[run]] if_param = "UPDATE=1"` in `installer/b-anime.toml`).

## Code signing

Unsigned installers trigger Windows SmartScreen ("Windows protected your PC") until enough people
have run them. Signing removes that, and needs a **code-signing certificate**: an OV or EV
certificate from a certificate authority (Sectigo, DigiCert, SSL.com, Certum and others), or
Microsoft's **Azure Trusted Signing** service. These cost money and need identity verification, so
they can't be generated here.

Once you have a `.pfx` certificate:

- **Locally:** set `BANIME_SIGN_PFX` (path to the `.pfx`) and `BANIME_SIGN_PASSWORD`, then run
  `installer\build.ps1`. Both `B-Anime.exe` and the setup are signed and timestamped.
- **In the release workflow:** add repository secrets `WINDOWS_CERT_PFX` (the `.pfx` file,
  base64-encoded: `[Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx"))`) and
  `WINDOWS_CERT_PASSWORD`. Releases are signed from then on.

EV certificates usually live on a hardware token or in a cloud HSM and can't be exported to a
`.pfx`. For those (and for Azure Trusted Signing), replace the `Sign-File` function in
`installer/build.ps1` with the provider's signing command.
