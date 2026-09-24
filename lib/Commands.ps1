<#
    Working out which command was asked for, and handling the one that only writes
    settings. Everything here is about arguments, not about deploys.
#>

# The names themselves live in CommandNames.ps1, which the shell wrapper reads too.

function Resolve-Command {
    param([string] $Name)

    # The report is a verb like the rest, so a half-typed command line does nothing.
    if (-not $Name) { return 'help' }

    $wanted = $Name.ToLowerInvariant()
    if ($script:knownCommands.Contains($wanted)) { return $wanted }
    if ($script:hiddenCommands -contains $wanted) { return $wanted }

    # A prefix is enough while it is unambiguous, so "env" and "auth" both work.
    $matched = @($script:knownCommands.Keys | Where-Object { $_.StartsWith($wanted) })
    if ($matched.Count -eq 1) { return $matched[0] }

    if ($matched.Count -gt 1) {
        Stop-WithMessage -Message "Ambiguous command '$Name'" -Hints @(
            "It matches: $($matched -join ', ')"
        )
    }

    Stop-WithMessage -Message "Unknown command '$Name'" -Hints (
        @('Commands:') + @($script:knownCommands.Keys | ForEach-Object {
            "  {0,-13}{1}" -f $_, $script:knownCommands[$_]
        })
    )
}

function Read-PullRequestNumber {
    param([string] $Value)

    if (-not $Value) {
        Stop-WithMessage -Message 'Which pull request?' -Hints @('deplyd pr 412')
    }

    $number = 0
    if (-not [int]::TryParse($Value.TrimStart('#'), [ref] $number) -or $number -le 0) {
        Stop-WithMessage -Message "Not a pull request number: $Value" -Hints @('deplyd pr 412')
    }
    return $number
}

function Save-RememberedSetting {
    param([hashtable] $Settings, [string] $What, [string] $To)

    $keys = @{
        author      = 'author'
        environment = 'environment'
        repo        = 'repoPath'
    }

    if (-not $What -or -not $keys.ContainsKey($What.ToLowerInvariant())) {
        Stop-WithMessage -Message 'What should be remembered?' -Hints @(
            'deplyd remember author "Ada"',
            'deplyd remember environment staging',
            'deplyd remember repo C:\some\repo'
        )
    }
    if (-not $To) {
        Stop-WithMessage -Message "Remember $What as what?" -Hints @("deplyd remember $What <value>")
    }

    $key = $keys[$What.ToLowerInvariant()]
    if ($key -eq 'repoPath') {
        if (-not (Test-Path -LiteralPath $To -PathType Container)) {
            Stop-WithMessage -Message "No such directory: $To" -Hints @('deplyd remember repo <path to a repo>')
        }
        $To = (Resolve-Path -LiteralPath $To).Path
        if (-not (Test-Path -LiteralPath (Join-Path $To '.git'))) {
            # Remembering a path that cannot work leaves every later run failing here.
            Stop-WithMessage -Message "Not a git repository: $To" -Hints @(
                'Point at the root of a clone, the directory holding .git'
            )
        }
    }

    $Settings[$key] = $To
    Save-Settings -Settings $Settings
}
