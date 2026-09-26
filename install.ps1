# Installs deplyd on Windows.
#
#   irm https://raw.githubusercontent.com/BertilVossebelt/deplyd/main/install.ps1 | iex
#
# Downloads the release for your platform, checks it against the published
# checksums, verifies its provenance when gh is available, and puts the binary on
# your PATH. Nothing needs administrator rights: it installs for you alone.
#
#   $env:DEPLYD_INSTALL_DIR   where to put it
#   $env:DEPLYD_VERSION       a tag to install, default the latest release

$ErrorActionPreference = 'Stop'

$repo = 'BertilVossebelt/deplyd'
$installDir = if ($env:DEPLYD_INSTALL_DIR) {
    $env:DEPLYD_INSTALL_DIR
} else {
    Join-Path $env:LOCALAPPDATA 'Programs\deplyd'
}

# A non-zero exit from gh is an answer here, not a failure. pwsh 7.4 and later turn
# one into a terminating error under the Stop preference, which would end the
# installer instead of falling through to "could not be verified".
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

    # deplyd needs gh anyway, so this costs nobody an extra tool. Signed through
    # Sigstore and recorded in a public log, so it says the binary came from that
    # repository's release workflow rather than from somewhere else.
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
    Copy-Item $binary (Join-Path $installDir 'deplyd.exe') -Force

    Write-Host "  installed  $installDir\deplyd.exe" -ForegroundColor DarkGray

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
            New-Item -ItemType HardLink -Path $alias -Value (Join-Path $installDir 'deplyd.exe') -ErrorAction Stop | Out-Null
        } catch {
            Copy-Item (Join-Path $installDir 'deplyd.exe') $alias -Force
        }
        Write-Host "  dp         short name for deplyd" -ForegroundColor DarkGray
    }
} finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}

# --- is it reachable --------------------------------------------------------

Write-Host ''
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -split ';' -contains $installDir) {
    Write-Host "Run 'deplyd check' to see what it is allowed to do."
} else {
    # Your PATH only, never the machine's, so this needs no administrator and
    # affects nobody else who uses this computer.
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$installDir", 'User')
    $env:Path = "$env:Path;$installDir"
    Write-Host "Added $installDir to your PATH."
    Write-Host 'Open a new terminal, then run: deplyd check'
}

Write-Host ''
Write-Host 'deplyd reads GitHub as you. If you have not already:  gh auth login' -ForegroundColor DarkGray
