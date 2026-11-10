# Builds B-Anime, then its installer with B-Installer.
#
#   installer\build.ps1 -Bi C:\tools\bi.exe           # release: fully optimised, for shipping
#   installer\build.ps1 -Bi C:\tools\bi.exe -Quick    # quick: for local testing, builds fast
#
# For everyday testing you don't need an installer: `npm run tauri dev` in app\.
#
# bi.exe needs bi-setup.exe next to it: download both from
# https://github.com/ahmedanjum30/B-installer/releases. Pass -Bi or put bi.exe on PATH.
#
# Code signing (optional): set BANIME_SIGN_PFX (path to a .pfx code-signing certificate) and
# BANIME_SIGN_PASSWORD, and both B-Anime.exe and the setup are signed with signtool.
# Release builds also write dist\SHA256SUMS.txt, which the in-app updater checks.
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

function Find-SignTool {
    $cmd = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $kits = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
    if (Test-Path $kits) {
        return Get-ChildItem $kits -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match '\\x64\\' } | Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
    }
}

function Sign-File([string]$Path) {
    if (-not $env:BANIME_SIGN_PFX) { return }
    $tool = Find-SignTool
    if (-not $tool) { throw "BANIME_SIGN_PFX is set but signtool.exe wasn't found (install the Windows SDK)." }
    & $tool sign /f $env:BANIME_SIGN_PFX /p $env:BANIME_SIGN_PASSWORD /fd sha256 /tr http://timestamp.digicert.com /td sha256 /d "B-Anime" $Path
    if ($LASTEXITCODE -ne 0) { throw "signing $Path failed" }
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
Sign-File (Join-Path $PSScriptRoot "$bindir\banime-app.exe")

& $Bi "$PSScriptRoot\b-anime.toml" -D "version=$Version" -D "bindir=$bindir" -D "label=$label"
if ($LASTEXITCODE -ne 0) { throw "bi failed" }

$setup = "$root\dist\B-Anime-Setup-$label.exe"
Sign-File $setup

if (-not $Quick) {
    # The updater only installs a setup whose hash matches this file.
    $hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
    "$hash  B-Anime-Setup-$label.exe" | Out-File -Encoding ascii "$root\dist\SHA256SUMS.txt"
}
Write-Host ("Installer: $setup  ({0:n0}s)" -f $timer.Elapsed.TotalSeconds)
