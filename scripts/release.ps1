<#
.SYNOPSIS
  Releases B-Anime: tags main with the next version and pushes the tag. No version-bump commit:
  the tag is the version, and the Release workflow writes it into the build
  (scripts/set-version.mjs) before building the installer and the APK.

.EXAMPLE
  scripts\release.ps1            # 0.3.0 -> 0.3.1, 0.3.9 -> 0.4.0 (parts stay single digits)
  scripts\release.ps1 minor      # 0.3.4 -> 0.4.0
  scripts\release.ps1 major      # 0.3.4 -> 1.0.0
  scripts\release.ps1 0.5.0      # exact version
  scripts\release.ps1 -DryRun    # show the next version and stop
  scripts\release.ps1 -Beta      # a beta: only copies on the Beta update channel get it

  A beta is published as a GitHub pre-release. Stable users get it once it's promoted with
  scripts\promote.ps1 <version> (the same build, no rebuild).
#>
param(
  [string]$Bump = "patch",
  [switch]$Beta,
  [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Run([string]$what, [scriptblock]$cmd) {
  & $cmd
  if ($LASTEXITCODE -ne 0) { throw "$what failed (exit code $LASTEXITCODE)." }
}

# --- Checks -----------------------------------------------------------------------------------
$branch = (git rev-parse --abbrev-ref HEAD).Trim()
if ($branch -ne "main") { throw "Releases go out from main; you're on '$branch'." }
if (git status --porcelain) { throw "There are uncommitted changes. Commit them (or stash them) first: a release is what's committed." }
Run "git pull" { git pull -q --ff-only origin main }
Run "git fetch --tags" { git fetch -q --tags origin }

# The current version is the newest release tag.
$latest = git tag --list "v*" | Where-Object { $_ -match '^v\d+\.\d+\.\d+$' } |
  Sort-Object { [version]($_.TrimStart("v")) } | Select-Object -Last 1
if (-not $latest) { throw "No release tags (vX.Y.Z) found." }
$current = $latest.TrimStart("v")
$major, $minor, $patch = $current.Split(".") | ForEach-Object { [int]$_ }

switch -Regex ($Bump) {
  # Each part stays a single digit: 0.3.9 -> 0.4.0, 0.9.9 -> 1.0.0.
  '^patch$' {
    if ($patch -lt 9) { $next = "$major.$minor.$($patch + 1)" }
    elseif ($minor -lt 9) { $next = "$major.$($minor + 1).0" }
    else { $next = "$($major + 1).0.0" }
  }
  '^minor$' { $next = if ($minor -lt 9) { "$major.$($minor + 1).0" } else { "$($major + 1).0.0" } }
  '^major$' { $next = "$($major + 1).0.0" }
  '^v?\d+\.\d+\.\d+$' { $next = $Bump.TrimStart("v") }
  default { throw "Use patch, minor, major or a version like 0.4.0 (got '$Bump')." }
}
if ([version]$next -le [version]$current) { throw "$next isn't newer than the current $current." }
if (git tag --list "v$next") { throw "Tag v$next already exists." }

$unpushed = (git rev-list --count origin/main..HEAD).Trim()
$kind = if ($Beta) { "beta " } else { "" }
Write-Host "Releasing $kind$current -> $next (main at $((git rev-parse --short HEAD).Trim()))"
if ($DryRun) { return }

# --- Tag and push -----------------------------------------------------------------------------
# Local commits not on GitHub yet go up first, so the tag points at something CI can build.
if ($unpushed -ne "0") { Run "git push main" { git push -q origin main } }
# A beta is an annotated tag whose message is "beta"; the Release workflow reads it and publishes
# a pre-release. Stable releases are plain tags.
if ($Beta) { Run "git tag" { git tag -a "v$next" -m "beta" } } else { Run "git tag" { git tag "v$next" } }
Run "git push tag" { git push -q origin "v$next" }

$repo = (git remote get-url origin).Trim() -replace '\.git$', '' -replace '^git@github\.com:', 'https://github.com/'
Write-Host "Pushed $($kind)v$next. The installer and APK build on GitHub in about 12 minutes:"
Write-Host "  $repo/releases/tag/v$next"
