# Builds B-Anime in release mode, then its installer with B-Installer.
#
#   installer\build.ps1                          # version from app\src-tauri\tauri.conf.json
#   installer\build.ps1 -Bi C:\tools\bi.exe      # bi.exe not on PATH
#
# bi.exe needs bi-setup.exe next to it: build B-Installer (cargo build --release) or download
# both from https://github.com/ahmedanjum30/B-installer/releases.
param(
    [string]$Bi = "bi.exe",
    [string]$Version
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

if (-not $Version) {
    $Version = (Get-Content "$root\app\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json).version
}

Push-Location "$root\app"
try {
    npm ci
    if ($LASTEXITCODE -ne 0) { throw "npm ci failed" }
    # Release exe only; the installer comes from B-Installer, not Tauri's bundler.
    npx tauri build --no-bundle
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
} finally {
    Pop-Location
}

& $Bi "$PSScriptRoot\b-anime.toml" -D "version=$Version"
if ($LASTEXITCODE -ne 0) { throw "bi failed" }
Write-Host "Installer: $root\dist\B-Anime-Setup-$Version.exe"
