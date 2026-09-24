<#
    What GitHub's deployments API says was deployed.

    A second, independent record of the commit: the run log says what checkout resolved,
    the deployment record says what the deployment was created for. Agreement between
    two sources that are produced differently is worth more than either alone, and the
    deployment record outlives the log, which GitHub deletes after the retention period.

    Not authoritative on its own. A deployment is created for a ref, and a branch can
    move between the deployment being created and the checkout running, so a
    disagreement is reported rather than resolved.
#>

$script:deploymentShas = @{}
$script:deploymentsAsked = @{}

function Get-DeploymentKey {
    param([long] $RunId, [string] $Environment)
    return ($RunId.ToString() + '|' + $Environment.ToLowerInvariant())
}

function Add-DeploymentSha {
    param([long] $RunId, [string] $Environment, [string] $Sha)
    if (-not $RunId -or -not $Sha) { return }

    # Keyed by run and environment, not run alone. One run can deploy to several
    # environments from different checkouts, which is the staged-pipeline shape, and a
    # run-wide key would hand one target the other's commit to disagree with.
    $key = Get-DeploymentKey -RunId $RunId -Environment $Environment
    if (-not $script:deploymentShas.ContainsKey($key)) { $script:deploymentShas[$key] = $Sha }

    # A run-wide entry as well, used only when the environment is unknown.
    $any = Get-DeploymentKey -RunId $RunId -Environment ''
    if (-not $script:deploymentShas.ContainsKey($any)) { $script:deploymentShas[$any] = $Sha }
}

function Request-Deployments {
    param([string] $Environment)

    # One walk per environment, however many targets ask. Returns the run ids found,
    # because the environment filter wants those and the sha map comes along free.
    if ($script:deploymentsAsked.ContainsKey($Environment)) {
        return $script:deploymentsAsked[$Environment]
    }

    $runIds = @()
    $query = "repos/{owner}/{repo}/deployments?per_page=20"
    if ($Environment) { $query = "repos/{owner}/{repo}/deployments?environment=$Environment&per_page=20" }

    $deploymentsJson = Invoke-ReadOnly gh @('api', $query)
    if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($deploymentsJson)) {
        foreach ($deployment in ($deploymentsJson | ConvertFrom-Json)) {
            $statusJson = Invoke-ReadOnly gh @('api', "repos/{owner}/{repo}/deployments/$($deployment.id)/statuses?per_page=5")
            if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($statusJson)) { continue }
            foreach ($status in ($statusJson | ConvertFrom-Json)) {
                $linkMatch = [regex]::Match([string] $status.log_url, '/actions/runs/(\d+)')
                if (-not $linkMatch.Success) { continue }
                $runId = [long] $linkMatch.Groups[1].Value
                $runIds += $runId
                Add-DeploymentSha -RunId $runId -Environment ([string] $deployment.environment) `
                    -Sha ([string] $deployment.sha)
            }
        }
    }

    $runIds = @($runIds | Select-Object -Unique)
    $script:deploymentsAsked[$Environment] = $runIds
    return $runIds
}

function Find-DeploymentSha {
    param([long] $RunId, [string] $Environment)

    $key = Get-DeploymentKey -RunId $RunId -Environment $Environment
    if ($script:deploymentShas.ContainsKey($key)) { return $script:deploymentShas[$key] }

    # No entry for this environment. With one named, stop here rather than borrowing
    # another environment's commit: a wrong comparison reads as a real disagreement.
    if ($Environment) { return '' }

    $any = Get-DeploymentKey -RunId $RunId -Environment ''
    if ($script:deploymentShas.ContainsKey($any)) { return $script:deploymentShas[$any] }
    return ''
}

function Get-DeploymentSha {
    param([long] $RunId, [string] $Environment, [bool] $AllowFetch = $false)

    $found = Find-DeploymentSha -RunId $RunId -Environment $Environment
    if ($found) { return $found }

    # Fetched only when something actually needs it: a log that has aged out, or one
    # that named no usable commit. Corroboration reads what is already there and is
    # never worth an API call of its own.
    if (-not $AllowFetch) { return '' }

    $null = Request-Deployments -Environment $Environment
    return (Find-DeploymentSha -RunId $RunId -Environment $Environment)
}

function Compare-ShaWithDeployment {
    param([string] $Sha, [long] $RunId, [string] $Environment, [bool] $AllowFetch = $false)

    $deployed = Get-DeploymentSha -RunId $RunId -Environment $Environment -AllowFetch $AllowFetch
    if (-not $deployed) { return [pscustomobject]@{ Known = $false; Agrees = $false; Sha = '' } }

    return [pscustomobject]@{
        Known  = $true
        Agrees = ($deployed -eq $Sha)
        Sha    = $deployed
    }
}
