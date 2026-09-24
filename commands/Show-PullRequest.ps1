<#
    pr: is one pull request live, and if not, why not.

    Get-PullRequestReport decides; Show-PullRequest only prints what it decided. The
    exit code and the JSON both come from the same object, so a script and a person
    can never be told different things.
#>

function Get-PullRequestReport {
    param($Context, $Targets, [int] $Number)

    $report = [ordered]@{
        number       = $Number
        title        = ''
        state        = ''
        commit       = ''
        commitSource = ''
        branch       = ''
        environment  = $Context.Environment
        status       = 'not found'
        uncertain    = $false
        files        = @()
        targets      = @()
    }

    $pr = Get-PullRequest -Number $Number
    if (-not $pr) { return $report }

    $report.title = $pr.Title
    $report.state = $pr.State
    $report.commit = $pr.Sha
    $report.commitSource = $pr.Source
    $report.branch = $pr.Branch

    if ($pr.State -ne 'MERGED' -or -not $pr.Sha) {
        $report.status = 'not merged'
        return $report
    }

    if (-not (Test-CommitExists -Sha $pr.Sha)) {
        $report.status = 'commit missing locally'
        return $report
    }

    $report.files = @(Get-CommitFiles -CommitSha $pr.Sha)

    foreach ($label in $Targets.Keys) {
        $target = $Targets[$label]
        if (-not (Test-TargetCoversFiles -Target $target -Files $report.files)) { continue }
        $report.targets += (Get-PullRequestTargetReport -Target $target -Label $label `
            -CommitSha $pr.Sha -Files $report.files)
    }

    $covering = @($report.targets)
    if ($covering.Count -eq 0) {
        $report.status = 'not covered'
        return $report
    }

    $report.uncertain = (@($covering | Where-Object { $_.uncertain }).Count -gt 0)

    if (@($covering | Where-Object { $_.status -eq 'reverted' }).Count -gt 0) {
        $report.status = 'reverted'
    } elseif (@($covering | Where-Object { $_.status -notlike 'live*' }).Count -eq 0) {
        $report.status = 'live'
    } else {
        $report.status = 'not live'
    }
    return $report
}

function Get-PullRequestTargetReport {
    param($Target, [string] $Label, [string] $CommitSha, [string[]] $Files)

    $entry = [ordered]@{
        label          = $Label
        deployedCommit = $Target.Sha
        status         = 'not live'
        uncertain      = ((Get-TargetState -Target $Target) -eq 'uncertain')
        copy           = $null
        reverts        = @()
        changedAfter   = @()
    }

    Invoke-ReadOnly git @('merge-base', '--is-ancestor', $CommitSha, $Target.Sha)
    if ($LASTEXITCODE -ne 0) {
        # A cherry-pick or rebase carries the same change under a different sha.
        $copy = Find-CopyOfCommit -CommitSha $CommitSha -DeployedSha $Target.Sha
        if ($copy) {
            $entry.status = 'live as copy'
            $entry.copy = [ordered]@{ commit = $copy.Sha; how = $copy.How; subject = $copy.Subject }
        }
        return $entry
    }

    # A revert is an ancestor too, so say which of the two happened.
    $reverts = @(Find-RevertCommit -CommitSha $CommitSha -DeployedSha $Target.Sha)
    if ($reverts.Count -gt 0) {
        $entry.status = 'reverted'
        $entry.reverts = @($reverts | ForEach-Object {
            $parts = $_ -split "`t", 2
            [ordered]@{ commit = $parts[0]; subject = $parts[1] }
        })
        return $entry
    }

    $entry.status = 'live'

    # Information, not a caveat: later edits do not make the change any less deployed.
    # Only commits touching the same files, and only the first of them.
    $entry.changedAfter = @(Get-LaterCommitsTouching -CommitSha $CommitSha -DeployedSha $Target.Sha -Paths $Files |
        ForEach-Object {
            $parts = $_ -split "`t", 2
            [ordered]@{ commit = $parts[0]; subject = $parts[1] }
        })
    return $entry
}

