<#
    config: what detection concluded, so it can be checked before being trusted.
#>

function Show-Config {
    param($Context)

    $displayRoot = $Context.RepoRoot
    if (Test-IsWindows) { $displayRoot = $displayRoot.Replace([char]47, [char]92) }
    $workflows = $Context.EnvironmentWorkflows
    $logCount = @($workflows | Where-Object { $_.NeedsLog }).Count

    Write-Host ''
    Write-Host "Repo           $displayRoot"
    Write-Host "Environments   $($Context.Environments -join ', ')"
    Write-Host "Selected       $($Context.Environment)"
    Write-Host "Author         $($Context.Author)"
    Write-Host ''
    Write-Host "Deploy workflows ($($workflows.Count))" -ForegroundColor Cyan

    $groups = $workflows | Group-Object { ($_.WorkingDirectories -join ', ') } | Sort-Object Name -Descending
    foreach ($group in $groups) {
        $scope = $group.Name
        if (-not $scope) { $scope = '(no scope)' }
        $first = $true
        foreach ($workflow in ($group.Group | Sort-Object File)) {
            if ($first) {
                Write-Host ("  {0,-16} {1}" -f $scope, $workflow.File)
                $first = $false
            } else {
                Write-Host ("  {0,-16} {1}" -f '', $workflow.File)
            }
        }
    }

    Write-Host ''
    if ($logCount -eq 0) {
        Write-Host "Commit source  the run's own ref" -ForegroundColor DarkGray
    } elseif ($logCount -eq $workflows.Count) {
        Write-Host 'Commit source  the run log - these workflows deploy a branch input or call other workflows' -ForegroundColor DarkGray
    } else {
        Write-Host "Commit source  the run log for $logCount of $($workflows.Count), the run's own ref for the rest" -ForegroundColor DarkGray
    }
    Write-Host "Ignored jobs   names containing $($Context.IgnoreJobs -join ', ')" -ForegroundColor DarkGray
    if ($Context.Scopes.Count -gt 0) {
        foreach ($label in ($Context.Scopes.Keys | Sort-Object)) {
            Write-Host "Scope override $label = $($Context.Scopes[$label] -join ', ')" -ForegroundColor DarkGray
        }
    }
    if ($Context.Override) {
        $where = 'kept beside deplyd'
        if ($Context.OverrideIsShared) { $where = 'committed in the repo' }
        Write-Host "Overrides      $($Context.OverridePath) ($where)" -ForegroundColor DarkGray
    } else {
        Write-Host "No overrides   deplyd init writes what is above to a file you can correct" -ForegroundColor DarkGray
    }
    Write-Host ''
}
