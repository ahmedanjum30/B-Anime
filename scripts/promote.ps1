<#
.SYNOPSIS
  Releases a version whose betas are good: tags vX.Y.Z on its last beta's commit and moves main
  up to it, so the stable build is exactly the code beta users tested (built again, as X.Y.Z).
  Every copy of B-Anime then gets it, and the website's downloads move to it.

.EXAMPLE
  scripts\promote.ps1 0.4.5          # after 0.4.5-beta.1, -beta.2, ... : releases 0.4.5
  scripts\promote.ps1 0.4.5 -DryRun  # show what it would do
#>
param(
  [Parameter(Mandatory = $true)][string]$Version,
  [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Run([string]$what, [scriptblock]$cmd) {
  & $cmd
  if ($LASTEXITCODE -ne 0) { throw "$what failed (exit code $LASTEXITCODE)." }
}

$Version = $Version.TrimStart("v")
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Use a version like 0.4.5 (got '$Version')." }
$tag = "v$Version"

Run "git fetch" { git fetch -q --tags --force origin main beta }
if (git tag --list $tag) { throw "$Version is already released." }

# Its last beta.
$last = git tag --list "v$Version-beta.*" | Where-Object { $_ -match '-beta\.(\d+)$' } |
  Sort-Object { [int]($_ -replace '.*-beta\.', '') } | Select-Object -Last 1
if (-not $last) { throw "$Version has no betas. Release a beta first (scripts\release.ps1 $Version -Beta), or a stable one from main." }
$commit = (git rev-list -n 1 $last).Trim()

# main only moves forward. If main has a fix the beta lacks: merge main into beta, release
# another beta, and promote after that one.
git merge-base --is-ancestor origin/main $commit
if ($LASTEXITCODE -ne 0) {
  throw "main has commits that $last doesn't. Merge main into beta, release another beta, and promote after that."
}
$newer = (git rev-list --count "$commit..origin/beta").Trim()
if ($newer -ne "0") {
  Write-Host "Note: beta has $newer commit(s) after $last; they're not in $Version (they go in a later beta)."
}

Write-Host "Releasing B-Anime $Version from $last ($(git rev-parse --short $commit))"
if ($DryRun) { return }

Run "git push main" { git push -q origin "${commit}:refs/heads/main" }
Run "git tag" { git tag $tag $commit }
Run "git push tag" { git push -q origin $tag }

$repo = (git remote get-url origin).Trim() -replace '\.git$', '' -replace '^git@github\.com:', 'https://github.com/'
Write-Host "Done: main is at $last, and $tag builds on GitHub in about 12 minutes:"
Write-Host "  $repo/releases/tag/$tag"
