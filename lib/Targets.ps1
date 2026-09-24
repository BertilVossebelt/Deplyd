<#
    Turning deploy runs into targets: one per deploy job, each with the commit it
    checked out, what it skipped, and what it covers.
#>

# How far back to look. Both are stated in the output when they bite: a target that
# last deployed further back is not reported, and silence reads as "no such target".
$script:runsPerWorkflow = 15
$script:barrenRunsBeforeStopping = 3

function Get-DeployRuns {
    param($Context)

    $runs = @()
    foreach ($workflow in $Context.EnvironmentWorkflows) {
        $json = Invoke-ReadOnly gh @(
            'run', 'list', '--workflow', $workflow.File, '--limit', [string] $script:runsPerWorkflow,
            '--json', 'databaseId,createdAt,url,status,conclusion,headSha'
        )
        if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($json)) { continue }

        foreach ($run in ($json | ConvertFrom-Json)) {
            $run | Add-Member -NotePropertyName WorkflowFile -NotePropertyValue $workflow.File -Force
            $run | Add-Member -NotePropertyName WorkflowToken -NotePropertyValue $workflow.Token -Force
            $run | Add-Member -NotePropertyName NeedsLog -NotePropertyValue $workflow.NeedsLog -Force
            $run | Add-Member -NotePropertyName WorkingDirectories -NotePropertyValue $workflow.WorkingDirectories -Force
            $run | Add-Member -NotePropertyName JobEnvironments -NotePropertyValue $workflow.JobEnvironments -Force
            $run | Add-Member -NotePropertyName CalledWorkflows -NotePropertyValue $workflow.CalledWorkflows -Force
            $runs += $run
        }
    }

    $runs = @($runs | Sort-Object databaseId -Unique | Sort-Object { [datetime] $_.createdAt } -Descending)

    if ($Context.Environment -and -not $Context.NarrowedByName) {
        $runs = @(Select-RunsByDeployment -Runs $runs -Environment $Context.Environment)
    }
    return $runs
}

function Select-RunsByDeployment {
    param($Runs, [string] $Environment)

    # The environment came in as a dispatch input, which the API does not expose. It does
    # record a deployment per environment, linking back to the run that created it, and
    # the same walk records the commit each deployment was created for.
    $runIds = @(Request-Deployments -Environment $Environment)

    # Widening to every environment is the opposite of what was asked for, so say so.
    if ($runIds.Count -eq 0) {
        Write-Host "  no deployment records for '$Environment'; showing runs from every environment" -ForegroundColor Yellow
        return $Runs
    }

    $filtered = @($Runs | Where-Object { $runIds -contains [long] $_.databaseId })
    if ($filtered.Count -eq 0) {
        Write-Note "  no runs matched the deployment records for '$Environment'; showing runs from every environment" -Colour Yellow
        return $Runs
    }

    Write-Note "  matched $($filtered.Count) run(s) to '$Environment' via GitHub deployments"
    return $filtered
}

function Get-DeployTargets {
    param($Context, $Runs)

    $successRuns = @($Runs | Where-Object { $_.conclusion -eq 'success' })
    if ($successRuns.Count -eq 0) {
        Stop-WithMessage -Message "No successful $(Get-EnvironmentPhrase -Context $Context)deploy runs found."
    }

    $targets = [ordered]@{}
    $runsWithoutNewTarget = 0
    $script:ignoredJobs = @{}

    foreach ($run in $successRuns) {
        # Walking the whole history costs an API call per run for no gain.
        if ($targets.Count -gt 0 -and $runsWithoutNewTarget -ge $script:barrenRunsBeforeStopping) {
            $looked = $successRuns.IndexOf($run)
            Write-Note ("  stopped after $looked of $($successRuns.Count) successful run(s): " +
                "the last $script:barrenRunsBeforeStopping revealed no new target")
            break
        }
        $countBefore = $targets.Count

        foreach ($job in (Get-RunJobs $run.databaseId)) {
            $target = New-TargetFromJob -Context $Context -Run $run -Job $job -Known $targets
            if ($target) { $targets[$target.Label] = $target }
        }

        if ($targets.Count -eq $countBefore) { $runsWithoutNewTarget++ } else { $runsWithoutNewTarget = 0 }
    }

    if ($targets.Count -eq 0) {
        # "Nothing found" is a dead end. An ignore word matching by accident is the
        # usual cause, so name the jobs that were passed over.
        $hints = @(
            "Looked at the newest $script:runsPerWorkflow runs of each workflow, stopping after $script:barrenRunsBeforeStopping in a row revealed no new target.",
            'A target deployed less recently than that will not be found.'
        )
        if ($script:ignoredJobs.Count -gt 0) {
            $hints += 'These jobs were skipped:'
            foreach ($name in ($script:ignoredJobs.Keys | Sort-Object)) {
                $hints += "  $name - $($script:ignoredJobs[$name])"
            }
            $hints += 'Set ignoreJobs in .deplyd.json to change that list.'
        }
        Stop-WithMessage -Message "No deploy jobs recognised in the recent $(Get-EnvironmentPhrase -Context $Context)runs." -Hints $hints
    }

    return $targets
}

