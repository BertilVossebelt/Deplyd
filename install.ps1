#Requires -Version 5.1
<#
    Adds deplyd to the PowerShell profile as a function, so it can be run from any
    directory by name. Removes it again with -Uninstall.
#>
[CmdletBinding()]
param(
    [switch] $Uninstall
)

$ErrorActionPreference = 'Stop'

function Test-IsWindows {
    if ($null -ne $PSVersionTable.Platform) { return ($PSVersionTable.Platform -eq 'Win32NT') }
    return $true
}

$marker = '# deplyd launcher'
$target = Join-Path $PSScriptRoot 'deplyd.ps1'
$shellInit = Join-Path $PSScriptRoot 'shell-init.ps1'

if (-not $Uninstall) {
    # A launcher pointing at a half-copied directory fails later with no explanation.
    foreach ($required in @($target, $shellInit, (Join-Path $PSScriptRoot 'lib'), (Join-Path $PSScriptRoot 'commands'))) {
        if (-not (Test-Path $required)) {
            Write-Host "Missing: $required" -ForegroundColor Red
            Write-Host 'Run this from a full copy of the repository.' -ForegroundColor DarkGray
            exit 1
        }
    }
}

$profileDirectory = Split-Path $PROFILE
if (-not (Test-Path $profileDirectory)) {
    New-Item -ItemType Directory -Path $profileDirectory -Force | Out-Null
}
if (-not (Test-Path $PROFILE)) {
    New-Item -ItemType File -Path $PROFILE | Out-Null
}

$existing = @(Get-Content -Path $PROFILE -ErrorAction SilentlyContinue)
$kept = @($existing | Where-Object { $_ -notmatch [regex]::Escape($marker) })
$removed = $existing.Count - $kept.Count

if ($Uninstall) {
    Set-Content -Path $PROFILE -Value $kept -Encoding utf8
    if ($removed -gt 0) {
        Write-Host "Removed $removed launcher line(s) from $PROFILE" -ForegroundColor Green
        Write-Host 'Reopen your terminals for it to take effect.' -ForegroundColor DarkGray
    } else {
        Write-Host 'Nothing to remove.' -ForegroundColor DarkGray
    }
    return
}

# Dot-sourced rather than defined inline, so the wrapper can declare the parameters
# that make tab completion possible.
$line = ". '" + $shellInit + "' " + $marker
Set-Content -Path $PROFILE -Value ($kept + $line) -Encoding utf8

# Run normally, the profile would load into a scope that disappears on exit, so
# claiming the command is ready would be untrue.
$loadedHere = ($MyInvocation.InvocationName -eq '.')

if ($removed -gt 0) {
    Write-Host "Replaced the previous launcher in $PROFILE" -ForegroundColor DarkGray
}
Write-Host "Installed 'deplyd' -> $target" -ForegroundColor Green

if ($loadedHere) { . $PROFILE }

function Confirm-Action {
    param([string] $Question, [switch] $DefaultYes)

    # Asking a question nobody can answer throws or hangs. Take silence as no.
    $interactive = $true
    try {
        if ([Console]::IsInputRedirected) { $interactive = $false }
        if (-not [Environment]::UserInteractive) { $interactive = $false }
    } catch {
        $interactive = $false
    }

    if (-not $interactive) {
        Write-Host "$Question  (not an interactive session, skipping)" -ForegroundColor DarkGray
        return $false
    }

    $choices = if ($DefaultYes) { '[Y/n]' } else { '[y/N]' }
    try {
        $answer = Read-Host "$Question $choices"
    } catch {
        return $false
    }
    if ($DefaultYes -and [string]::IsNullOrWhiteSpace($answer)) { return $true }
    return ($answer -match '^(y|yes)$')
}

function Invoke-Native {
    param([string] $Path, [string[]] $Arguments)

    # A non-zero exit is an answer here (not signed in, already installed), not a
    # failure. pwsh 7.4 and later turn it into a terminating error under the Stop
    # preference, which ended this installer after "Installed" and before anyone was
    # asked to sign in.
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $PSNativeCommandUseErrorActionPreference = $false
    try {
        & $Path @Arguments
    } finally {
        $ErrorActionPreference = $previousPreference
    }
}

