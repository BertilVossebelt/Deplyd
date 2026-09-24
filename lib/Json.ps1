<#
    Machine-readable output. The verdicts are worked out in one place and then either
    printed or serialised, so what a script reads and what a person reads cannot drift.
#>

# Progress chatter. Silent under -Json, where anything but the document is noise.
function Write-Note {
    param([string] $Text, [string] $Colour = 'DarkGray')
    if ($script:jsonMode) { return }
    Write-Host $Text -ForegroundColor $Colour
}

function Get-TargetState {
    param($Target)

    if ($Target.Concerns.Count -gt 0) { return 'uncertain' }
    if (-not $Target.ShaIsExact) { return 'uncertain' }
    return 'published'
}

function New-TargetReport {
    param($Context, $Targets)

    $reports = @()
    foreach ($label in $Targets.Keys) {
        $target = $Targets[$label]
        $reports += [ordered]@{
            label        = $label
            environment  = $Context.Environment
            commit       = $target.Sha
            state        = (Get-TargetState -Target $target)
            exactCommit  = [bool] $target.ShaIsExact
            commitSource = $target.ShaSource
            corroborated = [bool] $target.Corroborated
            scope       = @($target.Scope)
            skipped     = @($target.Skipped)
            runId       = $target.Run.databaseId
            runUrl      = $target.Run.url
            concerns    = @($target.Concerns | ForEach-Object {
                [ordered]@{ runId = $_.RunId; state = $_.State }
            })
        }
    }
    return $reports
}

# A verdict, not a failure. 0 only when the change is in what shipped and nothing about
# that reading is shaky, so a gate is never told "shipped" on evidence this tool has
# already called into question. 1 stays what it has always been: deplyd could not run.
function Get-VerdictExitCode {
    param([string] $Status, [bool] $Uncertain)

    if ($Status -eq 'live') {
        if ($Uncertain) { return 6 }
        return 0
    }

    switch ($Status) {
        'reverted'   { return 3 }
        'not merged' { return 4 }
        'not found'  { return 5 }
        default      { return 2 }
    }
}

function Write-DeplydJson {
    param($Report)

    # Not ConvertTo-Json: its indentation differs between 5.1 and 7, and anything piped
    # to another program should look the same whichever one is running deplyd.
    Write-Output (ConvertTo-JsonText -Value $Report)
}
