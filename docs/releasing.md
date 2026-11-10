# Releasing B-Anime

## Ship a version

1. Bump the version in `app/src-tauri/tauri.conf.json` (and `version` under `[workspace.package]`
   in `Cargo.toml`, to keep them in step).
2. Commit, then tag and push:

   ```
   git tag v0.2.0
   git push origin v0.2.0
   ```

The **Release** workflow (`.github/workflows/release.yml`) builds the app in release mode,
packs it with [B-Installer](https://github.com/ahmedanjum30/B-installer), writes
`SHA256SUMS.txt`, and publishes a GitHub release with both files. It refuses to run when the
tag doesn't match the version in `tauri.conf.json`.

## How installed copies update

- B-Anime checks the latest GitHub release 20 seconds after launch and every 6 hours
  (release builds only; `npm run tauri dev` never updates itself).
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