function Get-JobEnvironment {
    param($Run, [string] $JobName)

    if (-not $Run.JobEnvironments -or $Run.JobEnvironments.Count -eq 0) { return '' }

    # Jobs from a called workflow arrive as "Deploy API / deploy-api".
    $candidates = @($JobName, (($JobName -split '/')[-1]).Trim())
    foreach ($candidate in $candidates) {
        if ($Run.JobEnvironments.ContainsKey($candidate)) { return $Run.JobEnvironments[$candidate] }
    }
    return ''
}

function Get-CalledWorkflow {
    param($Run, [string] $JobName)

    if (-not $Run.CalledWorkflows -or $Run.CalledWorkflows.Count -eq 0) { return '' }

    $candidates = @($JobName, (($JobName -split '/')[-1]).Trim())
    foreach ($candidate in $candidates) {
        if ($Run.CalledWorkflows.ContainsKey($candidate)) { return $Run.CalledWorkflows[$candidate] }
    }
    return ''
}

# Shared with init, which has to skip the same plumbing when it guesses at labels
# without any runs to look at.
function Get-MatchedIgnoreWords {
    param([string] $Name, $Context)

    $token = Get-Token $Name
    return @($Context.IgnoreJobs | Where-Object { $token -like "*$(Get-Token $_)*" })
}

function Test-JobIsIgnored {
    param([string] $Name, $Context)
    return (@(Get-MatchedIgnoreWords -Name $Name -Context $Context).Count -gt 0)
}

