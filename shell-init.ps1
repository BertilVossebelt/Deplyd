<#
    Defines the deplyd function for an interactive session. The profile dot-sources
    this one file, so the wrapper can name its parameters and have something for tab
    completion to attach to. Wrapping @args alone gives PowerShell nothing to complete,
    and it falls back to offering file names.

    Deliberately not an advanced function: [CmdletBinding()] or any [Parameter()]
    attribute would add -ErrorAction and -ErrorVariable, and then -E no longer resolves
    to -Environment. Every name below therefore starts with a different letter, so each
    short flag stays unambiguous on its own.
#>

$script:deplydScript = Join-Path $PSScriptRoot 'deplyd.ps1'
$script:deplydNames = @{}

# Just the names, so what completion offers cannot drift from what the tool accepts.
. (Join-Path $PSScriptRoot 'lib/CommandNames.ps1')
$script:deplydVerbs = @($knownCommands.Keys)
$script:deplydOptions = @($knownOptions.Keys)

# Anything the wrapper does not declare but PowerShell would have accepted anyway.
$script:deplydCommonParameters = @(
    'Verbose', 'Debug', 'ErrorAction', 'ErrorVariable', 'WarningAction', 'WarningVariable',
    'InformationAction', 'InformationVariable', 'OutVariable', 'OutBuffer', 'PipelineVariable'
)

function script:Write-DeplydRefusal {
    param([string] $Message, [string[]] $Hints)
    Write-Host ''
    Write-Host $Message -ForegroundColor Red
    Write-Host ''
    foreach ($hint in $Hints) { Write-Host "  $hint" -ForegroundColor DarkGray }
    Write-Host ''
}

function deplyd {
    param($Command, $Noun, $Value, $Environment, $Author, $Take, $Skip, $RepoPath,
          [switch] $Json, [switch] $Force)

    $words = @()
    foreach ($positional in @('Command', 'Noun', 'Value')) {
        if ($PSBoundParameters.ContainsKey($positional)) { $words += $PSBoundParameters[$positional] }
    }

    # Named ones go back in a hashtable, never in the array. Splatting an array passes
    # every element positionally, so "-Author" would arrive as a value, not a name, and
    # land in the next positional parameter without anything complaining.
    $flags = @{}
    foreach ($named in @('Environment', 'Author', 'Take', 'Skip', 'RepoPath', 'Json', 'Force')) {
        if ($PSBoundParameters.ContainsKey($named)) { $flags[$named] = $PSBoundParameters[$named] }
    }

    # Whatever is left is either an extra word or a flag the wrapper does not declare.
    # Declaring all of them means an unknown one is genuinely unknown, so say so here
    # rather than letting PowerShell answer in a voice that is not the tool's.
    for ($i = 0; $i -lt $args.Count; $i++) {
        $word = [string] $args[$i]
        if ($word -notlike '-*') { $words += $args[$i]; continue }

        $name = $word.TrimStart('-')
        $common = @($script:deplydCommonParameters | Where-Object { $_ -like "$name*" })
        if ($common.Count -eq 0) {
            Write-DeplydRefusal -Message "Unknown option $word" -Hints (
                @('Options:') +
                @($script:deplydOptions | ForEach-Object { "  $_" }) +
                @('', 'Commands come first and without a dash: deplyd pr 412')
            )
            return
        }

        if ($i + 1 -lt $args.Count -and ([string] $args[$i + 1]) -notlike '-*') {
            $flags[$name] = $args[$i + 1]
            $i++
        } else {
            $flags[$name] = $true
        }
    }

    & $script:deplydScript @words @flags
}

# "deplyd" shares its first four letters with deploymentcsphelper.exe on Windows, so
# tab has to be pressed twice to get past it. Only claimed if nothing else answers to it.
if (-not (Get-Command dp -ErrorAction SilentlyContinue)) { Set-Alias dp deplyd -Scope Global }

