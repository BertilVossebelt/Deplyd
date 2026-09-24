#Requires -Version 5.1
<#
    deplyd - which commit an environment was last deployed from.

    Finds the newest successful deploy run per target, resolves the commit that run
    checked out, and reports whether a PR (or an author's commits) is in it.

    This file only parses arguments, loads the libraries and dispatches. The work lives
    in lib/ and the output in commands/.
#>
[CmdletBinding()]
param(
    # What to do is a verb; how to do it stays a flag.
    [Parameter(Position = 0)]
    [string] $Command = '',

    [Parameter(Position = 1)]
    [string] $Argument = '',

    [Parameter(Position = 2)]
    [string] $Value = '',

    [Alias('E')]
    [string] $Environment = '',
    [Alias('A')]
    [string] $Author = '',
    [Alias('T')]
    [ValidateRange(1, 1000)]
    [int] $Take = 10,
    [Alias('S')]
    [ValidateRange(0, 2147483647)]
    [int] $Skip = 0,
    [string] $RepoPath = '',
    [Alias('J')]
    [switch] $Json,
    [Alias('F')]
    [switch] $Force
)

$ErrorActionPreference = 'Stop'

# Read by Write-Note, so progress chatter never lands in the middle of a document.
$script:jsonMode = [bool] $Json

# ReadOnly.ps1 first: it defines the gateway and the audit everything else relies on.
# Then whatever is in lib/ and commands/, so a file added later needs no list updating.
$firstLibrary = Join-Path $PSScriptRoot 'lib/ReadOnly.ps1'
if (-not (Test-Path $firstLibrary)) {
    Write-Host "Missing library file: $firstLibrary" -ForegroundColor Red
    exit 1
}
. $firstLibrary

# Audit before loading anything else. Dot-sourcing runs a file's top-level code, so
# auditing afterwards would only refuse to continue, not refuse to run.
Test-SourceIsReadOnly

foreach ($directory in @('lib', 'commands')) {
    $path = Join-Path $PSScriptRoot $directory
    if (-not (Test-Path $path)) { continue }
    foreach ($file in (Get-ChildItem -Path $path -Filter *.ps1 -File | Sort-Object Name)) {
        if ($file.FullName -eq $firstLibrary) { continue }
        . $file.FullName
    }
}

$verb = Resolve-Command -Name $Command

if ($Json -and $verb -notin @('status', 'pr')) {
    Stop-WithMessage -Message "-Json has nothing to say about '$verb'" -Hints @(
        'It is available for the two commands that reach a verdict:',
        '  deplyd status -Json',
        '  deplyd pr 412 -Json'
    )
}

# The commands below change directory, and this runs in the caller's session.
$callerLocation = Get-Location

try {
    # --- needs nothing -------------------------------------------------------------

    if ($verb -eq 'help') { Show-Help; return }
    if ($verb -eq 'check') { Show-SelfCheck; return }

    # --- needs only the settings file ----------------------------------------------

    $settings = Get-Settings

    if ($verb -eq 'remember') {
        Save-RememberedSetting -Settings $settings -What $Argument -To $Value
        return
    }

    # --- needs a repository ---------------------------------------------------------

    $repoRoot = Resolve-RepoRoot -RepoPath $RepoPath -Settings $settings

    if ($verb -eq 'authors') { Show-Authors; return }

    # Before the author is resolved: the shell asks for these on a tab press, and a
    # missing git config user.name must not turn that into an error.
    if ($verb -eq 'complete') {
        Show-Completions -What $Argument -Context (
            New-DeplydContext -RepoRoot $repoRoot -Settings $settings -Author ''
        )
        return
    }

    if (-not $Author -and $settings.ContainsKey('author')) { $Author = $settings['author'] }
    if (-not $Author) { $Author = (Invoke-ReadOnly git @('config', 'user.name')) }
    if (-not $Author) {
        # git log --author='' matches everyone, reporting their work as yours.
        Stop-WithMessage -Message 'No author to filter on.' -Hints @(
            'Pass one with -A <name>, or keep one with: deplyd remember author <name>',
            'It normally comes from git config user.name, which is not set here.'
        )
    }

    # --- needs the workflows read ---------------------------------------------------

    $context = New-DeplydContext -RepoRoot $repoRoot -Settings $settings -Author $Author

    if ($verb -eq 'environments') { Show-Environments -Context $context; return }

    # Before an environment is chosen: the draft covers all of them, and a repo whose
    # detection is wrong enough to need this file may well reject the one you asked for.
    if ($verb -eq 'init') { Show-Init -Context $context -Force ([bool] $Force); return }

    $context = Select-Environment -Context $context -Requested $Environment

    if ($verb -eq 'config') { Show-Config -Context $context; return }

    # --- needs GitHub ---------------------------------------------------------------

    # Read the argument first: a typo in it is worth saying before a missing gh is.
    $pullRequest = 0
    if ($verb -eq 'pr') { $pullRequest = Read-PullRequestNumber -Value $Argument }

    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
        Stop-WithMessage -Message 'GitHub CLI (gh) is required and was not found.' -Hints @(
            (Get-GhInstallHint),
            'gh auth login'
        )
    }

    Write-Note "Inspecting $(Get-EnvironmentPhrase -Context $context)deploys..."

    $runs = @(Get-DeployRuns -Context $context)
    $targets = Get-DeployTargets -Context $context -Runs $runs
    Set-TargetConcerns -Targets $targets -Runs $runs

    # Those lists compare against the default branch. A single PR check does not.
    if ($pullRequest -le 0) { Invoke-FetchOnce -Reason 'so the pending list is current' }

    if ($pullRequest -gt 0) {
        $report = Get-PullRequestReport -Context $context -Targets $targets -Number $pullRequest

        if ($Json) {
            Write-DeplydJson -Report ([ordered]@{
                pullRequest = $report
                targets     = @(New-TargetReport -Context $context -Targets $targets)
            })
        } else {
            Show-TargetSummary -Context $context -Targets $targets -Runs $runs -IncludePending $false
            Show-PullRequest -Targets $targets -Report $report
        }

        exit (Get-VerdictExitCode -Status $report.status -Uncertain $report.uncertain)
    }

    $status = Get-StatusReport -Context $context -Targets $targets -Take $Take -Skip $Skip

    if ($Json) {
        Write-DeplydJson -Report (New-StatusJson -Context $context -Targets $targets -Status $status)
        return
    }

    Show-TargetSummary -Context $context -Targets $targets -Runs $runs -IncludePending $true
    Show-Status -Context $context -Targets $targets -Status $status

} finally {
    Set-Location $callerLocation
}
