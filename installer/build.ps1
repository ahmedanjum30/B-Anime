# Builds B-Anime, then its installer with B-Installer.
#
#   installer\build.ps1 -Bi C:\tools\bi.exe           # release: fully optimised, for shipping
#   installer\build.ps1 -Bi C:\tools\bi.exe -Quick    # quick: for local testing, builds fast
#
# For everyday testing you don't need an installer: `npm run tauri dev` in app\.
#
# bi.exe needs bi-setup.exe next to it: download both from
# https://github.com/ahmedanjum30/B-installer/releases. Pass -Bi or put bi.exe on PATH.
param(
    [string]$Bi = "bi.exe",
    [string]$Version,
    [switch]$Quick
)
# Native tools (npm, cargo, bi) report failure through exit codes, checked after each one.
# "Stop" would also abort on their harmless stderr output under Windows PowerShell 5.1.
$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot

if (-not $Version) {
    $Version = (Get-Content "$root\app\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json).version
}

$timer = [Diagnostics.Stopwatch]::StartNew()
Push-Location "$root\app"
try {
    if (-not (Test-Path node_modules)) {
        npm ci
        if ($LASTEXITCODE -ne 0) { throw "npm ci failed" }
    }
    if ($Quick) {
        # Same as `tauri build` minus LTO: build the UI, then the exe with the UI embedded.
        npm run build
        if ($LASTEXITCODE -ne 0) { throw "frontend build failed" }
        cargo build -p banime-app --profile quick --features tauri/custom-protocol
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    } else {
        npx tauri build --no-bundle
        if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
    }
} finally {
    Pop-Location
}

$bindir = if ($Quick) { "..\target\quick" } else { "..\target\release" }
$label = if ($Quick) { "$Version-quick" } else { $Version }
& $Bi "$PSScriptRoot\b-anime.toml" -D "version=$Version" -D "bindir=$bindir" -D "label=$label"
if ($LASTEXITCODE -ne 0) { throw "bi failed" }
Write-Host ("Installer: $root\dist\B-Anime-Setup-$label.exe  ({0:n0}s)" -f $timer.Elapsed.TotalSeconds)
