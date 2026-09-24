<#
    The default report: an author's pull requests, live and pending, across targets.
#>

function Get-StatusReport {
    param($Context, $Targets, [int] $Take, [int] $Skip)

    $records = @()
    $reverted = @{}
    foreach ($label in $Targets.Keys) {
        $target = $Targets[$label]
        $records += Get-Records -Arguments @('-200', $target.Sha) -Scope $target.Scope -Label $label -Author $Context.Author
        foreach ($key in (Get-RevertedCommits -DeployedSha $target.Sha).Keys) { $reverted[$key] = $true }
    }

    $live = @(Merge-Records -Records $records)
    return [pscustomobject]@{
        Author   = $Context.Author
        Live     = $live
        Page     = @($live | Select-Object -Skip $Skip -First $Take)
        Reverted = $reverted
        Skip     = $Skip
    }
}

function New-StatusJson {
    param($Context, $Targets, $Status)

    return [ordered]@{
        author      = $Status.Author
        environment = $Context.Environment
        targets     = @(New-TargetReport -Context $Context -Targets $Targets)
        changes     = @($Status.Live | ForEach-Object {
            [ordered]@{
                label    = $_.Label
                id       = $_.Id
                title    = $_.Title
                commit   = $_.Sha
                reverted = [bool] (Test-CommitWasReverted -ShortSha $_.Sha -RevertedCommits $Status.Reverted)
            }
        })
    }
}

function Show-Status {
    param($Context, $Targets, $Status)

    $live = $Status.Live
    $page = $Status.Page
    $reverted = $Status.Reverted
    $Skip = $Status.Skip
    # Not "PRs": commits pushed straight to a branch appear here too, marked (no PR).
    $heading = "LIVE CHANGES BY $($Context.Author.ToUpperInvariant())"

    if ($live.Count -eq 0) {
        Write-Host "${heading}: none" -ForegroundColor Cyan
    } elseif ($page.Count -eq 0) {
        # Skipped past the end. Counting from $Skip would print a range running backwards.
        Write-Host "$heading ($($live.Count) in total): nothing left after skipping $Skip" -ForegroundColor Cyan
    } else {
        $first = $Skip + 1
        $last = $Skip + $page.Count
        $width = Get-LabelWidth -Targets $Targets

        Write-Host "$heading ($first-$last of $($live.Count)):" -ForegroundColor Cyan
        $entryWidth = Get-EntryWidth -Entries $page
        $showLabels = Test-LabelsAreInformative -Targets $Targets

        foreach ($entry in $page) {
            if ($showLabels) {
                $line = "  {0,-$width}  {1,-$entryWidth}  {2}" -f $entry.Label, $entry.Id, $entry.Title
            } else {
                $line = "  {0,-$entryWidth}  {1}" -f $entry.Id, $entry.Title
            }
            if (Test-CommitWasReverted -ShortSha $entry.Sha -RevertedCommits $reverted) {
                Write-Host "$line  (REVERTED)" -ForegroundColor Red
            } else {
                Write-Host $line
            }
        }

        if ($last -lt $live.Count) {
            Write-Host "  ... $($live.Count - $last) older - next page: -Skip $last" -ForegroundColor DarkGray
        }
    }

    # Built from the last deploy that completed. If newer ones did not, parts of the
    # environment may no longer match that commit.
    $incomplete = @($Targets.Keys | Where-Object { $Targets[$_].Concerns.Count -gt 0 })
    if ($incomplete.Count -gt 0) {
        Write-Host ''
        Write-Host '  Listed against the last completed deploy. Newer ones did not complete,' -ForegroundColor DarkGray
        Write-Host '  so parts of the environment may have moved on since.' -ForegroundColor DarkGray
    }

}
