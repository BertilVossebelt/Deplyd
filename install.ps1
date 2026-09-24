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
    param([string] $Question)

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

    try {
        $answer = Read-Host "$Question [y/N]"
    } catch {
        return $false
    }
    return ($answer -match '^(y|yes)$')
}

$gh = Get-Command gh -ErrorAction SilentlyContinue

if (-not $gh) {
    Write-Host ''
    Write-Host 'GitHub CLI (gh) is required, and is not installed.' -ForegroundColor Yellow

    if ((Test-IsWindows) -and (Get-Command winget -ErrorAction SilentlyContinue)) {
        $installer = 'winget'
    } elseif (Get-Command brew -ErrorAction SilentlyContinue) {
        $installer = 'brew'
    } else {
        $installer = ''
    }

    if ($installer -and (Confirm-Action "Install it now with ${installer}?")) {
        if ($installer -eq 'winget') {
            winget install --id GitHub.cli --exact --source winget
        } else {
            brew install gh
        }
        $gh = Get-Command gh -ErrorAction SilentlyContinue
        if (-not $gh) {
            Write-Host 'Installed, but not on PATH yet. Reopen the terminal and run this installer again.' -ForegroundColor Yellow
        }
    } else {
        Write-Host 'Install it from https://github.com/cli/cli#installation' -ForegroundColor DarkGray
    }
}

if ($gh) {
    Write-Host ''
    Write-Host 'Checking GitHub sign-in...' -ForegroundColor DarkGray
    gh auth status
    if ($LASTEXITCODE -ne 0) {
        Write-Host ''
        Write-Host 'Not signed in to GitHub.' -ForegroundColor Yellow
        if (Confirm-Action 'Run gh auth login now?') {
            gh auth login
        } else {
            Write-Host 'Sign in later with: gh auth login' -ForegroundColor DarkGray
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
Write-Host ''
Write-Host 'Then, from inside any repo:' -ForegroundColor Cyan
Write-Host '  deplyd help'
Write-Host '  deplyd config'
Write-Host '  deplyd'
Write-Host '  deplyd pr 1234'
Write-Host ''
Write-Host 'Uninstall with: ./uninstall.ps1' -ForegroundColor DarkGray
