<#
    complete: bare names, one per line, for the shell to complete against. Not in the
    command list. It exists so shell-init.ps1 never has to run git itself, or scrape a
    listing written for people.
#>

function Show-Completions {
    param($Context, [string] $What)

    switch ($What) {
        'environments' {
            foreach ($name in $Context.Environments) { Write-Output $name }
        }
        'authors' {
            $branch = Get-DefaultBranchRef
            if (-not $branch) { return }
            $counted = Invoke-ReadOnly git @('shortlog', '-sn', '--no-merges', $branch)
            foreach ($line in @($counted)) {
                # "   42\tAda Lovelace" - the name is whatever follows the count.
                if ($line -match '^\s*\d+\s+(.+)$') { Write-Output $Matches[1].Trim() }
            }
        }
    }
}