function Show-PullRequest {
    param($Targets, $Report)

    if ($Report.status -eq 'not found') {
        Write-Host "PR #$($Report.number) : not found." -ForegroundColor Yellow
        Write-Host 'No such pull request, or no access to it.' -ForegroundColor DarkGray
        Write-Host ''
        return
    }

    Write-Host "PR #$($Report.number)  $($Report.title)" -ForegroundColor Cyan

    if ($Report.status -eq 'not merged') {
        Show-UnmergedPullRequest -Report $Report
        return
    }

    $shortSha = $Report.commit.Substring(0, 9)
    if ($Report.commitSource -eq 'api') {
        Write-Host "merge commit $shortSha" -ForegroundColor DarkGray
    } else {
        Write-Host "commit $shortSha (matched on the subject; the API was unavailable)" -ForegroundColor DarkGray
    }

    if ($Report.status -eq 'commit missing locally') {
        Write-Host ''
        Write-Host "  The merge commit $shortSha is not in your local clone." -ForegroundColor Yellow
        Write-Host '  Fetch, then run this again.' -ForegroundColor DarkGray
        Write-Host ''
        return
    }

    if ($Report.status -eq 'not covered') {
        # Either the change really is outside everything that deploys, or a scope is
        # wrong. Those look identical from one line of output, so show the comparison
        # that was actually made: every target, what it covers, and the files. Getting
        # this wrong is the most common way detection misleads, and the fix is a
        # scopes entry, which is only obvious once you can see what was compared.
        Write-Host ''
        Write-Host '  NOT COVERED - it changed nothing inside any deployed target' -ForegroundColor Yellow
        Write-Host ''
        Write-Host '  Targets considered:' -ForegroundColor DarkGray

        $width = Get-LabelWidth -Targets $Targets
        $unscoped = 0
        foreach ($label in $Targets.Keys) {
            $scope = @($Targets[$label].Scope)
            if ($scope.Count -eq 0) {
                $unscoped++
                Write-Host ("    {0,-$width}  covers everything, yet matched nothing" -f $label) -ForegroundColor DarkGray
            } else {
                Write-Host ("    {0,-$width}  covers {1}" -f $label, ($scope -join ', ')) -ForegroundColor DarkGray
            }
        }

        Write-Host ''
        Write-Host '  Files in this pull request:' -ForegroundColor DarkGray
        foreach ($file in $Report.files) { Write-Host "    $file" -ForegroundColor DarkGray }

        Write-Host ''
        if ($unscoped -gt 0) {
            # A target covering everything cannot fail to cover a file, so if one is
            # listed above, the files are not what is wrong.
            Write-Host '  A target above covers everything and still matched nothing, so this is' -ForegroundColor Yellow
            Write-Host '  a bug in deplyd rather than a scope to fix.' -ForegroundColor Yellow
        } else {
            Write-Host '  If a path above should belong to a target, its scope is wrong. Set it:' -ForegroundColor DarkGray
            Write-Host ''
            Write-Host '    deplyd init' -ForegroundColor Gray
            Write-Host ("    then edit scopes so the right target lists the path, for example:") -ForegroundColor DarkGray
            $example = @($Targets.Keys)[0]
            # The first file that has a directory above it. Split-Path -Parent on a
            # root-level file returns nothing, which would print an empty scope.
            $examplePath = ''
            foreach ($file in $Report.files) {
                $parent = (Split-Path $file -Parent) -replace '\\', '/'
                if ($parent) { $examplePath = $parent; break }
            }
            if (-not $examplePath -and $Report.files.Count -gt 0) { $examplePath = $Report.files[0] }
            if (-not $examplePath) { $examplePath = 'src/whatever' }
            Write-Host ("      `"scopes`": { `"$example`": [`"$examplePath`"] }") -ForegroundColor Gray
        }
        Write-Host ''
        return
    }

    $width = Get-LabelWidth -Targets $Targets
    foreach ($entry in $Report.targets) {
        Show-PullRequestForTarget -Entry $entry -Width $width
    }
    Write-Host ''
}

function Show-UnmergedPullRequest {
    param($Report)

    Write-Host ''
    switch ($Report.state) {
        'OPEN' {
            Write-Host '  NOT MERGED - still open, so it cannot be deployed' -ForegroundColor Yellow
            if ($Report.branch) {
                Write-Host "  branch $($Report.branch)" -ForegroundColor DarkGray
            }
        }
        'CLOSED' {
            Write-Host '  NOT MERGED - closed without merging' -ForegroundColor Yellow
        }
        default {
            Write-Host "  NOT MERGED - state is $($Report.state), with no merge commit" -ForegroundColor Yellow
        }
    }
    Write-Host ''
}

function Show-PullRequestForTarget {
    param($Entry, [int] $Width)

    $deployed = $Entry.deployedCommit.Substring(0, 9)

    if ($Entry.status -eq 'not live') {
        Write-Host ("  {0,-$Width}  NOT LIVE - deployed commit is {1}" -f $Entry.label, $deployed) -ForegroundColor Red
        return
    }

    if ($Entry.status -eq 'live as copy') {
        if ($Entry.copy.commit) {
            Write-Host ("  {0,-$Width}  LIVE in {1} as {2}, not the original commit" -f $Entry.label, $deployed, $Entry.copy.commit) -ForegroundColor Green
        } else {
            Write-Host ("  {0,-$Width}  LIVE in {1} as an equivalent change, not the original commit" -f $Entry.label, $deployed) -ForegroundColor Green
        }
        Write-Host ("  {0,-$Width}    {1}: {2}" -f '', $Entry.copy.how, $Entry.copy.subject) -ForegroundColor DarkGray
        return
    }

    if ($Entry.status -eq 'reverted') {
        Write-Host ("  {0,-$Width}  REVERTED - shipped, then undone before {1}" -f $Entry.label, $deployed) -ForegroundColor Red
        foreach ($revert in $Entry.reverts) {
            Write-Host ("  {0,-$Width}    by {1}  {2}" -f '', $revert.commit, $revert.subject) -ForegroundColor Red
        }
        return
    }

    $suffix = ''
    if ($Entry.uncertain) { $suffix = ' (but see UNCERTAIN above)' }
    Write-Host ("  {0,-$Width}  LIVE in {1}{2}" -f $Entry.label, $deployed, $suffix) -ForegroundColor Green

    if ($Entry.changedAfter.Count -eq 0) { return }
    Write-Host ("  {0,-$Width}    these files changed after it, first by:" -f '') -ForegroundColor DarkGray
    foreach ($later in $Entry.changedAfter) {
        Write-Host ("  {0,-$Width}      {1}  {2}" -f '', $later.commit, $later.subject) -ForegroundColor DarkGray
    }
}
