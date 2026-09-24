<#
    Everything a command needs to know, worked out once: where the repo is, who the
    author is, which workflows deploy, and which environments exist.
#>

function Test-IsWindows {
    # PowerShell 5.1 has no Platform property and only runs on Windows; 6+ reports it.
    if ($null -ne $PSVersionTable.Platform) { return ($PSVersionTable.Platform -eq 'Win32NT') }
    return $true
}

function Get-GhInstallHint {
    if (Test-IsWindows) { return 'winget install GitHub.cli' }
    if ($IsMacOS) { return 'brew install gh' }
    return 'see https://github.com/cli/cli#installation'
}

function Get-SettingsPath {
    return (Join-Path $script:deplydRoot 'deplyd.settings.json')
}

function Get-PrivateOverridePath {
    param([string] $RepoRoot)

    # Beside deplyd, not inside the repo. The premise of the tool is that you have no
    # authority over the repository you are pointing it at, and leaving an untracked
    # file in someone's working tree contradicts that. Named after the repo, with a
    # hash of the full path so two clones of the same name stay apart.
    # git rev-parse --show-toplevel answers in forward slashes on Windows while
    # -RepoPath arrives in backslashes. The same repo reached two ways must not end
    # up with two settings files, so normalise before hashing.
    $full = [System.IO.Path]::GetFullPath($RepoRoot).TrimEnd([char]92, [char]47)
    if (Test-IsWindows) { $full = $full.ToLowerInvariant() }

    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = $sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($full))
    } finally {
        $sha.Dispose()
    }
    $short = -join ($bytes[0..3] | ForEach-Object { $_.ToString('x2') })
    $leaf = ((Split-Path $full -Leaf) -replace '[^A-Za-z0-9._-]', '-').ToLowerInvariant()

    return (Join-Path (Join-Path $script:deplydRoot 'overrides') "$leaf-$short.json")
}

function Get-OverridePath {
    param([string] $RepoRoot)

    # A file committed at the repo root wins: someone put it there on purpose, and it
    # is the one every teammate sees. The private one is the fallback.
    $shared = Join-Path $RepoRoot '.deplyd.json'
    if (Test-Path -LiteralPath $shared) { return $shared }
    return (Get-PrivateOverridePath -RepoRoot $RepoRoot)
}

function Write-JsonFile {
    param([string] $Path, $Value)

    $directory = Split-Path $Path -Parent
    if ($directory -and -not (Test-Path -LiteralPath $directory)) {
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
    }
    # Not Set-Content -Encoding utf8: that writes a BOM on 5.1 and none on 7, so the
    # same command would produce different bytes on the two versions deplyd supports.
    # ConvertTo-Json differs between them too, hence ConvertTo-JsonText.
    # A line feed, not Environment::NewLine: the body is joined with line feeds, so a
    # platform-dependent terminator would put the version difference back in the one
    # byte at the end of the file.
    $text = (ConvertTo-JsonText -Value $Value) + "`n"
    [System.IO.File]::WriteAllText($Path, $text, (New-Object System.Text.UTF8Encoding $false))
}

function Get-Settings {
    $path = Get-SettingsPath
    $settings = @{}
    if (Test-Path $path) {
        $loaded = Get-Content -Path $path -Raw | ConvertFrom-Json
        foreach ($property in $loaded.PSObject.Properties) { $settings[$property.Name] = $property.Value }
    }
    return $settings
}

function Save-Settings {
    param([hashtable] $Settings)
    $path = Get-SettingsPath
    Write-JsonFile -Path $path -Value $Settings
    Write-Host "Saved to $path" -ForegroundColor Green
    $Settings.GetEnumerator() | ForEach-Object { Write-Host "  $($_.Key) = $($_.Value)" }
}

