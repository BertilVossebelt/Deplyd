<#
    Formatting helpers for the report. Nothing here decides anything.
#>

function Format-Entry {
    param([string] $Sha, [string] $Subject)

    # Kept apart so the caller can line the columns up: a short sha and a PR number are
    # different widths. A commit with no squash-merged PR shows its sha, which says so.
    $match = [regex]::Match($Subject, '\(#(\d+)\)\s*$')
    if ($match.Success) {
        return [pscustomobject]@{
            Id    = "PR #$($match.Groups[1].Value)"
            Title = $Subject.Substring(0, $match.Index).TrimEnd()
        }
    }
    return [pscustomobject]@{ Id = $Sha; Title = $Subject }
}

function Get-EntryWidth {
    param($Entries)
    $width = 0
    foreach ($entry in $Entries) {
        if ($entry.Id.Length -gt $width) { $width = $entry.Id.Length }
    }
    return $width
}

function Get-Records {
    param([string[]] $Arguments, [string[]] $Scope, [string] $Label, [string] $Author)

    # An empty author matches every commit, reported as this person's work.
    if (-not $Author) { throw 'Get-Records needs an author; an empty one matches every commit.' }

    $full = @('--author', $Author, '--no-merges', '--format=%h%x09%ct%x09%s') + $Arguments
    if ($Scope -and $Scope.Count -gt 0) { $full += @('--') + $Scope }
    $raw = @(Invoke-ReadOnly git (@('log') + $full))
    foreach ($line in $raw) {
        if (-not $line) { continue }
        $parts = $line -split "`t", 3
        [pscustomobject]@{ Sha = $parts[0]; When = [long] $parts[1]; Subject = $parts[2]; Label = $Label }
    }
}

function Merge-Records {
    param($Records)
    $grouped = $Records | Group-Object Sha | ForEach-Object {
        $labels = @($_.Group | Select-Object -ExpandProperty Label -Unique)
        if ($labels.Count -gt 1) { $label = 'COMBINED' } else { $label = $labels[0] }
        $entry = Format-Entry -Sha $_.Group[0].Sha -Subject $_.Group[0].Subject
        [pscustomobject]@{
            When  = $_.Group[0].When
            Label = $label
            Sha   = $_.Group[0].Sha
            Id    = $entry.Id
            Title = $entry.Title
        }
    }
    return @($grouped | Sort-Object When -Descending)
}
