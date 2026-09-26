# Installs deplyd on Windows.
#
#   irm https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.ps1 | iex
#
# Downloads the release, checks it, puts deplyd and dp on your PATH, installs the
# GitHub CLI if it is missing, signs you in if you are not, and turns on completion.
# Nothing needs administrator rights.
#
#   $env:DEPLYD_INSTALL_DIR   where to put it
#   $env:DEPLYD_VERSION       a tag to install, default the latest release
#   $env:DEPLYD_YES           answer yes to every question, for unattended installs

$ErrorActionPreference = 'Stop'

$repo = 'BertilVossebelt/Deplyd'
$installDir = if ($env:DEPLYD_INSTALL_DIR) {
    $env:DEPLYD_INSTALL_DIR
} else {
    Join-Path $env:LOCALAPPDATA 'Programs\deplyd'
}

# A non-zero exit from a tool is an answer here, not a failure. pwsh 7.4 and later turn
# one into a terminating error under the Stop preference, which would end the installer.
function Invoke-Native {
    param([string] $Path, [string[]] $Arguments)

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $PSNativeCommandUseErrorActionPreference = $false
    try {
        & $Path @Arguments 2>&1 | Out-Null
        return $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
    }
}

function Confirm($question) {
    if ($env:DEPLYD_YES) { return $true }
    $answer = Read-Host "$question [Y/n]"
    return ($answer -eq '' -or $answer -match '^[Yy]')
}

# A terminal keeps the PATH it was started with, so a gh installed since it opened is
# invisible until it is reopened. After PATH, look where the installers put it.
function Find-Gh {
    $onPath = Get-Command gh -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($onPath) { return $onPath.Source }

    foreach ($base in @($env:ProgramFiles, ${env:ProgramFiles(x86)}, (Join-Path $env:LOCALAPPDATA 'Programs'))) {
        if ($base) {
            $candidate = Join-Path $base 'GitHub CLI\gh.exe'
            if (Test-Path -LiteralPath $candidate) { return $candidate }
        }
    }
    return ''
}

function Fail($message) {
    Write-Host ''
    Write-Host $message -ForegroundColor Red
    Write-Host ''
    exit 1
}

# --- what are we running on -------------------------------------------------

$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { 'x86_64' }
    'ARM64' { 'aarch64' }
    default { Fail "deplyd has no build for $($env:PROCESSOR_ARCHITECTURE)." }
}
$target = "$arch-pc-windows-msvc"

# --- which release ----------------------------------------------------------

if ($env:DEPLYD_VERSION) {
    $tag = $env:DEPLYD_VERSION
} else {
    try {
        $latest = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
        $tag = $latest.tag_name
    } catch {
        Fail "Could not work out the latest release. Set DEPLYD_VERSION to a tag."
    }
}

$archive = "deplyd-$tag-$target.zip"
$base = "https://github.com/$repo/releases/download/$tag"

Write-Host ''
Write-Host "deplyd $tag for $target" -ForegroundColor Cyan

# --- download ---------------------------------------------------------------

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("deplyd-install-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $work -Force | Out-Null

try {
    $archivePath = Join-Path $work $archive
    try {
        Invoke-WebRequest "$base/$archive" -OutFile $archivePath -UseBasicParsing
    } catch {
        Fail "No build for $target in $tag. See https://github.com/$repo/releases"
    }

    # --- check it is what was published ------------------------------------

    try {
        $sumsPath = Join-Path $work 'SHA256SUMS'
        Invoke-WebRequest "$base/SHA256SUMS" -OutFile $sumsPath -UseBasicParsing
        $line = Get-Content $sumsPath | Where-Object { $_ -match [regex]::Escape($archive) + '$' }
        if ($line) {
            $expected = ($line -split '\s+')[0]
            $actual = (Get-FileHash $archivePath -Algorithm SHA256).Hash.ToLower()
            if ($expected.ToLower() -ne $actual) {
                Fail 'Checksum mismatch. Not installing.'
            }
            Write-Host '  checksum   ok' -ForegroundColor DarkGray
        }
    } catch {
        Write-Host '  checksum   could not be checked' -ForegroundColor Yellow
    }

    # Signed through Sigstore and recorded in a public log, so this says the binary
    # came from that repository's release workflow rather than somewhere else.
    $gh = Find-Gh
    if ($gh) {
        $code = Invoke-Native $gh @('attestation', 'verify', $archivePath, '--repo', $repo)
        if ($code -eq 0) {
            Write-Host '  provenance ok' -ForegroundColor DarkGray
        } else {
            Write-Host '  provenance could not be verified - continuing, but be aware' -ForegroundColor Yellow
        }
    }

    # --- install -----------------------------------------------------------

    Expand-Archive -Path $archivePath -DestinationPath $work -Force
    $binary = Join-Path $work 'deplyd.exe'
    if (-not (Test-Path $binary)) { Fail 'The archive did not contain deplyd.exe.' }

    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    $installed = Join-Path $installDir 'deplyd.exe'
    Copy-Item $binary $installed -Force

    Write-Host "  installed  $installed" -ForegroundColor DarkGray

    # dp is the short name. A hard link costs no disk and needs no administrator on
    # NTFS, unlike a symlink. Someone else's dp keeps the name.
    $alias = Join-Path $installDir 'dp.exe'
    $existing = Get-Command dp -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($existing -and $existing.Source -ne $alias) {
        Write-Host "  dp         taken by $($existing.Source), skipped" -ForegroundColor Yellow
    } else {
        Remove-Item $alias -Force -ErrorAction SilentlyContinue
        try {
            New-Item -ItemType HardLink -Path $alias -Value $installed -ErrorAction Stop | Out-Null
        } catch {
            Copy-Item $installed $alias -Force
        }
        Write-Host '  dp         short name for deplyd' -ForegroundColor DarkGray
    }
} finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}