function New-TargetFromJob {
    param($Context, $Run, $Job, $Known)

    if ($Job.conclusion -ne 'success') { return $null }

    # One workflow can deploy to several environments in the same run.
    $jobEnvironment = Get-JobEnvironment -Run $Run -JobName $Job.name
    if ($Context.Environment -and $jobEnvironment) {
        $folded = Resolve-EnvironmentAlias -Name $jobEnvironment -Aliases $Context.Aliases
        if ($folded -ne $Context.Environment) {
            $script:ignoredJobs[$Job.name] = "deploys to '$jobEnvironment', not $($Context.Environment)"
            return $null
        }
    }

    $matchedIgnore = @(Get-MatchedIgnoreWords -Name $Job.name -Context $Context)
    if ($matchedIgnore.Count -gt 0) {
        $script:ignoredJobs[$Job.name] = "name contains '$($matchedIgnore[0])'"
        return $null
    }
    if (@($Job.steps).Count -lt 3) {
        $script:ignoredJobs[$Job.name] = 'fewer than three steps'
        return $null
    }

    $label = Get-TargetLabel -JobName $Job.name
    if ($Known.Contains($label)) { return $null }

    $scope = @($Run.WorkingDirectories)

    # A calling job does its work over there, so the scope is described in that file.
    if ($scope.Count -eq 0) {
        $calledWorkflow = Get-CalledWorkflow -Run $Run -JobName $Job.name
        if ($calledWorkflow) {
            $facts = @($Context.Facts | Where-Object { $_.File -eq $calledWorkflow })
            if ($facts.Count -gt 0) { $scope = @($facts[0].WorkingDirectories) }
        }
    }

    if ($Context.Scopes -and $Context.Scopes.ContainsKey($label)) {
        $scope = @($Context.Scopes[$label])
    }

    $shaWarning = ''
    $shaIsExact = $true
    $shaSource = "the run's own ref"

    if ($Run.NeedsLog) {
        $log = @(Get-RunLog $Run.databaseId)
        $resolved = $null
        if ($log.Count -gt 0) { $resolved = Resolve-CheckoutSha -Log $log -JobName $Job.name }

        if ($resolved) {
            $sha = $resolved.Sha
            $shaIsExact = $resolved.Exact
            $shaWarning = $resolved.Warning
            $shaSource = 'the run log'
        } else {
            # The log is gone, or named nothing this clone has. GitHub keeps the
            # deployment record after it deletes the log, and that record names the
            # commit the deployment was created for.
            $fallback = Get-DeploymentSha -RunId $Run.databaseId -Environment $Context.Environment -AllowFetch $true
            if (-not $fallback -or -not (Test-CommitExists -Sha $fallback)) {
                if ($log.Count -eq 0) {
                    $script:ignoredJobs[$Job.name] = "run $($Run.databaseId) has no readable log, and no deployment record names a commit this clone has"
                } else {
                    $script:ignoredJobs[$Job.name] = 'no commit in the run log or the deployment record that exists in this clone; try fetching'
                }
                return $null
            }
            $sha = $fallback
            $shaIsExact = $false
            $shaSource = 'the deployment record'
            $shaWarning = 'the run log could not name the commit, so this is what the deployment was created for'
        }
    } else {
        $sha = $Run.headSha
        if (-not (Test-CommitExists -Sha $sha)) {
            $script:ignoredJobs[$Job.name] = "commit $($Run.headSha.Substring(0, 9)) is not in this clone; try fetching"
            return $null
        }
    }

    # A second opinion. Free when the deployment walk already happened, which it does
    # whenever an environment had to be matched. Worth paying for when the commit is
    # already a guess, since that is where a second record actually decides something.
    # Never paid for merely to agree with a commit the log stated outright.
    $corroborated = $false
    $worthFetching = (-not $shaIsExact) -or ($shaSource -eq 'the deployment record')
    $crossCheck = Compare-ShaWithDeployment -Sha $sha -RunId $Run.databaseId `
        -Environment $Context.Environment -AllowFetch $worthFetching
    if ($crossCheck.Known -and $shaSource -ne 'the deployment record') {
        if ($crossCheck.Agrees) {
            $corroborated = $true
        } else {
            # Two records of the same deploy that do not match. Usually the branch moved
            # between the deployment being created and the checkout running, so neither
            # is simply wrong, but the reader should decide rather than deplyd.
            $shaIsExact = $false
            $shaWarning = "the deployment record names $($crossCheck.Sha.Substring(0, 9)) instead; the branch may have moved mid-deploy"
        }
    }

    return [pscustomobject]@{
        Label        = $label
        Job          = $Job.name
        Run          = $Run
        Sha          = $sha
        ShaIsExact   = $shaIsExact
        ShaWarning   = $shaWarning
        ShaSource    = $shaSource
        Corroborated = $corroborated
        Skipped      = @($Job.steps | Where-Object { $_.conclusion -eq 'skipped' } | ForEach-Object { $_.name })
        Deployed     = @($Job.steps | Where-Object { $_.conclusion -eq 'success' } | ForEach-Object { $_.name })
        Scope        = $scope
        Concerns     = @()
    }
}

function Test-LabelsAreInformative {
    param($Targets)

    # One target means the same label on every row; several with no scope between them
    # means every row says COMBINED. Either way the column carries nothing.
    if ($Targets.Count -lt 2) { return $false }
    foreach ($label in $Targets.Keys) {
        if ($Targets[$label].Scope.Count -gt 0) { return $true }
    }
    return $false
}

function Get-LabelWidth {
    param($Targets)
    $width = 8
    foreach ($label in $Targets.Keys) {
        if ($label.Length -gt $width) { $width = $label.Length }
    }
    return $width
}

function Test-TargetCoversFiles {
    param($Target, [string[]] $Files)

    if ($Target.Scope.Count -eq 0) { return $true }
    foreach ($file in $Files) {
        foreach ($scope in $Target.Scope) {
            if ($file -like "$scope*") { return $true }
        }
    }
    return $false
}
