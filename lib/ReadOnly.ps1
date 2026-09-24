<#
    Read-only gateway. Every git and gh invocation in deplyd goes through
    Invoke-ReadOnly, and Test-SourceIsReadOnly refuses to run if any file calls
    them directly. Loaded first, before anything else happens.
#>

function Stop-WithMessage {
    param([string] $Message, [string[]] $Hints = @())
    Write-Host ''
    Write-Host $Message -ForegroundColor Red
    if ($Hints.Count -gt 0) { Write-Host '' }
    foreach ($hint in $Hints) { Write-Host "  $hint" -ForegroundColor DarkGray }
    Write-Host ''
    exit 1
}

# region read-only gateway
# Every git and gh invocation in this script goes through Invoke-ReadOnly. Anything not
# on these allowlists is refused before it runs, and Test-SourceIsReadOnly below fails
# the script if a call is ever added that bypasses this gateway.

$script:allowedGitVerbs = @(
    'rev-parse', 'rev-list', 'log', 'show', 'merge-base', 'shortlog', 'cat-file', 'diff', 'status', 'fetch', 'config', 'cherry',
    'ls-files'
)

$script:allowedGhVerbs = @{
    'api' = @()
    'run' = @('list', 'view')
    'pr'  = @('list', 'view')
}

function Deny-Command {
    param([string] $Command, [string[]] $Arguments, [string] $Reason)
    Stop-WithMessage -Message 'Refused: deplyd only performs read operations.' -Hints @(
        "Blocked: $Command $($Arguments -join ' ')",
        $Reason,
        'This is a deliberate guard. If you need this, it does not belong in deplyd.'
    )
}

function Assert-ReadOnly {
    param([string] $Command, [string[]] $Arguments)

    $positional = @($Arguments | Where-Object { $_ -notlike '-*' })

    if ($Command -eq 'git') {
        if ($positional.Count -eq 0) { Deny-Command $Command $Arguments 'No git subcommand given.' }
        $verb = $positional[0]
        if ($script:allowedGitVerbs -notcontains $verb) {
            Deny-Command $Command $Arguments "git '$verb' is not on the read-only allowlist."
        }
        if ($verb -eq 'config') {
            # Reading a value takes one positional after the verb; assigning takes two.
            if ($positional.Count -gt 2) { Deny-Command $Command $Arguments 'git config would write a value.' }
            $writeFlags = @('--add', '--unset', '--unset-all', '--replace-all', '--edit', '--rename-section', '--remove-section')
            foreach ($flag in $Arguments) {
                if ($writeFlags -contains $flag) { Deny-Command $Command $Arguments "git config $flag writes." }
            }
        }
        if ($verb -eq 'fetch') {
            foreach ($flag in $Arguments) {
                if ($flag -eq '--prune' -or $flag -eq '-p') { Deny-Command $Command $Arguments 'fetch --prune deletes local refs.' }
            }
        }
        return
    }

    if ($Command -eq 'gh') {
        if ($positional.Count -eq 0) { Deny-Command $Command $Arguments 'No gh subcommand given.' }
        $verb = $positional[0]
        if (-not $script:allowedGhVerbs.ContainsKey($verb)) {
            Deny-Command $Command $Arguments "gh '$verb' is not on the read-only allowlist."
        }
        $allowedSub = $script:allowedGhVerbs[$verb]
        if ($allowedSub.Count -gt 0) {
            if ($positional.Count -lt 2) { Deny-Command $Command $Arguments "gh $verb needs a subcommand." }
            if ($allowedSub -notcontains $positional[1]) {
                Deny-Command $Command $Arguments "gh $verb $($positional[1]) is not on the read-only allowlist."
            }
        }
        if ($verb -eq 'api') {
            # gh api is GET unless a method or a field is supplied, either a write.
            for ($i = 0; $i -lt $Arguments.Count; $i++) {
                $argument = $Arguments[$i]
                if ($argument -eq '-X' -or $argument -eq '--method') {
                    $method = ''
                    if ($i + 1 -lt $Arguments.Count) { $method = $Arguments[$i + 1] }
                    if ($method -notin @('GET', 'get', 'HEAD', 'head')) {
                        Deny-Command $Command $Arguments "gh api with method '$method' is a write."
                    }
                }
                if ($argument -like '--method=*') {
                    $method = $argument.Substring(9)
                    if ($method -notin @('GET', 'get', 'HEAD', 'head')) {
                        Deny-Command $Command $Arguments "gh api with method '$method' is a write."
                    }
                }
                if ($argument -in @('-f', '--field', '-F', '--raw-field', '--input')) {
                    Deny-Command $Command $Arguments "gh api $argument implies a POST."
                }
            }
        }
        return
    }

    Deny-Command $Command $Arguments "'$Command' is not git or gh."
}

function Invoke-ReadOnly {
    param([string] $Command, [string[]] $Arguments)
    Assert-ReadOnly -Command $Command -Arguments $Arguments

    # Under the Stop preference, a native command writing to stderr raises a terminating
    # NativeCommandError, which killed the run before the caller could look at the exit
    # code. Every caller here checks $LASTEXITCODE and most have a fallback, so a
    # complaint on stderr is theirs to interpret, not a reason to stop.
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Command @Arguments
    } finally {
        $ErrorActionPreference = $previousPreference
    }
}

# Captured while this file is dot-sourced, when $PSScriptRoot is still lib/. Never use
# $PSCommandPath below: in a dot-sourced file it resolves to this file, which would
# silently narrow the audit to this file alone.
$script:deplydLibDirectory = $PSScriptRoot
$script:deplydRoot = Split-Path $PSScriptRoot -Parent