# --- PATH -------------------------------------------------------------------

# Your PATH only, never the machine's, so this needs no administrator and affects
# nobody else who uses this computer.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $installDir) {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$installDir", 'User')
    Write-Host '  path       added for your account' -ForegroundColor DarkGray
}
$env:Path = "$env:Path;$installDir"

# --- the GitHub CLI ---------------------------------------------------------

Write-Host ''

$gh = Find-Gh
if (-not $gh) {
    $winget = Get-Command winget -CommandType Application -ErrorAction SilentlyContinue
    if ($winget) {
        if (Confirm 'deplyd reads GitHub through the GitHub CLI, which is not installed. Install it?') {
            Invoke-Native $winget.Source @('install', '--id', 'GitHub.cli', '-e', '--source', 'winget',
                '--accept-package-agreements', '--accept-source-agreements') | Out-Null
            $gh = Find-Gh
        }
    }
    if (-not $gh) {
        Write-Host 'The GitHub CLI is needed. Install it, then run: gh auth login' -ForegroundColor Yellow
        Write-Host '  https://cli.github.com' -ForegroundColor DarkGray
    }
}

if ($gh) {
    if ((Invoke-Native $gh @('auth', 'status')) -eq 0) {
        Write-Host '  github     signed in' -ForegroundColor DarkGray
    } elseif (Confirm 'You are not signed in to GitHub. Sign in now?') {
        # Interactive on purpose: it is a device flow against access you already have.
        & $gh auth login
    } else {
        Write-Host '  github     not signed in - run: gh auth login' -ForegroundColor Yellow
    }
}

# --- completion -------------------------------------------------------------

$startMarker = '# >>> deplyd completions >>>'
$endMarker = '# <<< deplyd completions <<<'

try {
    $profilePath = $PROFILE.CurrentUserAllHosts
    New-Item -ItemType Directory -Path (Split-Path $profilePath) -Force | Out-Null

    $lines = if (Test-Path -LiteralPath $profilePath) {
        @(Get-Content -LiteralPath $profilePath)
    } else {
        @()
    }

    # Drop our own previous block, and any launcher line left by the PowerShell
    # version of deplyd, whose file no longer exists.
    $kept = @()
    $inBlock = $false
    foreach ($line in $lines) {
        if ($line -eq $startMarker) { $inBlock = $true; continue }
        if ($line -eq $endMarker) { $inBlock = $false; continue }
        if ($inBlock) { continue }
        if ($line -match 'deplyd' -and $line -match 'shell-init\.ps1') { continue }
        $kept += $line
    }

    $block = @($startMarker) + @(& (Join-Path $installDir 'deplyd.exe') completions powershell) + @($endMarker)
    Set-Content -LiteralPath $profilePath -Value ($kept + $block) -Encoding utf8

    Write-Host '  completion added to your PowerShell profile' -ForegroundColor DarkGray
} catch {
    Write-Host '  completion could not be set up - run: deplyd completions powershell >> $PROFILE' -ForegroundColor Yellow
}

# --- done -------------------------------------------------------------------

Write-Host ''
Write-Host 'Done. Open a new terminal, then from inside any repo:' -ForegroundColor Green
Write-Host '  dp status'
Write-Host ''
