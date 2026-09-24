<#
    The per-target block every report starts with: what each target is running, and
    whether that can be trusted.
#>

# Worked out before anything prints, because the JSON path needs it too and nothing
# should depend on a report having been displayed first.
function Set-TargetConcerns {
    param($Targets, $Runs)

    foreach ($label in $Targets.Keys) {
        $Targets[$label].Concerns = @(Get-Concerns -Target $Targets[$label] -Runs $Runs -Targets $Targets)
    }
}

function Show-TargetSummary {
    param($Context, $Targets, $Runs, [bool] $IncludePending)

    foreach ($label in $Targets.Keys) {
        $target = $Targets[$label]

        Write-Host ''
        if ($Context.Environment) {
            Write-Host "$label ($($Context.Environment))" -ForegroundColor Cyan
        } else {
            Write-Host "$label" -ForegroundColor Cyan
        }
        Invoke-ReadOnly git @('log', '-1', '--format=  %h  %ci  %s', $target.Sha)
        Write-Host "  run $($target.Run.databaseId) - $($target.Run.url)" -ForegroundColor DarkGray

        if ($target.Scope.Count -gt 0) {
            Write-Host "  scope $($target.Scope -join ', ')" -ForegroundColor DarkGray
        } else {
            Write-Host '  scope everything, no working-directory to narrow it' -ForegroundColor DarkGray
        }
        if ($target.ShaSource -eq 'the deployment record') {
            Write-Host "  commit from $($target.ShaSource), the run log being unavailable" -ForegroundColor Yellow
        }

        if ($target.ShaWarning) {
            if ($target.ShaIsExact) {
                Write-Host "  note $($target.ShaWarning)" -ForegroundColor Yellow
            } else {
                Write-Host "  UNVERIFIED COMMIT: $($target.ShaWarning)" -ForegroundColor Yellow
            }
        }

        if ($target.Skipped.Count -gt 0) {
            Write-Host "  SKIPPED ($($target.Skipped.Count)): $($target.Skipped -join ', ')" -ForegroundColor Yellow
        }

        if ($target.Concerns.Count -gt 0) {
            $count = $target.Concerns.Count
            # Not "unsettled": a cancelled run has finished. What they share is not
            # completing, so each may have changed part of the environment.
            if ($count -eq 1) {
                Write-Host '  UNCERTAIN: a newer deploy did not complete' -ForegroundColor Yellow
            } else {
                Write-Host "  UNCERTAIN: $count newer deploys did not complete" -ForegroundColor Yellow
            }
            foreach ($concern in $target.Concerns) {
                Write-Host ("    run {0}  {1}" -f $concern.RunId, $concern.State) -ForegroundColor Yellow
            }
        } elseif (-not $target.ShaIsExact) {
            Write-Host '  UNCERTAIN: the deployed commit could not be read from the checkout step' -ForegroundColor Yellow
        } else {
            if ($target.Corroborated) {
                # Two records made in different ways, saying the same thing.
                Write-Host '  PUBLISHED: built and released, and the deployment record names the same commit' -ForegroundColor Green
            } else {
                Write-Host '  PUBLISHED: built and released, no newer deploy failing or in flight' -ForegroundColor Green
            }
        }

        if ($IncludePending) {
            Show-PendingForTarget -Context $Context -Target $target -Label $label
        }
    }

    Write-Host ''
}

function Show-PendingForTarget {
    param($Context, $Target, [string] $Label)

    Write-Host ''

    $defaultBranch = Get-DefaultBranchRef
    if (-not $defaultBranch) {
        Write-Host '  NOT LIVE YET: cannot tell, no default branch ref to compare against' -ForegroundColor Yellow
        Write-Host '    origin/HEAD, origin/main and origin/master all failed to resolve' -ForegroundColor DarkGray
        return
    }

    $pending = @(Merge-Records -Records @(
        Get-Records -Arguments @("$($Target.Sha)..$defaultBranch") -Scope $Target.Scope -Label $Label -Author $Context.Author
    ))

    if ($pending.Count -eq 0) {
        Write-Host '  NOT LIVE YET: none' -ForegroundColor DarkGray
        return
    }

    Write-Host '  NOT LIVE YET:' -ForegroundColor Yellow
    $entryWidth = Get-EntryWidth -Entries $pending
    foreach ($entry in $pending) {
        Write-Host ("    {0,-$entryWidth}  {1}" -f $entry.Id, $entry.Title) -ForegroundColor Yellow
    }
}
