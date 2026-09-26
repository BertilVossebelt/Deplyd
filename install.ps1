# Installs deplyd on Windows: downloads the release, checks it, puts deplyd and dp
# on your PATH, installs the GitHub CLI if it is missing, signs you in, and turns on
# completion. Nothing needs administrator rights.
#
#   irm https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.ps1 | iex
#
# iex cannot pass a switch, so uninstalling goes through a block, or the variable:
#
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.ps1))) -Uninstall
#   $env:DEPLYD_UNINSTALL = 1; irm https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.ps1 | iex
#
#   -Uninstall                remove what the installer put there, and nothing else
#   -Purge                    with -Uninstall, also remove settings and cache
#
#   $env:DEPLYD_INSTALL_DIR   where to put it
#   $env:DEPLYD_VERSION       a tag to install, default the latest release
#   $env:DEPLYD_YES           answer yes to every question, for unattended installs
#   $env:DEPLYD_UNINSTALL     uninstall, for when a switch cannot be passed
#   $env:DEPLYD_PURGE         with the above, also remove settings and cache
#   $env:DEPLYD_REMOVE_GH     remove the GitHub CLI too, for an unattended uninstall

param(
    [switch] $Uninstall,
    [switch] $Purge
)

$ErrorActionPreference = 'Stop'

if ($env:DEPLYD_UNINSTALL) { $Uninstall = $true }
if ($env:DEPLYD_PURGE) { $Uninstall = $true; $Purge = $true }

$repo = 'BertilVossebelt/Deplyd'
$installDir = if ($env:DEPLYD_INSTALL_DIR) {
    $env:DEPLYD_INSTALL_DIR
} else {
    Join-Path $env:LOCALAPPDATA 'Programs\deplyd'
}

# A non-zero exit is an answer here, not a failure. pwsh 7.4 turns one into a
# terminating error under the Stop preference, which would end the installer.
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

# Yes takes something away here, so DEPLYD_YES does not answer it.
function Confirm-No($question) {
    # No terminal to ask at is an answer: leave the thing alone.
    try { $answer = Read-Host "$question [y/N]" } catch { return $false }
    return ($answer -match '^[Yy]')
}

# A terminal keeps the PATH it started with, so a gh installed since is invisible.
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

# --- what this installer writes to a profile --------------------------------

$startMarker = '# >>> deplyd completions >>>'
$endMarker = '# <<< deplyd completions <<<'

# Anything the script defines, so a copy predating the markers is still recognised.
$unmarked = '^\s*(if \(-not \(Get-Command dp |\$script:Deplyd|Register-ArgumentCompleter -Native -CommandName .deplyd|function script:Deplyd)'

function Remove-DeplydBlock($path) {
    if (-not (Test-Path -LiteralPath $path)) { return @() }

    $kept = @()
    $inBlock = $false
    foreach ($line in @(Get-Content -LiteralPath $path)) {
        if ($line -eq $startMarker) { $inBlock = $true; continue }
        if ($line -eq $endMarker) { $inBlock = $false; continue }
        if ($inBlock) { continue }
        if ($line -match 'deplyd' -and $line -match 'shell-init\.ps1') { continue }
        $kept += $line
    }

    # An unmarked copy runs to the end of the file, appending being how it got there.
    for ($i = 0; $i -lt $kept.Count; $i++) {
        if ($kept[$i] -match $unmarked) {
            $kept = if ($i -eq 0) { @() } else { @($kept | Select-Object -First $i) }
            break
        }
    }
    return $kept
}


# --- uninstalling -----------------------------------------------------------

function Remove-Completions {
    $paths = @($PROFILE.CurrentUserAllHosts, $PROFILE.CurrentUserCurrentHost) |
        Select-Object -Unique
    foreach ($path in $paths) {
        if (Test-Path -LiteralPath $path) {
            # @() so a single surviving line stays a line rather than becoming a
            # string the file is then set to.
            Set-Content -LiteralPath $path -Value @(Remove-DeplydBlock $path) -Encoding utf8
            Write-Host "  profile    tidied: $path" -ForegroundColor DarkGray
        }
    }
}

# Your PATH only, and only the entry this installer added.
function Remove-FromPath {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $userPath) { return }

    $kept = @($userPath -split ';' | Where-Object { $_ -ne $installDir })
    if ($kept.Count -ne ($userPath -split ';').Count) {
        [Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User')
        Write-Host '  path       entry removed for your account' -ForegroundColor DarkGray
    }
}