function script:Get-WorkflowStamp {
    # Part of the cache key, so editing what detection reads is picked up without
    # reopening the shell. Walks up because tab is often pressed in a subdirectory.
    $directory = (Get-Location).Path
    while ($directory) {
        $workflows = Join-Path $directory '.github/workflows'
        if (Test-Path -LiteralPath $workflows) {
            $stamp = $workflows

            $times = @(Get-ChildItem -Path $workflows -File -ErrorAction SilentlyContinue |
                ForEach-Object { $_.LastWriteTimeUtc.Ticks })
            if ($times.Count -gt 0) { $stamp += '@' + ($times | Measure-Object -Maximum).Maximum }

            # The override sits beside .github and can replace the environment list
            # outright, so editing it is the edit most likely to mean the last list was
            # wrong. Absent to present moves the stamp by itself.
            $override = Join-Path $directory '.deplyd.json'
            if (Test-Path -LiteralPath $override) {
                $stamp += '+' + (Get-Item -LiteralPath $override).LastWriteTimeUtc.Ticks
            }

            return $stamp
        }
        $directory = Split-Path $directory -Parent
    }
    return ''
}

function script:Get-DeplydName {
    param([string] $What)

    # Half a second a run: too slow per tab press, fine once.
    $key = $What + '|' + (Get-Location).Path + '|' + (Get-WorkflowStamp)
    if ($script:deplydNames.ContainsKey($key)) { return $script:deplydNames[$key] }

    # Every stream but output: deplyd explains itself to a person when it cannot read a
    # repo, and a tab press is not a person asking. Write-Host lands on the information
    # stream, so 2>$null alone would still print it over the prompt.
    $found = @()
    $failed = $false
    try {
        # Zeroed first: a clean script leaves whatever the last command set.
        $global:LASTEXITCODE = 0
        $found = @(& $script:deplydScript complete $What 2>$null 3>$null 4>$null 5>$null 6>$null)
        $failed = ($LASTEXITCODE -ne 0)
    } catch {
        $failed = $true
    }

    # A failure and a repo with nothing to offer both come back empty. Caching the
    # failure would leave completion silently empty here for the rest of the session,
    # reading as "no environments" rather than "that did not work".
    if (-not $failed) { $script:deplydNames[$key] = $found }
    return $found
}

function script:New-DeplydCompletion {
    param([string[]] $Names, [string] $WordToComplete)

    # Quoted when it contains a space, or "-A Ada Lovelace" arrives as two arguments.
    $word = $WordToComplete.Trim("'", '"')
    foreach ($name in $Names) {
        if ($name -notlike "$word*") { continue }
        $text = $name
        if ($text -match '\s') { $text = "'" + $text.Replace("'", "''") + "'" }
        [System.Management.Automation.CompletionResult]::new($text, $name, 'ParameterValue', $name)
    }
}

Register-ArgumentCompleter -CommandName deplyd -ParameterName Command -ScriptBlock {
    param($commandName, $parameterName, $wordToComplete)
    New-DeplydCompletion -WordToComplete $wordToComplete -Names $script:deplydVerbs
}

Register-ArgumentCompleter -CommandName deplyd -ParameterName Noun -ScriptBlock {
    param($commandName, $parameterName, $wordToComplete, $commandAst)

    # Only "remember" takes a second word with a fixed set of answers.
    $verb = ''
    if ($commandAst.CommandElements.Count -gt 1) { $verb = [string] $commandAst.CommandElements[1] }
    if ($verb -notlike 'rem*') { return }

    New-DeplydCompletion -WordToComplete $wordToComplete -Names @('author', 'environment', 'repo')
}

Register-ArgumentCompleter -CommandName deplyd -ParameterName Environment -ScriptBlock {
    param($commandName, $parameterName, $wordToComplete)
    New-DeplydCompletion -WordToComplete $wordToComplete -Names (Get-DeplydName -What 'environments')
}

Register-ArgumentCompleter -CommandName deplyd -ParameterName Author -ScriptBlock {
    param($commandName, $parameterName, $wordToComplete)
    New-DeplydCompletion -WordToComplete $wordToComplete -Names (Get-DeplydName -What 'authors')
}

# "deplyd remember environment <tab>" and "remember author <tab>" want the same lists.
Register-ArgumentCompleter -CommandName deplyd -ParameterName Value -ScriptBlock {
    param($commandName, $parameterName, $wordToComplete, $commandAst)

    $words = @($commandAst.CommandElements | ForEach-Object { [string] $_ })
    if ($words.Count -lt 3 -or $words[1] -notlike 'rem*') { return }

    $what = switch -Wildcard ($words[2]) {
        'env*'  { 'environments'; break }
        'auth*' { 'authors'; break }
        default { '' }
    }
    if (-not $what) { return }

    New-DeplydCompletion -WordToComplete $wordToComplete -Names (Get-DeplydName -What $what)
}