$script:forbiddenConstructs = @(
    @{ Pattern = '(?i)\bInvoke-Expression\b';                      Why = 'Invoke-Expression can run anything' }
    @{ Pattern = '(?i)(^|[^\w-])iex([^\w-]|$)';                    Why = 'iex is Invoke-Expression' }
    @{ Pattern = '(?i)\bStart-Process\b';                          Why = 'Start-Process can launch any executable' }
    @{ Pattern = '(?i)\bStart-Job\b';                              Why = 'Start-Job runs code this audit cannot see' }
    @{ Pattern = '(?i)\bStart-ThreadJob\b';                        Why = 'Start-ThreadJob runs code this audit cannot see' }
    @{ Pattern = '(?i)\bInvoke-Item\b';                            Why = 'Invoke-Item can launch a file' }
    @{ Pattern = '(?i)\bInvoke-Command\b';                         Why = 'Invoke-Command can run a scriptblock' }
    @{ Pattern = '&\s*[\$\(]';                                     Why = 'the call operator on a variable or expression hides what runs' }
    @{ Pattern = '(?i)\[(System\.)?Diagnostics\.Process\]::Start'; Why = 'Process::Start launches an executable' }
    @{ Pattern = '(?i)New-Object\s+(System\.)?Diagnostics\.';      Why = 'a Process object can launch an executable' }
    @{ Pattern = '(?i)(?<![\w$.-])(cmd|powershell|pwsh|wsl|bash|sh)(\.exe)?\s'; Why = 'spawning a shell bypasses this audit' }
    @{ Pattern = '(?i)\.\s*Invoke\s*\(';                           Why = 'invoking a scriptblock hides what runs' }
    @{ Pattern = '(?i)\bSet-Alias\b|\bNew-Alias\b';                Why = 'an alias could point at something this audit does not recognise' }
)

function Get-ShippedFile {
    # Every .ps1 deplyd ships, so the self-check can say what share of them is audited.
    # The tests are not shipped and the fixtures deliberately contain odd things.
    # StartsWith, not -like: an install path containing [ or ] is a wildcard pattern.
    $tests = Join-Path $script:deplydRoot 'tests'
    return @(Get-ChildItem -Path $script:deplydRoot -Filter *.ps1 -File -Recurse |
        Where-Object { -not $_.FullName.StartsWith($tests, [StringComparison]::OrdinalIgnoreCase) } |
        ForEach-Object { $_.FullName })
}

function Get-AuditedFile {
    # Enumerated the same way the loader enumerates them, so a file added to lib/ or
    # commands/ is audited without anyone updating a list.
    $files = @()

    $entry = Join-Path $script:deplydRoot 'deplyd.ps1'
    if (Test-Path $entry) { $files += $entry }

    foreach ($directory in @('lib', 'commands')) {
        $path = Join-Path $script:deplydRoot $directory
        if (-not (Test-Path $path)) { continue }
        $files += @(Get-ChildItem -Path $path -Filter *.ps1 -File | Sort-Object Name | ForEach-Object { $_.FullName })
    }

    if ($files.Count -lt 2 -or -not (Test-Path $entry)) {
        Stop-WithMessage -Message 'Refused: deplyd could not read its own files to audit them.' -Hints @(
            "Looked for deplyd.ps1 and the lib/ and commands/ directories under $script:deplydRoot",
            "Found $($files.Count) file(s)."
        )
    }

    return $files
}

function Test-SourceIsReadOnly {
    param([switch] $Report)

    $violations = @()

    foreach ($file in (Get-AuditedFile)) {
        $source = @(Get-Content -Path $file)
        $name = Split-Path $file -Leaf
        $inGateway = $false
        $inBlockComment = $false

        for ($number = 1; $number -le $source.Count; $number++) {
            $line = $source[$number - 1]

            # Block comments describe the gateway, so they must not be scanned.
            if ($line -match '<#') { $inBlockComment = $true }
            if ($inBlockComment) {
                if ($line -match '#>') { $inBlockComment = $false }
                continue
            }

            if ($line -match '^# region read-only gateway') { $inGateway = $true; continue }
            if ($line -match '^# endregion') { $inGateway = $false; continue }
            if ($inGateway) { continue }

            # Strip string literals and comments so help text and hints cannot trip the audit.
            $stripped = $line -replace "'[^']*'", '' -replace '"[^"]*"', ''
            $hash = $stripped.IndexOf('#')
            if ($hash -ge 0) { $stripped = $stripped.Substring(0, $hash) }

            # Dispatch through the gateway, probing for the executable, and the loader
            # dot-sourcing a library. None can run an arbitrary command by itself.
            $stripped = $stripped -replace 'Invoke-ReadOnly\s+(git|gh)\b', 'Invoke-ReadOnly'
            $stripped = $stripped -replace 'Get-Command\s+(git|gh)\b', 'Get-Command'
            $stripped = $stripped -replace '^\s*\.\s+\$libraryPath\s*$', ''

            if ($stripped -match '(?<![\w$.-])(git|gh)(\.exe)?\s') {
                $violations += "${name} line ${number}: $($line.Trim())"
            }

            # Indirection that would reach git or gh without this audit seeing a name.
            foreach ($construct in $script:forbiddenConstructs) {
                if ($stripped -match $construct.Pattern) {
                    $violations += "${name} line ${number}: $($construct.Why) - $($line.Trim())"
                    break
                }
            }
        }
    }

    if ($Report) { return $violations }

    if ($violations.Count -gt 0) {
        Stop-WithMessage -Message 'Refused: deplyd has been modified to call git or gh outside the read-only gateway.' -Hints (
            @('Every call must go through Invoke-ReadOnly, which allows only read operations.') + $violations
        )
    }
}
# endregion