function Resolve-RepoRoot {
    param([string] $RepoPath, [hashtable] $Settings)

    if (-not $RepoPath -and $Settings.ContainsKey('repoPath')) { $RepoPath = $Settings['repoPath'] }
    if (-not $RepoPath) { $RepoPath = (Get-Location).Path }

    if (-not (Test-Path $RepoPath)) {
        Stop-WithMessage -Message "Path does not exist: $RepoPath"
    }
    Set-Location $RepoPath

    # Outside a repository git writes to stderr, terminating under the Stop preference.
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $repoRoot = (Invoke-ReadOnly git @('rev-parse', '--show-toplevel') 2>$null)
    $ErrorActionPreference = $previousPreference

    if ($LASTEXITCODE -ne 0 -or -not $repoRoot) {
        Stop-WithMessage -Message "Not a git repository: $RepoPath" -Hints @(
            'Either cd into a repo first, or point at one:',
            'deplyd -RepoPath <path to a repo>',
            'deplyd remember repo <path to a repo>'
        )
    }

    $repoRoot = $repoRoot.Trim()
    Set-Location $repoRoot
    return $repoRoot
}

$script:environmentAliases = [ordered]@{
    production  = @('production', 'prod', 'prd', 'live')
    staging     = @('staging', 'stage', 'stg')
    acceptance  = @('acceptance', 'accept', 'uat')
    test        = @('test', 'qa')
    development = @('development', 'develop', 'dev')
    preview     = @('preview', 'sandbox')
}

function Get-EnvironmentPhrase {
    param($Context)
    # Plenty of repos deploy without naming an environment.
    if ($Context.Environment) { return "$($Context.Environment) " }
    return ''
}

function New-DeplydContext {
    param([string] $RepoRoot, [hashtable] $Settings, [string] $Author)

    $overridePath = Get-OverridePath -RepoRoot $RepoRoot
    $override = $null
    if (Test-Path -LiteralPath $overridePath) {
        $override = Get-Content -Path $overridePath -Raw | ConvertFrom-Json
    }

    $workflowDirectory = Join-Path $RepoRoot '.github/workflows'
    if (-not (Test-Path $workflowDirectory)) {
        Stop-WithMessage -Message "No .github/workflows found in $RepoRoot" -Hints @(
            'There is nothing for deplyd to inspect in this repository.'
        )
    }

    $workflowFiles = @(Get-ChildItem -Path $workflowDirectory -File | Where-Object { $_.Extension -in @('.yml', '.yaml') })
    $facts = @($workflowFiles | ForEach-Object { Get-WorkflowFacts -File $_ })

    # cd as a whole word, so cd.yml and ci-cd.yml match but "cdn-purge" does not.
    $deployPattern = 'deploy|release|publish|ship|\bcd\b'
    if ($override -and $override.deployPattern) { $deployPattern = $override.deployPattern }

    $deployWorkflows = @($facts | Where-Object { "$($_.File) $($_.Name)" -match $deployPattern })
    if ($deployWorkflows.Count -eq 0) {
        # Nothing matched by name, so fall back to GitHub's own marker for shipping:
        # a job that declares an environment.
        $deployWorkflows = @($facts | Where-Object { $_.DeclaredEnvironments.Count -gt 0 })
    }
    if ($deployWorkflows.Count -eq 0) {
        Stop-WithMessage -Message 'No deploy workflows found.' -Hints @(
            "Workflow names were searched for /$deployPattern/, and no job declares an environment.",
            'Add .deplyd.json at the repo root with a deployPattern or an explicit environments map.',
            'See the README section "Correcting it by hand".'
        )
    }

    $ignoreJobs = @('merge', 'environment', 'notify', 'trigger', 'lint', 'test', 'summary', 'setup', 'prepare', 'complete')
    if ($override -and $override.ignoreJobs) { $ignoreJobs = @($override.ignoreJobs) }

    # Scope normally comes from working-directory. Without one it can be named per
    # target, keyed by the label deplyd reports.
    $scopes = @{}
    if ($override -and $override.scopes) {
        foreach ($property in $override.scopes.PSObject.Properties) {
            $scopes[$property.Name.ToUpperInvariant()] = @($property.Value)
        }
    }

    $context = [pscustomobject]@{
        RepoRoot             = $RepoRoot
        Settings             = $Settings
        Author               = $Author
        Override             = $override
        OverridePath         = $overridePath
        OverrideIsShared     = ($overridePath -eq (Join-Path $RepoRoot '.deplyd.json'))
        Facts                = $facts
        DeployPattern        = $deployPattern
        DeployWorkflows      = $deployWorkflows
        Aliases              = $script:environmentAliases
        IgnoreJobs           = $ignoreJobs
        Scopes               = $scopes
        Environments         = @()
        Environment          = ''
        EnvironmentWorkflows = $deployWorkflows
        NarrowedByName       = $false
    }

    $context.Environments = @(Get-DetectedEnvironments -Context $context)
    return $context
}

