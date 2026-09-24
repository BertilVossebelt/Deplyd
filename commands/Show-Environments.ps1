<#
    environments, and authors: the two "what is there" listings.
#>

function Show-Environments {
    param($Context)

    Write-Host ''
    if ($Context.Environments.Count -eq 0) {
        Write-Host 'No named environments.' -ForegroundColor Cyan
        Write-Host 'This repo deploys without naming environments, which is fine.' -ForegroundColor DarkGray
        Write-Host 'Run deplyd with no -E. Name them in .deplyd.json if you want them split.' -ForegroundColor DarkGray
        Write-Host ''
        return
    }

    Write-Host 'Environments' -ForegroundColor Cyan
    $width = 0
    foreach ($name in $Context.Environments) {
        if ($name.Length -gt $width) { $width = $name.Length }
    }

    foreach ($name in $Context.Environments) {
        $matched = @(Get-WorkflowsForEnvironment -Name $name -Context $Context)
        Write-Host ("  {0,-$width}  {1} workflow(s)" -f $name, $matched.Count)
        foreach ($workflow in $matched) {
            Write-Host ("  {0,-$width}    {1}" -f '', $workflow.File) -ForegroundColor DarkGray
        }
    }

    Write-Host ''
    Write-Host 'Use any of these, or a prefix: deplyd -E <name>' -ForegroundColor DarkGray
    Write-Host ''
}

function Show-Authors {
    # Local HEAD is whatever this clone has checked out, usually behind.
    $branch = Get-DefaultBranchRef
    if (-not $branch) {
        Write-Host ''
        Write-Host 'Cannot list authors: no default branch ref to count over.' -ForegroundColor Yellow
        Write-Host 'origin/HEAD, origin/main and origin/master all failed to resolve.' -ForegroundColor DarkGray
        Write-Host ''
        return
    }

    Write-Host ''
    Write-Host "Authors (commit count, name) on $branch" -ForegroundColor Cyan
    Invoke-ReadOnly git @('shortlog', '-sn', '--no-merges', $branch)
    Write-Host ''
    Write-Host 'Use any of these with -A. Partial matches work, so a first name is enough.' -ForegroundColor DarkGray
    Write-Host 'One person can appear under several names; git counts them separately.' -ForegroundColor DarkGray
    Write-Host ''
}
