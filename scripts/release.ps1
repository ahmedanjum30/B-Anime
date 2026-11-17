<#
.SYNOPSIS
  Bumps B-Anime's version, commits "Version X.Y.Z", tags vX.Y.Z and pushes both.
  The Release workflow on GitHub then builds and publishes the installer.

.EXAMPLE
  scripts\release.ps1            # 0.2.5 -> 0.2.6
  scripts\release.ps1 minor      # 0.2.5 -> 0.3.0
  scripts\release.ps1 major      # 0.2.5 -> 1.0.0
  scripts\release.ps1 0.4.0      # exact version
  scripts\release.ps1 -DryRun    # show the next version and stop
#>
param(
  [string]$Bump = "patch",
  [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Run([string]$what, [scriptblock]$cmd) {
  & $cmd
  if ($LASTEXITCODE -ne 0) { throw "$what failed (exit code $LASTEXITCODE)." }
}

# The version files. Each pattern matches the first "version" in the file, which is the app's own.
$files = @(
  @{ Path = "Cargo.toml";                    Pattern = '(?m)^(version = ")([^"]+)(")' },
  @{ Path = "app/package.json";              Pattern = '("version": ")([^"]+)(")' },
  @{ Path = "app/src-tauri/tauri.conf.json"; Pattern = '("version": ")([^"]+)(")' }
)

# --- Checks -----------------------------------------------------------------------------------
$branch = (git rev-parse --abbrev-ref HEAD).Trim()
if ($branch -ne "main") { throw "Releases go out from main; you're on '$branch'." }
if (git status --porcelain) { throw "There are uncommitted changes. Commit or stash them first." }
Run "git pull" { git pull -q --ff-only origin main }
Run "git fetch --tags" { git fetch -q --tags origin }

$current = [regex]::Match([IO.File]::ReadAllText("$root/Cargo.toml"), $files[0].Pattern).Groups[2].Value
if ($current -notmatch '^(\d+)\.(\d+)\.(\d+)$') { throw "Can't read the current version from Cargo.toml (got '$current')." }
$major, $minor, $patch = [int]$Matches[1], [int]$Matches[2], [int]$Matches[3]

switch -Regex ($Bump) {
  '^patch$' { $next = "$major.$minor.$($patch + 1)" }
  '^minor$' { $next = "$major.$($minor + 1).0" }
  '^major$' { $next = "$($major + 1).0.0" }
  '^v?\d+\.\d+\.\d+$' { $next = $Bump.TrimStart("v") }
  default { throw "Use patch, minor, major or a version like 0.3.0 (got '$Bump')." }
}
if ([version]$next -le [version]$current) { throw "$next isn't newer than the current $current." }
if (git tag --list "v$next") { throw "Tag v$next already exists." }

Write-Host "Releasing $current -> $next"
if ($DryRun) { return }

# --- Bump -------------------------------------------------------------------------------------
foreach ($f in $files) {
  $path = "$root/$($f.Path)"
  $text = [IO.File]::ReadAllText($path)
  $re = [regex]$f.Pattern
  if (-not $re.IsMatch($text)) { throw "No version found in $($f.Path)." }
  [IO.File]::WriteAllText($path, $re.Replace($text, "`${1}$next`${3}", 1))
}

# Lockfiles follow the manifests.
Push-Location app
try { Run "npm install --package-lock-only" { npm install --package-lock-only --silent } } finally { Pop-Location }
Run "cargo update -w" { cargo update -w -q }
$lock = [IO.File]::ReadAllText("$root/Cargo.lock")
foreach ($crate in "banime-app", "banime-core") {
  if ($lock -notmatch "name = `"$crate`"\r?\nversion = `"$([regex]::Escape($next))`"") {
    throw "Cargo.lock didn't pick up $next for $crate. Check the changes with 'git diff' (undo with 'git checkout .')."
  }
}

# --- Commit, tag, push ------------------------------------------------------------------------
# The message must start with "Version " so release-cache.yml skips its warm-up.
Run "git add" { git add Cargo.toml Cargo.lock app/package.json app/package-lock.json app/src-tauri/tauri.conf.json }
Run "git commit" { git commit -q -m "Version $next" }
Run "git tag" { git tag "v$next" }
Run "git push main" { git push -q origin main }
Run "git push tag" { git push -q origin "v$next" }

$repo = (git remote get-url origin).Trim() -replace '\.git$', '' -replace '^git@github\.com:', 'https://github.com/'
Write-Host "Pushed v$next. The installer builds on GitHub in about 12 minutes:"
Write-Host "  $repo/releases/tag/v$next"
