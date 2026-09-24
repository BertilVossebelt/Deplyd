<#
    check: what the gateway allows, and whether the source still obeys it.
#>

function Show-SelfCheck {
    Write-Host ''
    Write-Host 'deplyd read-only self-check' -ForegroundColor Cyan
    Write-Host ''
    Write-Host "  git verbs allowed   $($script:allowedGitVerbs -join ', ')"
    Write-Host "  gh commands allowed $(($script:allowedGhVerbs.Keys | Sort-Object) -join ', ')"
    Write-Host '  gh api              GET and HEAD only; -X/--method or -f/-F/--input is refused'
    Write-Host '  git config          reading only; assignment and --add/--unset are refused'
    Write-Host '  indirection         Invoke-Expression, Start-Process, & $var and shells are refused'
    Write-Host "  writes on disk      only inside $script:deplydRoot"
    Write-Host '                      remembered defaults, and what deplyd init scaffolds'
    Write-Host ''

    $violations = @(Test-SourceIsReadOnly -Report)
    if ($violations.Count -gt 0) {
        Write-Host '  Source audit        FAIL' -ForegroundColor Red
        foreach ($violation in $violations) { Write-Host "    $violation" -ForegroundColor Red }
        Write-Host ''
        exit 1
    }

    $audited = @(Get-AuditedFile)
    $shipped = @(Get-ShippedFile)
    $unaudited = @($shipped | Where-Object { $audited -notcontains $_ } |
        ForEach-Object { Split-Path $_ -Leaf } | Sort-Object)

    Write-Host '  Source audit        PASS - every git and gh call goes through the gateway' -ForegroundColor Green
    Write-Host "  Files audited       $($audited.Count) of $($shipped.Count)" -ForegroundColor DarkGray
    if ($unaudited.Count -gt 0) {
        # Naming them is the point: a number on its own hides which files are outside.
        Write-Host "  Not audited         $($unaudited -join ', ')" -ForegroundColor DarkGray
        Write-Host '                      deplyd never loads these; they set it up or launch it' -ForegroundColor DarkGray
    }
    Write-Host ''
    Write-Host '  git fetch updates your own remote-tracking refs. Nothing is sent to GitHub,' -ForegroundColor DarkGray
    Write-Host '  and nothing another person could see is ever changed.' -ForegroundColor DarkGray
    Write-Host ''
}
