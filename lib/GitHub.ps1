<#
    Everything that talks to the GitHub API, plus resolving the commit a run
    actually checked out.
#>

# One job list and one log per run, however many targets ask for them.
$script:jobCache = @{}
$script:logCache = @{}

function Get-RunJobs {
    param([long] $RunId)
    if ($script:jobCache.ContainsKey($RunId)) { return $script:jobCache[$RunId] }
    $json = Invoke-ReadOnly gh @('api', "repos/{owner}/{repo}/actions/runs/$RunId/jobs?per_page=100")
    $jobs = @()
    if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($json)) {
        $jobs = @(($json | ConvertFrom-Json).jobs)
    }
    $script:jobCache[$RunId] = $jobs
    return $jobs
}

function Get-RunLog {
    param([long] $RunId)
    if ($script:logCache.ContainsKey($RunId)) { return $script:logCache[$RunId] }
    Write-Note "  reading log for run $RunId..."
    $lines = Invoke-ReadOnly gh @('run', 'view', $RunId, '--log')
    if ($LASTEXITCODE -ne 0) { $lines = @() }
    $script:logCache[$RunId] = $lines
    return $lines
}

$script:hasFetched = $false
$script:defaultBranchRef = ''

function Invoke-FetchOnce {
    param([string] $Reason = 'a commit was missing locally')

    # Costly on a large repository, so it happens once and says what prompted it.
    if ($script:hasFetched) { return }
    $script:hasFetched = $true
    Write-Note "  fetching: $Reason..."
    Invoke-ReadOnly git @('fetch', 'origin', '--quiet')
}

function Get-DefaultBranchRef {
    # origin/HEAD is not always set: single-branch clones and many CI checkouts omit it.
    if ($script:defaultBranchRef) { return $script:defaultBranchRef }

    foreach ($candidate in @('origin/HEAD', 'origin/main', 'origin/master')) {
        $null = Invoke-ReadOnly git @('rev-parse', '--verify', '--quiet', "$candidate^{commit}")
        if ($LASTEXITCODE -eq 0) {
            $script:defaultBranchRef = $candidate
            return $candidate
        }
    }
    return ''
}

function Test-CommitExists {
    param([string] $Sha, [bool] $AllowFetch = $true)

    $null = Invoke-ReadOnly git @('rev-parse', '--verify', '--quiet', "$Sha^{commit}")
    if ($LASTEXITCODE -eq 0) { return $true }

    if (-not $AllowFetch -or $script:hasFetched) { return $false }
    Invoke-FetchOnce
    $null = Invoke-ReadOnly git @('rev-parse', '--verify', '--quiet', "$Sha^{commit}")
    return ($LASTEXITCODE -eq 0)
}

# gh run view --log writes the job name in the first tab-separated column. Matching it
# whole, rather than looking for it anywhere in the line, is what keeps matrix legs
# apart: "deploy (api)" tokenises to a prefix of "deploy (api, eu-west-1)", so
# containment hands one leg the other leg's checkout.
function Select-JobLines {
    param([string[]] $Log, [string] $JobName)

    $wanted = @(
        (Get-Token $JobName),
        (Get-Token (($JobName -split '/')[-1]).Trim())
    ) | Select-Object -Unique

    $lines = @($Log | Where-Object {
        if ($_ -match 'Download action repository') { return $false }
        $wanted -contains (Get-Token ($_ -split "`t", 2)[0])
    })
    if ($lines.Count -gt 0) { return $lines }

    # No column matched, so this log is not in that shape. Rather than report nothing,
    # fall back to the old whole-line containment and let the caller's checks apply.
    return @($Log | Where-Object {
        $_ -notmatch 'Download action repository' -and (Get-Token $_) -like "*$($wanted[0])*"
    })
}

function Resolve-CheckoutSha {
    param([string[]] $Log, [string] $JobName)

    # actions/checkout prints the resolved commit on a line of its own. That shape,
    # within the job's own lines, is the only trustworthy source; looser is a guess.
    $pattern = '[0-9a-f]{40}'
    $bareShaLine = "Z[^\S\r\n]+'?$pattern'?[^\S\r\n]*$"
    $jobLines = @(Select-JobLines -Log $Log -JobName $JobName)

    $strict = @()
    foreach ($line in $jobLines) {
        if ($line -notmatch $bareShaLine) { continue }
        $candidate = [regex]::Match($line, $pattern).Value
        if (Test-CommitExists -Sha $candidate) { $strict += $candidate }
    }
    $strict = @($strict | Select-Object -Unique)

    if ($strict.Count -ge 1) {
        # More than one checkout: the deploy steps ran against the first.
        return [pscustomobject]@{
            Sha      = $strict[0]
            Exact    = $true
            Warning  = if ($strict.Count -gt 1) { "the job checked out $($strict.Count) different commits; using the first" } else { '' }
        }
    }

    # Nothing bare in this job, so fall back to any commit named anywhere in the run.
    # Without fetching: pinned action SHAs and cache keys are 40-hex too, and fetching on
    # the first that misses would pay the whole cost to answer a guess.
    foreach ($line in @($Log | Where-Object { $_ -notmatch 'Download action repository' -and $_ -match $pattern })) {
        foreach ($match in [regex]::Matches($line, $pattern)) {
            if (Test-CommitExists -Sha $match.Value -AllowFetch $false) {
                return [pscustomobject]@{
                    Sha     = $match.Value
                    Exact   = $false
                    Warning = 'no checkout line found for this job; this commit was taken from elsewhere in the run log'
                }
            }
        }
    }

    return $null
}