function Remove-Gh {
    $gh = Find-Gh
    if (-not $gh) { return }

    if (-not $env:DEPLYD_REMOVE_GH) {
        if (-not (Confirm-No 'Remove the GitHub CLI as well? Other things may be using it.')) {
            Write-Host '  gh         kept' -ForegroundColor DarkGray
            return
        }
    }

    $winget = Get-Command winget -CommandType Application -ErrorAction SilentlyContinue
    if (-not $winget) {
        Write-Host '  gh         left alone - it came from somewhere this cannot undo' -ForegroundColor Yellow
        return
    }
    # gh is not ours, so winget saying no does not stop the uninstall.
    $code = Invoke-Native $winget.Source @('uninstall', '--id', 'GitHub.cli', '-e',
        '--accept-source-agreements')
    if ($code -ne 0) {
        Write-Host '  gh         still here - remove it the way it was installed' -ForegroundColor Yellow
        return
    }

    Write-Host '  gh         removed - its sign-in is still in ~\AppData\Roaming\GitHub CLI' -ForegroundColor DarkGray
}

function Invoke-Uninstall {
    Write-Host ''
    Write-Host 'Removing deplyd' -ForegroundColor Cyan
    Write-Host ''

    $binary = Join-Path $installDir 'deplyd.exe'
    $alias = Join-Path $installDir 'dp.exe'

    # Ours only if this installer made it: a hard link to the binary, or a copy.
    if (Test-Path -LiteralPath $alias) {
        $ours = $false
        if (Test-Path -LiteralPath $binary) {
            $ours = (Get-FileHash $alias -Algorithm SHA256).Hash -eq
                (Get-FileHash $binary -Algorithm SHA256).Hash
        }
        if ($ours) {
            Remove-Item -LiteralPath $alias -Force
            Write-Host '  dp         removed' -ForegroundColor DarkGray
        } else {
            Write-Host '  dp         left alone, it is not the one this installer made' -ForegroundColor Yellow
        }
    }

    if (Test-Path -LiteralPath $binary) {
        Remove-Item -LiteralPath $binary -Force
        Write-Host "  deplyd     removed from $installDir" -ForegroundColor DarkGray
    } else {
        Write-Host "  deplyd     was not in $installDir" -ForegroundColor DarkGray
    }

    # Unlike ~/.local/bin, the default here is deplyd's own, so an empty one goes.
    $default = Join-Path $env:LOCALAPPDATA 'Programs\deplyd'
    if ($installDir -eq $default -and (Test-Path -LiteralPath $installDir)) {
        if (-not @(Get-ChildItem -LiteralPath $installDir -Force)) {
            Remove-Item -LiteralPath $installDir -Force
            Write-Host '  folder     removed, it was empty' -ForegroundColor DarkGray
        }
    }

    Remove-Completions
    Remove-FromPath

    $config = Join-Path $env:APPDATA 'deplyd'
    if ($Purge) {
        if (Test-Path -LiteralPath $config) {
            Remove-Item -LiteralPath $config -Recurse -Force
            Write-Host "  settings   removed from $config" -ForegroundColor DarkGray
        }
    } elseif (Test-Path -LiteralPath $config) {
        Write-Host "  settings   kept in $config, -Purge removes them" -ForegroundColor DarkGray
    }

    Write-Host '  repos      any .deplyd.json left where it is' -ForegroundColor DarkGray

    Remove-Gh

    Write-Host ''
    Write-Host 'Done. Open a new terminal.' -ForegroundColor Green
    Write-Host ''
}

if ($Uninstall) {
    Invoke-Uninstall
    return
}

# dev-install.ps1 wants these functions and none of the work below.
if ($env:DEPLYD_SOURCE_ONLY) { return }

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

# Before the download because gh is what checks it. Only the tool is wanted here:
# the check reads a bundle published with the release, so signing in can wait.

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

# Verifying arrived in gh 2.49. Cannot check is not the same as check failed.
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

    # Every release publishes SHA256SUMS, so anything missing means this cannot be
    # shown to be the published download. Only the fetch throws, so only it is caught.
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

    # Sigstore, recorded in a public log: this says the binary came from that repo's
    # release workflow. The bundle ships with the release, so it needs no account.
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
        # The early releases published no bundle, so ask the API, which wants a sign-in.
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

    # A hard link costs no disk and needs no administrator, unlike a symlink.
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

# Your PATH only, never the machine's, so this needs no administrator.
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

try {
    # All hosts, so the VS Code terminal gets it. Both cleaned, in case of an
    # earlier install writing to the other.
    $profilePath = $PROFILE.CurrentUserAllHosts
    $otherPath = $PROFILE.CurrentUserCurrentHost

    if ($otherPath -ne $profilePath -and (Test-Path -LiteralPath $otherPath)) {
        Set-Content -LiteralPath $otherPath -Value @(Remove-DeplydBlock $otherPath) -Encoding utf8
    }

    # @() around both: one surviving line comes back as a string, and adding an
    # array to a string concatenates, collapsing the file.
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