function Find-GhExecutable {
    # A terminal keeps the PATH it was started with, so a gh installed since then is
    # invisible to it until it is reopened; IDE terminals can lag further behind. That
    # made this installer say gh was missing, skip the sign-in, and leave the first
    # real run to fail. So after PATH, look where the installers put it.
    $onPath = Get-Command gh -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($onPath) { return $onPath.Source }

    $candidates = @()
    if (Test-IsWindows) {
        foreach ($base in @($env:ProgramFiles, ${env:ProgramFiles(x86)}, (Join-Path $env:LOCALAPPDATA 'Programs'))) {
            if ($base) { $candidates += (Join-Path $base 'GitHub CLI\gh.exe') }
        }
    } else {
        $candidates += @('/opt/homebrew/bin/gh', '/usr/local/bin/gh', '/home/linuxbrew/.linuxbrew/bin/gh', '/usr/bin/gh')
    }
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    return ''
}

function Update-PathFromRegistry {
    # An installer appends to the registry PATH; this session's copy predates that.
    if (-not (Test-IsWindows)) { return }
    $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $user = [Environment]::GetEnvironmentVariable('Path', 'User')
    $env:Path = (@($env:Path, $machine, $user) | Where-Object { $_ }) -join ';'
}

$ghPath = Find-GhExecutable

if (-not $ghPath) {
    Write-Host ''
    Write-Host 'GitHub CLI (gh) is required, and was not found.' -ForegroundColor Yellow

    if ((Test-IsWindows) -and (Get-Command winget -ErrorAction SilentlyContinue)) {
        $installer = 'winget'
    } elseif (Get-Command brew -ErrorAction SilentlyContinue) {
        $installer = 'brew'
    } else {
        $installer = ''
    }

    if ($installer -and (Confirm-Action "Install it now with ${installer}?" -DefaultYes)) {
        if ($installer -eq 'winget') {
            Invoke-Native 'winget' @('install', '--id', 'GitHub.cli', '--exact', '--source', 'winget')
        } else {
            Invoke-Native 'brew' @('install', 'gh')
        }
        Update-PathFromRegistry
        $ghPath = Find-GhExecutable
        if (-not $ghPath) {
            Write-Host 'Installed, but it cannot be found yet. Reopen the terminal and run this installer again.' -ForegroundColor Yellow
        }
    } else {
        Write-Host 'Install it from https://github.com/cli/cli#installation' -ForegroundColor DarkGray
    }
}

$signedIn = $false
if ($ghPath) {
    if (-not (Get-Command gh -CommandType Application -ErrorAction SilentlyContinue)) {
        # deplyd calls it by name, so this session needs to be able to as well.
        $env:Path = (Split-Path $ghPath -Parent) + [System.IO.Path]::PathSeparator + $env:Path
        Write-Host ''
        Write-Host "Found gh at $ghPath, which this terminal's PATH did not include." -ForegroundColor DarkGray
        Write-Host 'Added for this session. A terminal opened before gh was installed needs reopening.' -ForegroundColor DarkGray
    }

    Write-Host ''
    Write-Host 'Checking GitHub sign-in...' -ForegroundColor DarkGray
    Invoke-Native $ghPath @('auth', 'status')
    $signedIn = ($LASTEXITCODE -eq 0)

    if (-not $signedIn) {
        Write-Host ''
        Write-Host 'Not signed in to GitHub. deplyd cannot read anything until you are.' -ForegroundColor Yellow
        if (Confirm-Action 'Sign in now with gh auth login?' -DefaultYes) {
            Invoke-Native $ghPath @('auth', 'login')
            $signedIn = ($LASTEXITCODE -eq 0)
        }
    }
}

Write-Host ''
Write-Host ''
if ($loadedHere) {
    Write-Host 'Ready in this session. Other open terminals need to be reopened.' -ForegroundColor DarkGray
} else {
    Write-Host 'Open a new terminal to use it.' -ForegroundColor DarkGray
    Write-Host 'Dot-sourcing the installer next time (. ./install.ps1) skips that.' -ForegroundColor DarkGray
}

if (-not $signedIn) {
    # Said last and in colour: this is the step that was easiest to miss, and without
    # it the first run answers with nothing.
    Write-Host ''
    Write-Host 'Still to do before deplyd can read GitHub:' -ForegroundColor Yellow
    if (-not $ghPath) { Write-Host '  install the GitHub CLI: https://github.com/cli/cli#installation' -ForegroundColor Yellow }
    Write-Host '  gh auth login' -ForegroundColor Yellow
}

Write-Host ''
Write-Host 'Then, from inside any repo:' -ForegroundColor Cyan
Write-Host '  deplyd help'
Write-Host '  deplyd config'
Write-Host '  deplyd'
Write-Host '  deplyd pr 1234'
Write-Host ''
Write-Host 'Uninstall with: ./uninstall.ps1' -ForegroundColor DarkGray
