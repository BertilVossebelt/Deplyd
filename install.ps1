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
$bundle = 'attestation.json'
$base = "https://github.com/$repo/releases/download/$tag"

Write-Host ''
Write-Host "deplyd $tag for $target" -ForegroundColor Cyan

# --- the GitHub CLI ---------------------------------------------------------

# Before the download rather than after it: gh is what checks the download, and a
# binary whose provenance cannot be checked is not one to install. Only the tool is
# wanted here. The check runs against a bundle published with the release, so it needs
# no account and no token, and signing in can wait until deplyd is on the disk.

Write-Host ''

$gh = Find-Gh
if (-not $gh) {
    $winget = Get-Command winget -CommandType Application -ErrorAction SilentlyContinue
    if ($winget -and (Confirm 'deplyd reads GitHub through the GitHub CLI, which is not installed. Install it?')) {
        Invoke-Native $winget.Source @('install', '--id', 'GitHub.cli', '-e', '--source', 'winget',
            '--accept-package-agreements', '--accept-source-agreements') | Out-Null
        $gh = Find-Gh
    }
}
if (-not $gh) {
    Fail "The GitHub CLI is needed, to check this download and to run deplyd.`nInstall it from https://cli.github.com, then run this again."
}

# gh learned to verify attestations in 2.49. An older one cannot check, which is not
# the same answer as a check that failed, so say which it is.
if ((Invoke-Native $gh @('attestation', 'verify', '--help')) -ne 0) {
    Fail 'This gh cannot check provenance - that arrived in 2.49. Update it, then run this again.'
}

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

    # Every release publishes SHA256SUMS, so a missing file or entry means this
    # download cannot be shown to be the published one. Refuse rather than install
    # it anyway. The fetch is the only part that throws, so only it is caught.
    $sumsPath = Join-Path $work 'SHA256SUMS'
    $sumsError = ''
    try {
        Invoke-WebRequest "$base/SHA256SUMS" -OutFile $sumsPath -UseBasicParsing
    } catch {
        $sumsError = $_.Exception.Message
    }
    if ($sumsError) {
        Fail "Could not fetch SHA256SUMS for $tag ($sumsError), so the download cannot be checked. Not installing."
    }

    $line = Get-Content $sumsPath |
        Where-Object { $_ -match [regex]::Escape($archive) + '$' } |
        Select-Object -First 1
    if (-not $line) { Fail "SHA256SUMS has no entry for $archive. Not installing." }

    $expected = ($line -split '\s+')[0]
    $actual = (Get-FileHash $archivePath -Algorithm SHA256).Hash.ToLower()
    if ($expected.ToLower() -ne $actual) { Fail 'Checksum mismatch. Not installing.' }
    Write-Host '  checksum   ok' -ForegroundColor DarkGray

    # Signed through Sigstore and recorded in a public log, so this says the binary
    # came from that repository's release workflow rather than somewhere else. The
    # bundle is published with the release, which is what makes this check cost
    # nothing to run: no account, no token, nothing to set up first.
    $bundlePath = Join-Path $work $bundle
    $haveBundle = $true
    try {
        Invoke-WebRequest "$base/$bundle" -OutFile $bundlePath -UseBasicParsing
    } catch {
        $haveBundle = $false
    }

    $badProvenance = "Provenance check failed: $archive is not what $repo's release workflow built. Not installing."
    if ($haveBundle) {
        if ((Invoke-Native $gh @('attestation', 'verify', $archivePath, '--repo', $repo,
            '--bundle', $bundlePath)) -ne 0) { Fail $badProvenance }
    } elseif ((Invoke-Native $gh @('auth', 'status')) -eq 0) {
        # The early releases published no bundle. Ask GitHub for the attestation
        # instead, which works but wants the sign-in those releases could assume.
        if ((Invoke-Native $gh @('attestation', 'verify', $archivePath, '--repo', $repo)) -ne 0) {
            Fail $badProvenance
        }
    } else {
        Fail "$tag published no attestation bundle, so checking it means asking GitHub.`nSign in with: gh auth login, or install the latest release, which carries its own."
    }
    Write-Host '  provenance ok' -ForegroundColor DarkGray

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

# --- signing in -------------------------------------------------------------

Write-Host ''

if ((Invoke-Native $gh @('auth', 'status')) -eq 0) {
    Write-Host '  github     signed in' -ForegroundColor DarkGray
} elseif (Confirm 'You are not signed in to GitHub. Sign in now?') {
    # Interactive on purpose: it is a device flow against access you already have.
    & $gh auth login
} else {
    Write-Host '  github     not signed in - run: gh auth login' -ForegroundColor Yellow
}

# --- completion -------------------------------------------------------------

$startMarker = '# >>> deplyd completions >>>'
$endMarker = '# <<< deplyd completions <<<'

# Anything the script defines, so a copy appended before it carried markers is still
# recognised rather than left behind next to a second one.
$unmarked = '^\s*(if \(-not \(Get-Command dp |\$script:Deplyd|Register-ArgumentCompleter -Native -CommandName .deplyd|function script:Deplyd)'

function Remove-DeplydBlock($path) {
    if (-not (Test-Path -LiteralPath $path)) { return @() }

    $kept = @()
    $inBlock = $false
    foreach ($line in @(Get-Content -LiteralPath $path)) {
        if ($line -eq $startMarker) { $inBlock = $true; continue }
        if ($line -eq $endMarker) { $inBlock = $false; continue }
        if ($inBlock) { continue }
        # A launcher line left by the PowerShell version, whose file is long gone.
        if ($line -match 'deplyd' -and $line -match 'shell-init\.ps1') { continue }
        $kept += $line
    }

    # An unmarked copy is removed whole: it runs to the end of the file, because
    # appending is the only way it got there.
    for ($i = 0; $i -lt $kept.Count; $i++) {
        if ($kept[$i] -match $unmarked) {
            $kept = if ($i -eq 0) { @() } else { @($kept | Select-Object -First $i) }
            break
        }
    }
    return $kept
}

try {
    # All hosts, so the VS Code terminal and the ISE get it too. Both files are
    # cleaned, in case an earlier install wrote to the other one.
    $profilePath = $PROFILE.CurrentUserAllHosts
    $otherPath = $PROFILE.CurrentUserCurrentHost

    if ($otherPath -ne $profilePath -and (Test-Path -LiteralPath $otherPath)) {
        Set-Content -LiteralPath $otherPath -Value @(Remove-DeplydBlock $otherPath) -Encoding utf8
    }

    # @() around both: a single surviving line comes back as a string, and adding an
    # array to a string concatenates instead of appending, collapsing the file.
    $kept = @(Remove-DeplydBlock $profilePath)
    New-Item -ItemType Directory -Path (Split-Path $profilePath) -Force | Out-Null

    $block = @(& (Join-Path $installDir 'deplyd.exe') completions powershell)
    Set-Content -LiteralPath $profilePath -Value ($kept + $block) -Encoding utf8

    Write-Host "  completion added to $profilePath" -ForegroundColor DarkGray
} catch {
    Write-Host '  completion could not be set up - run:' -ForegroundColor Yellow
    Write-Host '    deplyd completions powershell >> $PROFILE.CurrentUserAllHosts' -ForegroundColor Yellow
}

# --- done -------------------------------------------------------------------

Write-Host ''
Write-Host 'Done. Open a new terminal, then from inside any repo:' -ForegroundColor Green
Write-Host '  dp status'
Write-Host ''
