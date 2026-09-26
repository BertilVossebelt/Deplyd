# Builds the tree and puts that build on PATH for this terminal only. Nothing is
# written to your profile, your account's PATH, or where a real install lives.
# Dot-source it: a script cannot change the PATH of the shell that ran it.
#
#   . .\dev-install.ps1              build, then use it here
#   . .\dev-install.ps1 -Release     the release profile, for a realistic binary
#   . .\dev-install.ps1 -Persist     install it for real, to test the uninstaller
#   . .\dev-install.ps1 -Revert      undo a -Persist
#
# Not the installer: that one only takes a published release and refuses what it
# cannot verify.

param(
    [switch] $Release,
    [switch] $Persist,
    [switch] $Revert,

    # One dash in PowerShell, so --persist binds to nothing. Caught, not ignored.
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $Rest
)

# Dot-sourced, so this is the caller's session. Put back what was there.
$deplydPreviousPreference = $ErrorActionPreference
$deplydDotSourced = $MyInvocation.InvocationName -eq '.'

try {
    $ErrorActionPreference = 'Stop'
    $root = $PSScriptRoot

    if ($Rest) {
        Write-Host ''
        Write-Host "Unknown option: $($Rest -join ' ')" -ForegroundColor Red
        Write-Host '  This is PowerShell: -Release, -Persist, -Revert, one dash each.'
        Write-Host ''
        return
    }

    if (-not $deplydDotSourced -and -not ($Persist -or $Revert)) {
        Write-Host ''
        Write-Host 'Run this dot-sourced, or the PATH it sets dies with the script:' -ForegroundColor Yellow
        Write-Host '  . .\dev-install.ps1'
        Write-Host ''
        return
    }

    # --- the persistent kind, for working on the installer -----------------

    if ($Persist -or $Revert) {
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
    }

    # --- build -------------------------------------------------------------

    $profileName = if ($Release) { 'release' } else { 'debug' }
    Write-Host ''
    Write-Host "Building deplyd ($profileName)" -ForegroundColor Cyan

    $arguments = @('build')
    if ($Release) { $arguments += '--release' }
    Push-Location $root
    try { & cargo @arguments } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

    $built = Join-Path $root "target\$profileName\deplyd.exe"
    if (-not (Test-Path $built)) { throw "No binary at $built" }

    # Under target, so it is gitignored and cargo clean takes it.
    $destination = if ($Persist) { $installDir } else { Join-Path $root 'target\dev-bin' }
    New-Item -ItemType Directory -Path $destination -Force | Out-Null

    $exe = Join-Path $destination 'deplyd.exe'
    $alias = Join-Path $destination 'dp.exe'

    # A running deplyd holds its own file open. Name it, rather than a lock error.
    try {
        Copy-Item $built $exe -Force
    } catch {
        $holding = Get-Process -Name 'deplyd', 'dp' -ErrorAction SilentlyContinue
        if ($holding) { throw "deplyd is running (pid $($holding.Id -join ', ')). Close it and run this again." }
        throw
    }

    Remove-Item $alias -Force -ErrorAction SilentlyContinue
    try {
        New-Item -ItemType HardLink -Path $alias -Value $exe -ErrorAction Stop | Out-Null
    } catch {
        Copy-Item $exe $alias -Force
    }

    Write-Host ''
    Write-Host "  built      $exe" -ForegroundColor DarkGray
    Write-Host '  dp         short name for deplyd' -ForegroundColor DarkGray

    if ($Persist) {
        # --- the parts that outlive the terminal ---------------------------

        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if (($userPath -split ';') -notcontains $installDir) {
            [Environment]::SetEnvironmentVariable('Path', "$userPath;$installDir", 'User')
            Write-Host '  path       added for your account' -ForegroundColor DarkGray
        }
        $env:Path = "$env:Path;$installDir"

        $profilePath = $PROFILE.CurrentUserAllHosts
        $kept = @(Remove-DeplydBlock $profilePath)
        New-Item -ItemType Directory -Path (Split-Path $profilePath) -Force | Out-Null
        Set-Content -LiteralPath $profilePath -Value ($kept + @(& $exe completions powershell)) -Encoding utf8
        Write-Host "  completion added to $profilePath" -ForegroundColor DarkGray

        Write-Host ''
        Write-Host "Installed $(& $exe --version), and it will still be here tomorrow." -ForegroundColor Green
        Write-Host '  .\install.ps1 -Uninstall     test the uninstaller against it'
        Write-Host '  . .\dev-install.ps1 -Revert  take it back off'
        Write-Host ''
        return
    }

    # --- this terminal only -------------------------------------------------

    # Earlier runs dropped first, so sourcing twice does not stack up.
    $env:Path = (@($destination) + @($env:Path -split ';' | Where-Object { $_ -ne $destination })) -join ';'

    # Register-ArgumentCompleter reaches the session, so no profile is touched.
    & $exe completions powershell | Out-String | Invoke-Expression

    Write-Host ''
    Write-Host "Using $(& $exe --version) in this terminal only." -ForegroundColor Green
    Write-Host '  dp status       try it'
    Write-Host '  close this terminal and nothing of it is left'
    Write-Host ''
} finally {
    $ErrorActionPreference = $deplydPreviousPreference
}
