# Puts the build in your working tree where the installer would put it, so a change
# can be tried the way someone else will meet it: on PATH, as deplyd and dp, with
# completion wired up.
#
#   .\dev-install.ps1              build, then stage it
#   .\dev-install.ps1 -Release     the release profile, for a realistic binary
#   .\dev-install.ps1 -Revert      put the published release back
#
# This is not the installer and never will be. install.ps1 takes a published release
# and refuses anything it cannot verify; a local build is neither, so staging one is
# a separate job with a separate name. What it borrows is the tidying, so a dev
# install and a real one leave the same shape behind.

param(
    [switch] $Release,
    [switch] $Revert
)

$ErrorActionPreference = 'Stop'

$root = $PSScriptRoot

# Everything install.ps1 defines, and nothing it does.
$env:DEPLYD_SOURCE_ONLY = 1
try {
    . (Join-Path $root 'install.ps1')
} finally {
    $env:DEPLYD_SOURCE_ONLY = $null
}

if ($Revert) {
    Invoke-Uninstall
    Write-Host 'Now reinstall the published release:' -ForegroundColor Cyan
    Write-Host '  irm https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.ps1 | iex'
    Write-Host ''
    return
}

# --- build ------------------------------------------------------------------

$profileName = if ($Release) { 'release' } else { 'debug' }
Write-Host ''
Write-Host "Building deplyd ($profileName)" -ForegroundColor Cyan

$arguments = @('build')
if ($Release) { $arguments += '--release' }
& cargo @arguments
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$built = Join-Path $root "target\$profileName\deplyd.exe"
if (-not (Test-Path $built)) { throw "No binary at $built" }

# --- stage it ---------------------------------------------------------------

Write-Host ''
New-Item -ItemType Directory -Path $installDir -Force | Out-Null

$installed = Join-Path $installDir 'deplyd.exe'
$alias = Join-Path $installDir 'dp.exe'

# A running deplyd holds its own file open, so replacing it can fail with nothing
# obviously wrong. Say which process rather than leaving a locked-file error.
try {
    Copy-Item $built $installed -Force
} catch {
    $holding = Get-Process -Name 'deplyd', 'dp' -ErrorAction SilentlyContinue
    if ($holding) { throw "deplyd is running (pid $($holding.Id -join ', ')). Close it and run this again." }
    throw
}

Remove-Item $alias -Force -ErrorAction SilentlyContinue
try {
    New-Item -ItemType HardLink -Path $alias -Value $installed -ErrorAction Stop | Out-Null
} catch {
    Copy-Item $installed $alias -Force
}

Write-Host "  installed  $installed" -ForegroundColor DarkGray
Write-Host '  dp         short name for deplyd' -ForegroundColor DarkGray

# --- PATH and completion ----------------------------------------------------

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $installDir) {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$installDir", 'User')
    Write-Host '  path       added for your account' -ForegroundColor DarkGray
}
$env:Path = "$env:Path;$installDir"

$profilePath = $PROFILE.CurrentUserAllHosts
$kept = @(Remove-DeplydBlock $profilePath)
New-Item -ItemType Directory -Path (Split-Path $profilePath) -Force | Out-Null
$block = @(& $installed completions powershell)
Set-Content -LiteralPath $profilePath -Value ($kept + $block) -Encoding utf8
Write-Host "  completion added to $profilePath" -ForegroundColor DarkGray

Write-Host ''
Write-Host "Staged $(& $installed --version)." -ForegroundColor Green
Write-Host '  dp status                    try it'
Write-Host '  .\install.ps1 -Uninstall     test the uninstaller against it'
Write-Host '  .\dev-install.ps1 -Revert    take it back off'
Write-Host ''
