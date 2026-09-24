<#
    Works out what a repository deploys, by reading .github/workflows.
#>

function Get-Token {
    param([string] $Value)
    return ($Value -replace '[^A-Za-z0-9]', '').ToLowerInvariant()
}

function Get-JobMaps {
    param([string] $Text)

    # Two things the jobs API does not report: which environment a job ships to, and
    # whether it is only a call to another workflow. Kept apart so each map's name
    # describes what is in it.
    #
    # Indentation is read from the file, not assumed: four spaces is as valid as two,
    # and guessing wrong would silently merge the environments back together.
    $environments = @{}
    $called = @{}
    $names = [ordered]@{}
    $inJobs = $false
    $jobIndent = -1
    $jobKey = ''
    $displayName = ''
    $environmentIndent = -1

    foreach ($line in ($Text -split "`r?`n")) {
        if ($line.Trim().Length -eq 0) { continue }

        $indent = $line.Length - $line.TrimStart().Length

        if (-not $inJobs) {
            if ($line -match '^jobs:') { $inJobs = $true }
            continue
        }
        if ($indent -eq 0) { break }   # back to a top-level key, so jobs: is over

        $keyMatch = [regex]::Match($line, '^\s*([A-Za-z0-9_.\-]+):\s*(.*)$')

        # The first key inside jobs: sets the depth every job key sits at.
        if ($jobIndent -lt 0 -and $keyMatch.Success) { $jobIndent = $indent }

        if ($indent -eq $jobIndent -and $keyMatch.Success) {
            $jobKey = $keyMatch.Groups[1].Value
            $displayName = ''
            $environmentIndent = -1
            # Until a name: says otherwise, a job is known by its key, which is what
            # the jobs API reports for a job that does not name itself.
            $names[$jobKey] = $jobKey
            continue
        }
        if (-not $jobKey -or -not $keyMatch.Success) { continue }

        $key = $keyMatch.Groups[1].Value
        $value = $keyMatch.Groups[2].Value.Trim().Trim('"', "'")

        if ($key -eq 'uses' -and $value -match '\.ya?ml$') {
            # What such a job deploys, and from where, is described over there.
            $called[$jobKey] = ($value -split '/')[-1]
            if ($displayName) { $called[$displayName] = ($value -split '/')[-1] }
            continue
        }

        if ($key -eq 'environment') {
            if ($value) {
                Add-JobEnvironment -Map $environments -JobKey $jobKey -DisplayName $displayName -Environment $value
                $environmentIndent = -1
            } else {
                $environmentIndent = $indent
            }
            continue
        }

        if ($key -eq 'name') {
            if ($environmentIndent -ge 0 -and $indent -gt $environmentIndent) {
                Add-JobEnvironment -Map $environments -JobKey $jobKey -DisplayName $displayName -Environment $value
                $environmentIndent = -1
            } elseif ($environmentIndent -lt 0) {
                $displayName = $value
                if ($jobKey) { $names[$jobKey] = $value }
            }
        }
    }
    return @{ Environments = $environments; CalledWorkflows = $called; JobNames = $names }
}

function Add-JobEnvironment {
    param($Map, [string] $JobKey, [string] $DisplayName, [string] $Environment)
    if (-not $Environment) { return }
    $Map[$JobKey] = $Environment
    if ($DisplayName) { $Map[$DisplayName] = $Environment }
}

