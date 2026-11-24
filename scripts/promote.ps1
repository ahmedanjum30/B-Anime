<#
.SYNOPSIS
  Makes a beta stable: turns its GitHub pre-release into the latest release, so every copy of
  B-Anime gets it, moves main up to it (main is what stable users run), and has the Promote
  workflow move the website's downloads to it. The same build beta users ran: nothing is
  rebuilt. Needs the GitHub CLI (gh), signed in.

.EXAMPLE
  scripts\promote.ps1 0.4.3
#>
param(
  [Parameter(Mandatory = $true)][string]$Version
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Run([string]$what, [scriptblock]$cmd) {
  & $cmd
  if ($LASTEXITCODE -ne 0) { throw "$what failed (exit code $LASTEXITCODE)." }
}

$Version = $Version.TrimStart("v")
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Use a version like 0.4.3 (got '$Version')." }
$tag = "v$Version"
$repo = ((git remote get-url origin).Trim() -replace '\.git$', '' -replace '^git@github\.com:', 'https://github.com/') -replace '^https://github\.com/', ''

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { throw "The GitHub CLI (gh) isn't installed." }
$info = gh release view $tag -R $repo --json isPrerelease,isDraft,name | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw "There's no release $tag yet (it may still be building)." }
if ($info.isDraft) { throw "$tag is still a draft." }
if (-not $info.isPrerelease) { throw "$tag is already stable." }

# main moves up to the beta's commit. Only forward: if main has a fix the beta lacks, merge main
# into beta, release a new beta, and promote that.
Run "git fetch" { git fetch -q origin main --tags }
$commit = (git rev-list -n 1 $tag).Trim()
git merge-base --is-ancestor origin/main $commit
if ($LASTEXITCODE -ne 0) {
  throw "main has commits that $tag doesn't. Merge main into beta, release a new beta, and promote that one."
}

Write-Host "Promoting $tag to stable"
Run "gh release edit" { gh release edit $tag -R $repo --prerelease=false --latest --title "B-Anime $Version" }
Run "git push main" { git push -q origin "${commit}:refs/heads/main" }
Run "gh workflow run" { gh workflow run promote.yml -R $repo --ref main -f tag=$tag }
# A local main, if checked out elsewhere, catches up on its next pull.
Write-Host "Done. main is at $tag, every copy of B-Anime is offered $Version now, and the website's downloads follow in a minute:"
Write-Host "  https://github.com/$repo/releases/tag/$tag"