function Get-DetectedEnvironments {
    param($Context)

    # Three sources, merged, all limited to workflows already identified as deploys.
    $environments = @()

    # 1. Aliases recognised in the workflow filename.
    foreach ($key in $Context.Aliases.Keys) {
        foreach ($alias in $Context.Aliases[$key]) {
            if (@($Context.DeployWorkflows | Where-Object { $_.Token -like "*$alias*" }).Count -gt 0) {
                $environments += $key
                break
            }
        }
    }

    # 2. Environments the jobs declare, folded so production-api and production-web
    #    both land on production, and anything unrecognised keeps its own name.
    foreach ($workflow in $Context.DeployWorkflows) {
        foreach ($name in $workflow.DeclaredEnvironments) {
            $environments += (Resolve-EnvironmentAlias -Name $name -Aliases $Context.Aliases)
        }
    }

    # 3. Options of a workflow_dispatch choice input naming the environment.
    foreach ($workflow in $Context.DeployWorkflows) {
        $environments += $workflow.InputEnvironments
    }

    $environments = @($environments | Select-Object -Unique | Sort-Object)

    if ($Context.Override -and $Context.Override.environments) {
        $environments = @($Context.Override.environments.PSObject.Properties.Name)
    }
    return $environments
}

function Select-Environment {
    param($Context, [string] $Requested)

    $fromSettings = $false
    if (-not $Requested -and $Context.Settings.ContainsKey('environment')) {
        $Requested = $Context.Settings['environment']
        $fromSettings = $true
    }
    if (-not $Requested) {
        if ($Context.Environments -contains 'production') {
            $Requested = 'production'
        } elseif ($Context.Environments.Count -gt 0) {
            $Requested = $Context.Environments[0]
        }
    }

    if ($Requested -and $Context.Environments.Count -gt 0) {
        # Exact first, as with commands: with both "prod" and "production" present,
        # -E prod names one of them rather than being ambiguous between the two.
        $matched = @($Context.Environments | Where-Object { $_ -eq $Requested })
        if ($matched.Count -eq 0) {
            $matched = @($Context.Environments | Where-Object { $_ -like "$Requested*" })
        }
        if ($matched.Count -eq 1) {
            $Requested = $matched[0]
        } elseif ($matched.Count -gt 1) {
            Stop-WithMessage -Message "Ambiguous environment '$Requested'" -Hints @(
                "It matches: $($matched -join ', ')",
                'Spell out more of the name.'
            )
        } else {
            $hints = @(
                "Detected: $($Context.Environments -join ', ')",
                'Run deplyd environments to see where each one came from.'
            )
            if ($fromSettings) {
                # Nobody typed this, so say where it came from before they go looking.
                $hints += "This came from a remembered default. Change it with: deplyd remember environment <name>"
            }
            Stop-WithMessage -Message "Unknown environment '$Requested'" -Hints $hints
        }
    }

    $Context.Environment = $Requested
    if ($Requested) {
        $selected = @(Get-WorkflowsForEnvironment -Name $Requested -Context $Context)
        if ($selected.Count -gt 0) {
            $Context.EnvironmentWorkflows = $selected
            $Context.NarrowedByName = ($selected.Count -lt $Context.DeployWorkflows.Count)
        }
    }
    return $Context
}