function Get-WorkflowFacts {
    param($File)
    # Normalised to LF first. The patterns below anchor with $, which in .NET matches
    # only before the line feed and so never sees past a carriage return. On a CRLF
    # checkout, which core.autocrlf makes the norm on Windows, every one of them would
    # silently find nothing.
    $text = (Get-Content -Path $File.FullName -Raw) -replace "`r`n", "`n"
    $name = $File.BaseName
    $nameMatch = [regex]::Match($text, '(?m)^name:[ \t]*(.+?)[ \t]*$')
    if ($nameMatch.Success) { $name = $nameMatch.Groups[1].Value.Trim() }

    $directories = @([regex]::Matches($text, '(?m)^[ \t]*working-directory:[ \t]*(\S+)[ \t]*$') |
        ForEach-Object { $_.Groups[1].Value } | Select-Object -Unique)

    $declaredEnvironments = @()
    $inlinePattern = '(?m)^[^\S\r\n]{4,}environment:[^\S\r\n]*([A-Za-z0-9_.-]+)[^\S\r\n]*$'
    $blockPattern = '(?m)^[^\S\r\n]{4,}environment:[^\S\r\n]*$\s*^[^\S\r\n]+name:[^\S\r\n]*([A-Za-z0-9_.-]+)[^\S\r\n]*$'
    foreach ($hit in [regex]::Matches($text, $inlinePattern)) { $declaredEnvironments += $hit.Groups[1].Value }
    foreach ($hit in [regex]::Matches($text, $blockPattern)) { $declaredEnvironments += $hit.Groups[1].Value }

    $inputEnvironments = @()
    $inputBlockPattern = '(?ms)^[^\S\r\n]*(environment|env|target|stage)[^\S\r\n]*:[^\S\r\n]*\r?\n(.*?)(?=^\S|\Z)'
    $optionPattern = '(?m)^[^\S\r\n]*-[^\S\r\n]*(.+?)[^\S\r\n]*$'
    foreach ($inputMatch in [regex]::Matches($text, $inputBlockPattern)) {
        $body = $inputMatch.Groups[2].Value
        if ($body -notmatch '(?i)type[^\S\r\n]*:[^\S\r\n]*choice' -and $body -notmatch '(?i)options[^\S\r\n]*:') { continue }
        foreach ($option in [regex]::Matches($body, $optionPattern)) {
            $inputEnvironments += $option.Groups[1].Value.Trim('"', "'").ToLowerInvariant()
        }
    }

    $usesInputRef = [regex]::IsMatch($text, 'ref:[ \t]*\$\{\{[ \t]*(inputs|github\.event\.inputs)\.')
    # uses: <something>.yml checks out somewhere this file cannot see, in this repo or
    # another, so the run's own ref cannot be trusted and the log has to be read.
    $callsWorkflow = [regex]::IsMatch($text, '(?m)^[^\S\r\n]*uses:[^\S\r\n]*\S+\.ya?ml(@\S+)?[^\S\r\n]*$')

    $jobMaps = Get-JobMaps -Text $text

    return [pscustomobject]@{
        File               = $File.Name
        Name               = $name
        Token              = (Get-Token "$($File.BaseName) $name")
        UsesInputRef       = $usesInputRef
        CallsWorkflow      = $callsWorkflow
        JobEnvironments    = $jobMaps.Environments
        CalledWorkflows    = $jobMaps.CalledWorkflows
        JobNames           = $jobMaps.JobNames
        NeedsLog           = ($usesInputRef -or $callsWorkflow)
        DeclaredEnvironments = @($declaredEnvironments | Select-Object -Unique)
        InputEnvironments    = @($inputEnvironments | Select-Object -Unique)
        WorkingDirectories = $directories
    }
}

function Resolve-EnvironmentAlias {
    param([string] $Name, $Aliases)
    $token = Get-Token $Name
    foreach ($key in $Aliases.Keys) {
        foreach ($alias in $Aliases[$key]) {
            if ($token -like "*$alias*") { return $key }
        }
    }
    return $Name.ToLowerInvariant()
}

function Get-WorkflowsForEnvironment {
    param([string] $Name, $Context)

    # An explicit list in .deplyd.json always wins.
    if ($Context.Override -and $Context.Override.environments -and
        $Context.Override.environments.$Name -and $Context.Override.environments.$Name.workflows) {
        $wanted = @($Context.Override.environments.$Name.workflows)
        $explicit = @($Context.Facts | Where-Object { $wanted -contains $_.File })
        if ($explicit.Count -gt 0) { return $explicit }
    }

    $aliasList = @($Name)
    if ($Context.Aliases.Contains($Name)) { $aliasList = $Context.Aliases[$Name] }

    $matched = @($Context.DeployWorkflows | Where-Object {
        $token = $_.Token
        $byName = @($aliasList | Where-Object { $token -like "*$_*" }).Count -gt 0
        $declared = @($_.DeclaredEnvironments | ForEach-Object { Resolve-EnvironmentAlias -Name $_ -Aliases $Context.Aliases })
        $byName -or ($declared -contains $Name) -or ($_.InputEnvironments -contains $Name)
    })
    return @($matched | Sort-Object File)
}
