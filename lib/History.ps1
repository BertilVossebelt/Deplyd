<#
    Questions about a commit's fate after it was merged. Ancestry says a commit was
    included in a build; it does not say the change survived.
#>

function Find-RevertCommit {
    param([string] $CommitSha, [string] $DeployedSha)

    # git and GitHub's revert button both write "This reverts commit <full sha>."
    $byBody = @(Invoke-ReadOnly git @(
        'log', "$CommitSha..$DeployedSha", '--format=%h%x09%s',
        '--fixed-strings', "--grep=This reverts commit $CommitSha"
    ))
    if ($byBody.Count -gt 0) { return $byBody }

    # A hand-written revert may only say so in the subject.
    $subject = (Invoke-ReadOnly git @('log', '-1', '--format=%s', $CommitSha))
    if (-not $subject) { return @() }

    $quoted = $subject -replace '\s*\(#\d+\)\s*$', ''
    if (-not $quoted) { return @() }

    return @(Invoke-ReadOnly git @(
        'log', "$CommitSha..$DeployedSha", '--format=%h%x09%s',
        '--fixed-strings', "--grep=Revert", "--grep=$quoted", '--all-match'
    ))
}

function Get-LaterCommitsTouching {
    param([string] $CommitSha, [string] $DeployedSha, [string[]] $Paths)

    # File level, deliberately. Line level cannot be done correctly from here: git
    # log -L resolves its range against the END of the revision range, while the line
    # numbers available are from the commit at the start, so any later insertion makes
    # the answer quietly about different lines.
    #
    # Only the first such commit counts; after that, later edits say nothing about this
    # change in particular.
    $arguments = @('log', "$CommitSha..$DeployedSha", '--no-merges', '--format=%h%x09%s')
    if ($Paths -and $Paths.Count -gt 0) { $arguments += @('--') + $Paths }

    $commits = @(Invoke-ReadOnly git $arguments | Where-Object { $_ })
    if ($commits.Count -eq 0) { return @() }

    return @($commits[-1])   # log is newest first, so the earliest is last
}

function Find-CopyOfCommit {
    param([string] $CommitSha, [string] $DeployedSha)

    # A recorded cherry-pick names its origin in the message. Exact, so check it first.
    $trailer = @(Invoke-ReadOnly git @(
        'log', $DeployedSha, '--format=%h%x09%s', '--fixed-strings',
        "--grep=(cherry picked from commit $CommitSha"
    ) | Where-Object { $_ })
    if ($trailer.Count -gt 0) {
        $parts = $trailer[0] -split "`t", 2
        return [pscustomobject]@{ Sha = $parts[0]; Subject = $parts[1]; How = 'recorded cherry-pick' }
    }

    # git cherry compares by patch id, so a copy with a different sha still matches.
    # A leading "-" means the deployed commit already contains an equivalent change.
    $marks = @(Invoke-ReadOnly git @('cherry', $DeployedSha, $CommitSha) | Where-Object { $_ })
    $equivalent = @($marks | Where-Object { $_ -match "^-\s+$CommitSha" })
    if ($equivalent.Count -eq 0) { return $null }

    # Name the copy where possible: a cherry-pick keeps the original subject.
    $subject = (Invoke-ReadOnly git @('log', '-1', '--format=%s', $CommitSha))
    if ($subject) {
        $named = @(Invoke-ReadOnly git @(
            'log', $DeployedSha, '--max-count=5', '--format=%h%x09%s', '--fixed-strings', "--grep=$subject"
        ) | Where-Object { $_ })
        if ($named.Count -gt 0) {
            $parts = $named[0] -split "`t", 2
            return [pscustomobject]@{ Sha = $parts[0]; Subject = $parts[1]; How = 'identical change' }
        }
    }

    return [pscustomobject]@{ Sha = ''; Subject = $subject; How = 'identical change' }
}

function Get-RevertedCommits {
    param([string] $DeployedSha)

    # Every revert names what it undid, so one pass collects them all.
    $bodies = @(Invoke-ReadOnly git @(
        'log', $DeployedSha, '--max-count=500', '--format=%b',
        '--fixed-strings', '--grep=This reverts commit'
    ))

    $reverted = @{}
    foreach ($match in [regex]::Matches(($bodies -join "`n"), 'This reverts commit ([0-9a-f]{7,40})')) {
        $reverted[$match.Groups[1].Value] = $true
    }
    return $reverted
}

function Test-CommitWasReverted {
    param([string] $ShortSha, $RevertedCommits)

    if (-not $RevertedCommits -or $RevertedCommits.Count -eq 0) { return $false }
    foreach ($full in $RevertedCommits.Keys) {
        if ($full.StartsWith($ShortSha)) { return $true }
    }
    return $false
}
