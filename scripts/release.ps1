<#
.SYNOPSIS
  Releases B-Anime: tags the next version and pushes the tag. No version-bump commit: the tag is
  the version, and the Release workflow writes it into the build (scripts/set-version.mjs)
  before building the installer and the APK.

.DESCRIPTION
  Betas: numbered test builds of the next version, from the beta branch. -Beta tags
  vX.Y.Z-beta.N (N counts up by itself), published on the beta Releases page; only copies on
  the Beta update channel get them. When a version's betas are good, scripts\promote.ps1 X.Y.Z
  releases it to everyone.

  Stable releases without a beta (urgent fixes) go out from main, as plain vX.Y.Z.

.EXAMPLE
  scripts\release.ps1 -Beta            # the next beta: 0.4.5-beta.1, then -beta.2, ...
  scripts\release.ps1 0.4.6 -Beta      # the first beta of 0.4.6 (or the next one, if it has some)
  scripts\release.ps1                  # stable from main: 0.3.0 -> 0.3.1, 0.3.9 -> 0.4.0
  scripts\release.ps1 minor            # 0.3.4 -> 0.4.0
  scripts\release.ps1 0.5.0            # exact version
  scripts\release.ps1 -Beta -DryRun    # show the version and stop
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
# Betas go out from the beta branch (where work lands); stable releases from main (what stable
# users run).
$want = if ($Beta) { "beta" } else { "main" }
$branch = (git rev-parse --abbrev-ref HEAD).Trim()
if ($branch -ne $want) {
  $what = if ($Beta) { "Betas go out from the beta branch" } else { "Stable releases go out from main (betas: add -Beta, from beta)" }
  throw "$what; you're on '$branch'. Run: git switch $want"
}
if (git status --porcelain) { throw "There are uncommitted changes. Commit them (or stash them) first: a release is what's committed." }
Run "git pull" { git pull -q --ff-only origin $branch }
Run "git fetch --tags" { git fetch -q --tags --force origin }

# The current stable version is the newest vX.Y.Z tag (betas don't count).
$latest = git tag --list "v*" | Where-Object { $_ -match '^v\d+\.\d+\.\d+$' } |
  Sort-Object { [version]($_.TrimStart("v")) } | Select-Object -Last 1
if (-not $latest) { throw "No release tags (vX.Y.Z) found." }
$current = $latest.TrimStart("v")
$major, $minor, $patch = $current.Split(".") | ForEach-Object { [int]$_ }

# Betas so far: version -> highest beta number.
$betas = @{}
foreach ($t in git tag --list "v*-beta.*") {
  if ($t -match '^v(\d+\.\d+\.\d+)-beta\.(\d+)$') {
    $n = [int]$Matches[2]
    if (-not $betas.ContainsKey($Matches[1]) -or $betas[$Matches[1]] -lt $n) { $betas[$Matches[1]] = $n }
  }
}
# The version whose betas are under way (newer than stable), if any.
$open = $betas.Keys | Where-Object { [version]$_ -gt [version]$current } | Sort-Object { [version]$_ } | Select-Object -Last 1

switch -Regex ($Bump) {
  # Each part stays a single digit: 0.3.9 -> 0.4.0, 0.9.9 -> 1.0.0.
  '^patch$' {
    if ($Beta -and $open) { $next = $open }   # the next beta of the version being tested
    elseif ($patch -lt 9) { $next = "$major.$minor.$($patch + 1)" }
    elseif ($minor -lt 9) { $next = "$major.$($minor + 1).0" }
    else { $next = "$($major + 1).0.0" }
  }
  '^minor$' { $next = if ($minor -lt 9) { "$major.$($minor + 1).0" } else { "$($major + 1).0.0" } }
  '^major$' { $next = "$($major + 1).0.0" }
  '^v?\d+\.\d+\.\d+$' { $next = $Bump.TrimStart("v") }
  default { throw "Use patch, minor, major or a version like 0.4.0 (got '$Bump')." }
}
if ([version]$next -le [version]$current) { throw "$next isn't newer than the current $current." }
if (git tag --list "v$next") { throw "$next is already released." }

if ($Beta) {
  $n = if ($betas.ContainsKey($next)) { $betas[$next] + 1 } else { 1 }
  if ($n -gt 98) { throw "$next has 98 betas already; release it, or start the next version." }
  $tag = "v$next-beta.$n"
  $label = "$next Beta $n"
} else {
  if ($betas.ContainsKey($next)) {
    throw "$next has betas. Release it with: scripts\promote.ps1 $next (it builds the last beta's commit)."
  }
  $tag = "v$next"
  $label = $next
}

$unpushed = (git rev-list --count "origin/$branch..HEAD").Trim()
Write-Host "Releasing B-Anime $label (stable is $current; $branch at $((git rev-parse --short HEAD).Trim()))"
if ($DryRun) { return }

# --- Tag and push -----------------------------------------------------------------------------
# Local commits not on GitHub yet go up first, so the tag points at something CI can build.
if ($unpushed -ne "0") { Run "git push $branch" { git push -q origin $branch } }
Run "git tag" { git tag $tag }
Run "git push tag" { git push -q origin $tag }

$repo = (git remote get-url origin).Trim() -replace '\.git$', '' -replace '^git@github\.com:', 'https://github.com/'
$page = if ($Beta) { "$repo-Beta" } else { $repo }
Write-Host "Pushed $tag. The installer and APK build on GitHub in about 12 minutes:"
Write-Host "  $page/releases/tag/$tag"