$script:labelNoiseWords = @('deploy', 'deploys', 'deployment', 'deploying', 'quick', 'to', 'and', 'the', 'job')

function Get-CleanLabelPart {
    param([string] $Text)

    # Whole words: stripping "deploy" as a substring leaves "build-and", "to-production".
    $segments = @($Text -split '[\s_,\-]+' | Where-Object { $_ })
    $kept = @($segments | Where-Object { $script:labelNoiseWords -notcontains $_.ToLowerInvariant() })
    return ($kept -join '-')
}

function Get-TargetLabel {
    param([string] $JobName)

    # Jobs from a called workflow arrive as "Deploy API / deploy-api".
    $name = (($JobName -split '/')[-1]).Trim()

    # A matrix leg arrives as "deploy (api, eu-west-1)", and those values are what tell
    # the legs apart, so they belong in the label.
    $values = ''
    $matrix = [regex]::Match($name, '^(.*?)\s*\((.+)\)\s*$')
    if ($matrix.Success) {
        $name = $matrix.Groups[1].Value
        $values = $matrix.Groups[2].Value
    }

    $parts = @()
    foreach ($piece in @((Get-CleanLabelPart -Text $name), (Get-CleanLabelPart -Text $values))) {
        if ($piece) { $parts += $piece }
    }

    if ($parts.Count -eq 0) { return (($JobName -split '/')[-1]).Trim().ToUpperInvariant() }
    return ($parts -join '-').ToUpperInvariant()
}

function Test-RunDidAnything {
    param([long] $RunId)

    # A run cancelled in the queue deployed nothing. One whose jobs had started may have
    # published part of a deploy, which is worth flagging.
    foreach ($job in (Get-RunJobs $RunId)) {
        if ($job.conclusion -eq 'skipped') { continue }
        if ($job.started_at) { return $true }
    }
    return $false
}

function Get-Concerns {
    param($Target, $Runs, $Targets)
    $concerns = @()
    $targetToken = Get-Token $Target.Label
    $otherTokens = @($Targets.Keys | Where-Object { $_ -ne $Target.Label } | ForEach-Object { Get-Token $_ })

    foreach ($run in $Runs) {
        if ([datetime] $run.createdAt -le [datetime] $Target.Run.createdAt) { continue }
        $relevant = ($run.WorkflowToken -like "*$targetToken*") -or ($run.WorkflowFile -eq $Target.Run.WorkflowFile)
        if (-not $relevant) {
            # A workflow naming a different target is that target's problem, not this
            # one's. A workflow naming neither (a combined deploy) counts for both.
            $mentionsOther = @($otherTokens | Where-Object { $run.WorkflowToken -like "*$_*" }).Count -gt 0
            if ($mentionsOther) { continue }
        }
        if ($run.status -ne 'completed') {
            # The API reports snake_case states; do not put those in front of a reader.
            $concerns += [pscustomobject]@{
                RunId = $run.databaseId
                State = ([string] $run.status).Replace('_', ' ')
            }
        } elseif ($run.conclusion -eq 'failure') {
            $concerns += [pscustomobject]@{ RunId = $run.databaseId; State = 'failed' }
        } elseif ($run.conclusion -eq 'cancelled') {
            if (Test-RunDidAnything -RunId $run.databaseId) {
                $concerns += [pscustomobject]@{ RunId = $run.databaseId; State = 'cancelled part-way' }
            }
        }
    }
    return @($concerns | Sort-Object RunId -Unique -Descending)
}

function Get-PullRequest {
    param([int] $Number)

    # The API knows the merge commit whatever the style was. The fallback below only
    # handles squash merges, which is why this is tried first.
    $json = Invoke-ReadOnly gh @('pr', 'view', "$Number", '--json', 'number,title,state,mergedAt,mergeCommit,headRefName')
    if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($json)) {
        $pr = $json | ConvertFrom-Json
        $sha = ''
        if ($pr.mergeCommit) { $sha = [string] $pr.mergeCommit.oid }
        return [pscustomobject]@{
            Number = $pr.number
            Title  = $pr.title
            State  = $pr.state
            Sha    = $sha
            Branch = $pr.headRefName
            Source = 'api'
        }
    }

    # No API answer: fall back to the squash-merge subject, which carries "(#123)".
    $defaultBranch = Get-DefaultBranchRef
    if (-not $defaultBranch) { return $null }

    $found = @(Invoke-ReadOnly git @(
        'log', $defaultBranch, '--no-merges', '--format=%H%x09%s',
        '--fixed-strings', "--grep=(#$Number)", '-1'
    ) | Where-Object { $_ })
    if ($found.Count -eq 0) { return $null }

    $parts = $found[0] -split "`t", 2
    return [pscustomobject]@{
        Number = $Number
        Title  = $parts[1]
        State  = 'MERGED'
        Sha    = $parts[0]
        Branch = ''
        Source = 'subject match'
    }
}

function Get-CommitFiles {
    param([string] $CommitSha)

    # show prints nothing for a clean merge, whose combined diff is empty.
    $parents = @((Invoke-ReadOnly git @('rev-list', '--parents', '-n', '1', $CommitSha)) -split '\s+' | Where-Object { $_ })
    if ($parents.Count -gt 2) {
        return @(Invoke-ReadOnly git @('diff', '--name-only', "$CommitSha^1", $CommitSha) | Where-Object { $_ })
    }
    return @(Invoke-ReadOnly git @('show', '--name-only', '--format=', $CommitSha) | Where-Object { $_ })
}
