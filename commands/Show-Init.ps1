<#
    init: write a .deplyd.json holding what detection concluded.

    Detection is a guess, and the repos it guesses worst are the ones that most need
    the tool. Writing its answer out turns fixing it into editing rather than
    authoring: every key is already there, filled in with what was actually found,
    so a wrong line can be corrected without knowing the schema.

    Written beside deplyd, not into the repo: the premise of the tool is that you have
    no authority over the repository, and an untracked file in someone's working tree
    is a change they have to notice, explain, or gitignore. Moving it to the repo root
    is a deliberate act, and deplyd reads it from there in preference when it is.

    Once it is there, that file is the one deplyd reads, so -Force rewrites it rather
    than a shadow copy that would never take effect. That is the one path where deplyd
    edits a tracked file, and both the refusal and the confirmation say so plainly.

    -Force rewrites from the current conclusion, which already includes anything the
    existing file sets. Regenerating from bare detection instead would throw away the
    corrections the file exists to hold, and in a repo where detection finds nothing
    without them, would leave nothing to write.
#>

function New-OverrideDraft {
    param($Context)

    $environments = [ordered]@{}
    foreach ($name in $Context.Environments) {
        $environments[$name] = [ordered]@{
            workflows = @(Get-WorkflowsForEnvironment -Name $name -Context $Context |
                ForEach-Object { $_.File })
        }
    }

    # Every target deplyd expects to report, with the scope it inferred. An empty list
    # is the interesting case: it means that target is assumed to cover everything.
    # Read from the workflow files rather than from runs, because the repos that need
    # this file are the ones where finding targets is what went wrong.
    $scopes = [ordered]@{}
    foreach ($workflow in $Context.DeployWorkflows) {
        foreach ($name in $workflow.JobNames.Values) {
            if (Test-JobIsIgnored -Name $name -Context $Context) { continue }
            $label = Get-TargetLabel -JobName $name
            if ($scopes.Contains($label)) { continue }
            $scopes[$label] = @($workflow.WorkingDirectories)
        }
    }

    return [ordered]@{
        deployPattern = $Context.DeployPattern
        environments  = $environments
        ignoreJobs    = @($Context.IgnoreJobs)
        scopes        = $scopes
    }
}

function Test-FileIsTracked {
    param([string] $Path)

    # Asked, not assumed. deplyd has no idea how a .deplyd.json came to be in a repo,
    # and warning that something is committed when it is not is the kind of claim the
    # rest of the tool refuses to make.
    # Stderr suppressed: an untracked path is an answer here, not a problem, and git
    # says "did you forget to git add?" about a question deplyd asked on its own.
    $null = Invoke-ReadOnly git @('ls-files', '--error-unmatch', '--', $Path) 2>$null
    return ($LASTEXITCODE -eq 0)
}

function Show-Init {
    param($Context, [bool] $Force)

    $path = $Context.OverridePath
    $exists = Test-Path -LiteralPath $path
    $tracked = $false
    if ($exists -and $Context.OverrideIsShared) { $tracked = Test-FileIsTracked -Path $path }

    if ($exists -and -not $Force) {
        # The three cases are not equally consequential, so they must not read alike.
        if ($tracked) {
            Stop-WithMessage -Message 'There is already a .deplyd.json in this repository, and git is tracking it.' -Hints @(
                $path,
                'Nothing was changed. deplyd reads that file in preference, so rewriting it',
                'edits a tracked file and shows up as a modification:',
                '  deplyd init -Force',
                'Read the result with git diff before committing it.'
            )
        }
        if ($Context.OverrideIsShared) {
            Stop-WithMessage -Message 'There is already a .deplyd.json in this repository.' -Hints @(
                $path,
                'Nothing was changed. git is not tracking it, but it is in your working tree',
                'and deplyd reads it in preference, so rewriting it replaces it:',
                '  deplyd init -Force'
            )
        }
        Stop-WithMessage -Message 'There is already a settings file for this repo.' -Hints @(
            $path,
            'Nothing was changed. To rewrite it from what deplyd concludes now, which',
            'includes whatever that file already sets:',
            '  deplyd init -Force'
        )
    }

    $draft = New-OverrideDraft -Context $Context
    Write-JsonFile -Path $path -Value $draft

    Write-Host ''
    Write-Host "Wrote $path" -ForegroundColor Cyan
    Write-Host ''
    if ($tracked) {
        # "Changes nothing" is false here: this is a tracked file that just changed.
        Write-Host '  git is tracking that file, so it now shows as modified.' -ForegroundColor Yellow
        Write-Host '  Read it with git diff before committing it.' -ForegroundColor Yellow
        Write-Host ''
        Write-Host '  It holds what deplyd already concluded, so deplyd behaves as before.' -ForegroundColor DarkGray
    } elseif ($Context.OverrideIsShared) {
        Write-Host '  It is in your working tree, though git is not tracking it.' -ForegroundColor Yellow
        Write-Host ''
        Write-Host '  It holds what detection found, so it changes nothing on its own.' -ForegroundColor DarkGray
    } else {
        Write-Host '  It holds what detection found, so it changes nothing on its own.' -ForegroundColor DarkGray
    }
    Write-Host '  Edit the lines that are wrong, then run deplyd config to check.' -ForegroundColor DarkGray
    Write-Host ''
    Write-Host "  environments  $($Context.Environments.Count) found: $($Context.Environments -join ', ')" -ForegroundColor DarkGray
    Write-Host "  scopes        $($draft.scopes.Count) target(s), empty means covers everything" -ForegroundColor DarkGray
    Write-Host "  deployPattern the regex that decided which workflows deploy" -ForegroundColor DarkGray
    Write-Host "  ignoreJobs    substrings that keep a job from being a target" -ForegroundColor DarkGray
    Write-Host ''
    if (-not $Context.OverrideIsShared) {
        # The name is the whole mechanism, and "move it as .deplyd.json" reads as a
        # move with a note attached. Print the command so the rename cannot be missed.
        $shared = Join-Path $Context.RepoRoot '.deplyd.json'
        $copy = if (Test-IsWindows) { 'copy' } else { 'cp' }

        Write-Host '  It lives beside deplyd, so your repository is left alone.' -ForegroundColor DarkGray
        Write-Host ''
        Write-Host '  To share the fixes, copy it into the repo under the name .deplyd.json,' -ForegroundColor DarkGray
        Write-Host '  which is the only name deplyd looks for, and commit it:' -ForegroundColor DarkGray
        Write-Host ''
        Write-Host "    $copy `"$path`" `"$shared`"" -ForegroundColor Gray
        Write-Host ''
    }
}
