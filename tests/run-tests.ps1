#Requires -Version 5.1
<#
    Detection tests for deplyd.

    Each fixture under tests/fixtures is copied to a temp directory, turned into a git
    repository, and inspected with -ListEnvironments and -ShowConfig. Only detection is
    covered: fixtures have no runs, so nothing here touches the GitHub API.
#>
[CmdletBinding()]
param(
    [switch] $KeepTemp
)

$ErrorActionPreference = 'Stop'

# Shadows git with a local-only version for the whole suite: fixtures may be written
# to, but nothing here can reach a remote. Must be loaded before any git call.
. (Join-Path $PSScriptRoot 'lib/LocalGit.ps1')

# The suite spawns child shells for cases that exit rather than throw. Use whichever
# PowerShell is running this, so it works under pwsh on macOS and Linux too.
$script:shellPath = (Get-Process -Id $PID).Path
if (-not $script:shellPath) { $script:shellPath = 'powershell' }
$script:shellArguments = @('-NoProfile')
if ($null -eq $PSVersionTable.Platform -or $PSVersionTable.Platform -eq 'Win32NT') {
    $script:shellArguments += @('-ExecutionPolicy', 'Bypass')
}

$toolPath = Join-Path (Split-Path $PSScriptRoot -Parent) 'deplyd.ps1'
$fixtureRoot = Join-Path $PSScriptRoot 'fixtures'
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('deplyd-tests-' + [guid]::NewGuid().ToString('N').Substring(0, 8))

if (-not (Test-Path $toolPath)) { throw "deplyd.ps1 not found at $toolPath" }

# Group names run from three letters to eighteen. One width for all of them, taken from
# the longest in use, so a long one never runs into the description beside it.
$script:groupWidth = 0

function Format-TestGroup {
    param([string] $Group)
    if ($Group.Length -ge $script:groupWidth) { return $Group + '  ' }
    return $Group.PadRight($script:groupWidth)
}

$cases = @(
    @{
        Fixture  = 'conventional'
        Name     = 'environments and scopes from conventional filenames'
        Arguments = @('environments')
        Expect   = @('production', 'staging', 'deploy-production-api.yml', 'deploy-staging-web.yml')
    }
    @{
        Fixture  = 'conventional'
        Name     = 'scope and log parsing for a branch-input workflow'
        Arguments = @('config', '-E', 'prod')
        Expect   = @('Selected       production', 'services/api', 'the run log')
    }
    @{
        Fixture  = 'conventional'
        Name     = "run's own ref when nothing overrides the checkout"
        Arguments = @('config', '-E', 'stag')
        Expect   = @('Selected       staging', 'services/web', "the run's own ref")
    }
    @{
        Fixture  = 'unnamed'
        Name     = 'falls back to declared environments when names match nothing'
        Arguments = @('environments')
        Expect   = @('production', 'staging', 'hoopdiepoopdieloop.yml', 'wobble.yml')
    }
    @{
        Fixture  = 'choice-input'
        Name     = 'reads environments off a workflow_dispatch choice input'
        Arguments = @('environments')
        Expect   = @('demo', 'sandbox', 'deploy-anywhere.yml')
    }
    @{
        Fixture  = 'custom-env'
        Name     = 'keeps an environment name that is not in the alias table'
        Arguments = @('environments')
        Expect   = @('canary', 'deploy-canary.yml')
    }
    @{
        Fixture  = 'custom-env'
        Name     = 'rejects an environment that does not exist'
        Arguments = @('config', '-E', 'production')
        ExpectFail = $true
        Expect   = @('Unknown environment')
    }
    @{
        Fixture  = 'override'
        Name     = '.deplyd.json overrides pattern and pins workflows'
        Arguments = @('config')
        Expect   = @('Selected       production', 'zonk.yml')
    }
    @{
        Fixture  = 'external-reusable'
        Name     = 'a job calling a workflow in another repo needs the log read'
        Arguments = @('config')
        Expect   = @('Selected       production', 'the run log')
    }
    @{
        Fixture  = 'matrix'
        Name     = 'a matrix workflow keeps its environment and scope'
        Arguments = @('config')
        Expect   = @('Selected       production', 'services', "the run's own ref")
    }
    @{
        Fixture  = 'composite-checkout'
        Name     = 'a composite action checkout uses the run ref'
        Arguments = @('config')
        Expect   = @('Selected       production', "the run's own ref")
    }
    @{
        Fixture  = 'staged-pipeline'
        Name     = 'one workflow deploying to staging then production'
        Arguments = @('environments')
        Expect   = @('production', 'staging', 'deploy.yml')
    }
    @{
        Fixture  = 'scoped-override'
        Name     = 'scopes can be set by hand when there is no working-directory'
        Arguments = @('config')
        Expect   = @('Scope override API = services/api', 'Scope override WEB = services/web')
    }
    @{
        Fixture  = 'cd-named'
        Name     = 'a workflow called cd.yml is recognised as a deploy'
        Arguments = @('config')
        Expect   = @('Deploy workflows (1)', 'cd.yml')
    }
    @{
        Fixture  = 'four-space'
        Name     = 'a four-space indented workflow is read the same as a two-space one'
        Arguments = @('environments')
        Expect   = @('production', 'staging', 'deploy.yml')
    }
    @{
        Fixture  = 'no-environments'
        Name     = 'a plain deploy workflow with no environment at all'
        Arguments = @('environments')
        Expect   = @('No named environments', 'which is fine')
    }
    @{
        Fixture  = 'no-environments'
        Name     = 'still finds the workflow without an environment'
        Arguments = @('config')
        Expect   = @('Deploy workflows (1)', 'deploy.yml', "the run's own ref")
    }
    @{
        Fixture  = 'no-deploys'
        Name     = 'fails clearly when there is nothing to inspect'
        Arguments = @('config')
        ExpectFail = $true
        Expect   = @('No deploy workflows found')
    }
    @{
        Fixture  = 'conventional'
        Name     = 'no command lists the commands rather than running a report'
        Arguments = @()
        Expect   = @('COMMANDS', 'deplyd status', 'deplyd check')
    }
    @{
        Fixture  = 'prefix-env'
        Name     = 'an exact environment name beats a longer one sharing its prefix'
        Arguments = @('config', '-E', 'canary')
        Expect   = @('Selected       canary')
        Reject   = @('Ambiguous', 'Selected       canary-eu')
    }
    @{
        Fixture  = 'prefix-env'
        Name     = 'a prefix matching two environments is still ambiguous'
        Arguments = @('config', '-E', 'can')
        ExpectFail = $true
        Expect   = @('Ambiguous environment', 'canary, canary-eu')
    }
    @{
        Fixture  = 'conventional'
        Name     = '-Json is refused for a command that reaches no verdict'
        Arguments = @('config', '-Json')
        ExpectFail = $true
        Expect   = @('-Json has nothing to say', 'deplyd pr 412 -Json')
        Reject   = @('Deploy workflows')
    }
    @{
        Fixture  = 'conventional'
        Name     = 'a bad pull request number is named before gh is looked for'
        Arguments = @('pr', 'abc')
        ExpectFail = $true
        Expect   = @('Not a pull request number: abc')
        Reject   = @('GitHub CLI')
    }
    @{
        Fixture  = 'conventional'
        Name     = 'complete prints bare environment names for the shell'
        Arguments = @('complete', 'environments')
        Expect   = @('production', 'staging')
        Reject   = @('workflow(s)', 'Environments')
    }
)

function Assert-NoRemote {
    # Fixture repos are created with git init and must never gain a remote. With none
    # configured there is nowhere for anything to be sent, which is the point.
    $remotes = @(git remote)
    if ($remotes.Count -gt 0) {
        throw "fixture repo has a remote configured ($($remotes -join ', ')); refusing to run"
    }
}

function New-FixtureRepo {
    param([string] $Fixture)
    $source = Join-Path $fixtureRoot $Fixture
    if (-not (Test-Path $source)) { throw "Fixture not found: $source" }

    $destination = Join-Path $tempRoot ($Fixture + '-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    Copy-Item -Path (Join-Path $source '*') -Destination $destination -Recurse -Force

    Push-Location $destination
    try {
        git init --quiet .
        Assert-NoRemote
        if ($LASTEXITCODE -ne 0) { throw "git init failed in $destination" }
    } finally {
        Pop-Location
    }
    return $destination
}

$passed = 0
$failed = 0

function Test-Allowlist {
    param(
        [string] $Name,
        [string] $Command,
        [string[]] $Arguments,
        [bool] $ShouldRefuse,
        [string[]] $Expect = @()
    )

    # Assert-ReadOnly only validates - it never runs the command - so this is safe to
    # exercise with write operations.
    $libraryPath = Join-Path (Split-Path $toolPath -Parent) 'lib/ReadOnly.ps1'
    $quoted = ($Arguments | ForEach-Object { "'" + ($_ -replace "'", "''") + "'" }) -join ','
    $probe = ". '$libraryPath'; Assert-ReadOnly -Command '$Command' -Arguments @($quoted); Write-Host 'ALLOWED'"

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = (& $script:shellPath @script:shellArguments -Command $probe 2>&1 | Out-String)
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $previousPreference

    $problems = @()
    if ($ShouldRefuse) {
        if ($exitCode -eq 0) { $problems += 'expected it to be refused' }
        if ($output -match 'ALLOWED') { $problems += 'command was allowed' }
    } else {
        if ($output -notmatch 'ALLOWED') { $problems += 'expected it to be allowed' }
    }
    foreach ($expected in $Expect) {
        if ($output -notmatch [regex]::Escape($expected)) { $problems += "missing '$expected'" }
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'allowlist') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'allowlist') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        foreach ($line in ($output -split "`r?`n")) {
            if ($line.Trim()) { Write-Host "        $line" -ForegroundColor DarkGray }
        }
        $script:failed++
    }
}


function Test-ShaResolution {
    param(
        [string] $Name,
        [string] $JobName,
        [string[]] $LogTemplate,
        [string] $ExpectWhich,
        [bool] $ExpectExact
    )

    # A throwaway repo with two real commits, so Test-CommitExists has something to
    # verify against. The log lines are synthetic: no GitHub call is made.
    $repo = Join-Path $tempRoot ('sha-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
    New-Item -ItemType Directory -Path $repo -Force | Out-Null
    Push-Location $repo
    try {
        git init --quiet .
        Assert-NoRemote
        Set-Content -Path 'a.txt' -Value 'one' -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'first'
        $first = (git rev-parse HEAD).Trim()
        Set-Content -Path 'a.txt' -Value 'two' -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'second'
        $second = (git rev-parse HEAD).Trim()

        $libraries = Split-Path $toolPath -Parent
        . (Join-Path $libraries 'lib/ReadOnly.ps1')
        . (Join-Path $libraries 'lib/Detection.ps1')
        . (Join-Path $libraries 'lib/GitHub.ps1')

        $log = @($LogTemplate | ForEach-Object { $_ -replace 'FIRST', $first -replace 'SECOND', $second })
        $result = Resolve-CheckoutSha -Log $log -JobName $JobName

        $expected = if ($ExpectWhich -eq 'first') { $first } elseif ($ExpectWhich -eq 'second') { $second } else { $null }

        $problems = @()
        if ($null -eq $expected) {
            if ($null -ne $result) { $problems += 'expected no result' }
        } else {
            if ($null -eq $result) {
                $problems += 'expected a result, got none'
            } else {
                if ($result.Sha -ne $expected) { $problems += "resolved $($result.Sha.Substring(0,9)), expected $($expected.Substring(0,9)) ($ExpectWhich)" }
                if ($result.Exact -ne $ExpectExact) { $problems += "Exact was $($result.Exact), expected $ExpectExact" }
            }
        }
    } finally {
        Pop-Location
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'sha') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'sha') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

function Test-RevertDetection {
    param([string] $Name, [bool] $ActuallyRevert, [bool] $ExpectDetected)

    $repo = Join-Path $tempRoot ('revert-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
    New-Item -ItemType Directory -Path $repo -Force | Out-Null
    Push-Location $repo
    try {
        git init --quiet .
        Assert-NoRemote
        Set-Content -Path 'a.txt' -Value 'base' -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'base'

        Set-Content -Path 'a.txt' -Value 'feature' -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'feat: the change (#1629)'
        $feature = (git rev-parse HEAD).Trim()

        if ($ActuallyRevert) {
            git -c user.email=t@t -c user.name=Test revert --no-edit $feature | Out-Null
        } else {
            Set-Content -Path 'a.txt' -Value 'feature plus more' -Encoding utf8
            git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'chore: unrelated follow-up'
        }
        $head = (git rev-parse HEAD).Trim()

        $libraries = Split-Path $toolPath -Parent
        . (Join-Path $libraries 'lib/ReadOnly.ps1')
        . (Join-Path $libraries 'lib/History.ps1')

        $found = @(Find-RevertCommit -CommitSha $feature -DeployedSha $head)
        $problems = @()
        if ($ExpectDetected -and $found.Count -eq 0) { $problems += 'revert not detected' }
        if (-not $ExpectDetected -and $found.Count -gt 0) { $problems += "falsely reported a revert: $($found -join '; ')" }
    } finally {
        Pop-Location
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'revert') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'revert') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

function Test-CherryPickDetection {
    param([string] $Name, [string] $Mode, [bool] $ExpectFound)

    $repo = Join-Path $tempRoot ('cherry-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
    New-Item -ItemType Directory -Path $repo -Force | Out-Null
    Push-Location $repo
    try {
        git init --quiet .
        Assert-NoRemote
        Set-Content -Path 'a.txt' -Value 'base' -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'base'
        $base = (git rev-parse HEAD).Trim()

        git checkout --quiet -b feature
        Add-Content -Path 'a.txt' -Value 'the feature line'
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'feat: the change (#1629)'
        $feature = (git rev-parse HEAD).Trim()

        # A branch off base, so no assumption about what the default branch is called.
        git checkout --quiet -b deployed $base

        # Diverge before copying. Cherry-picking onto the same parent within the same
        # second reproduces a bit-identical commit, so the "copy" would share the
        # original's sha and the test would prove nothing.
        Set-Content -Path 'b.txt' -Value 'deployed branch moved on' -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'chore: on deployed'

        if ($Mode -eq 'plain') {
            git -c user.email=t@t -c user.name=Test cherry-pick $feature | Out-Null
        } elseif ($Mode -eq 'recorded') {
            git -c user.email=t@t -c user.name=Test cherry-pick -x $feature | Out-Null
        } else {
            Add-Content -Path 'a.txt' -Value 'something else entirely'
            git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'chore: unrelated'
        }
        $head = (git rev-parse HEAD).Trim()

        if ($head -eq $feature) { throw 'fixture is wrong: the copy has the same sha as the original' }

        $libraries = Split-Path $toolPath -Parent
        . (Join-Path $libraries 'lib/ReadOnly.ps1')
        . (Join-Path $libraries 'lib/History.ps1')

        $copy = Find-CopyOfCommit -CommitSha $feature -DeployedSha $head
        $problems = @()
        if ($ExpectFound -and -not $copy) { $problems += 'cherry-pick not detected' }
        if (-not $ExpectFound -and $copy) { $problems += "falsely reported a copy: $($copy.Sha) $($copy.How)" }
        if ($ExpectFound -and $copy -and $copy.Sha -eq $feature.Substring(0, $copy.Sha.Length)) {
            $problems += 'returned the original commit rather than the copy'
        }
    } finally {
        Pop-Location
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'cherry') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'cherry') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

function Test-LocalGitGuard {
    param([string] $Name, [string[]] $Arguments, [bool] $ShouldRefuse)

    # Assert-LocalOnlyGit decides and returns; it never runs git. Nothing here performs
    # a remote operation, and nothing should ever be added that does.
    $refused = $false
    $message = ''
    try {
        Assert-LocalOnlyGit -Arguments $Arguments
    } catch {
        $refused = $true
        $message = $_.Exception.Message
    }

    $problems = @()
    if ($ShouldRefuse -and -not $refused) { $problems += 'expected it to be refused' }
    if (-not $ShouldRefuse -and $refused) { $problems += "unexpectedly refused: $message" }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'localgit') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'localgit') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

function Test-AuthorGuard {
    param([string] $Name, [string] $Author, [bool] $ShouldRefuse)

    # Get-Records refuses before it builds any git arguments, so nothing runs here.
    $libraries = Split-Path $toolPath -Parent
    . (Join-Path $libraries 'lib/ReadOnly.ps1')
    . (Join-Path $libraries 'lib/Report.ps1')

    $refused = $false
    try {
        $null = Get-Records -Arguments @('-1') -Scope @() -Label 'X' -Author $Author
    } catch {
        if ($_.Exception.Message -like '*needs an author*') { $refused = $true } else { throw }
    }

    $problems = @()
    if ($ShouldRefuse -and -not $refused) { $problems += 'an empty author was accepted' }
    if (-not $ShouldRefuse -and $refused) { $problems += 'a real author was refused' }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'author') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'author') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

function Test-CommitFiles {
    param([string] $Name, [string] $Mode, [string[]] $Expect)

    $repo = Join-Path $tempRoot ('files-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
    New-Item -ItemType Directory -Path $repo -Force | Out-Null
    Push-Location $repo
    try {
        git init --quiet .
        Assert-NoRemote
        Set-Content -Path 'base.txt' -Value 'base' -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'base'
        $base = (git rev-parse HEAD).Trim()

        git checkout --quiet -b feature
        Set-Content -Path 'from-feature.txt' -Value 'x' -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'feat: add a file'

        git checkout --quiet -b target $base
        Set-Content -Path 'from-target.txt' -Value 'y' -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'chore: other side'

        if ($Mode -eq 'merge') {
            git -c user.email=t@t -c user.name=Test merge --no-ff --no-edit feature | Out-Null
        }
        $head = (git rev-parse HEAD).Trim()

        $libraries = Split-Path $toolPath -Parent
        . (Join-Path $libraries 'lib/ReadOnly.ps1')
        . (Join-Path $libraries 'lib/GitHub.ps1')

        $files = @(Get-CommitFiles -CommitSha $head)
        $problems = @()
        foreach ($expected in $Expect) {
            if ($files -notcontains $expected) { $problems += "missing '$expected' (got: $($files -join ', '))" }
        }
        if ($Expect.Count -gt 0 -and $files.Count -eq 0) { $problems += 'returned nothing' }
    } finally {
        Pop-Location
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'files') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'files') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

function Test-LaterCommits {
    param([string] $Name, [string] $Mode, [bool] $ExpectFound, [string] $ExpectSubject = '')

    $repo = Join-Path $tempRoot ('later-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
    New-Item -ItemType Directory -Path $repo -Force | Out-Null
    Push-Location $repo
    try {
        git init --quiet .
        Assert-NoRemote
        Set-Content -Path 'a.txt' -Value @('one', 'two', 'three') -Encoding utf8
        Set-Content -Path 'other.txt' -Value 'untouched' -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'base'

        Set-Content -Path 'a.txt' -Value @('one', 'CHANGED BY PR', 'three') -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'feat: the change (#1629)'
        $pr = (git rev-parse HEAD).Trim()

        if ($Mode -eq 'same-file') {
            Set-Content -Path 'a.txt' -Value @('one', 'CHANGED AGAIN', 'three') -Encoding utf8
            git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'chore: first edit'
            Set-Content -Path 'a.txt' -Value @('one', 'CHANGED A THIRD TIME', 'three') -Encoding utf8
            git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'chore: second edit'
        } else {
            Set-Content -Path 'other.txt' -Value 'edited elsewhere' -Encoding utf8
            git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'chore: a different file'
        }
        $head = (git rev-parse HEAD).Trim()

        $libraries = Split-Path $toolPath -Parent
        . (Join-Path $libraries 'lib/ReadOnly.ps1')
        . (Join-Path $libraries 'lib/History.ps1')

        $later = @(Get-LaterCommitsTouching -CommitSha $pr -DeployedSha $head -Paths @('a.txt'))

        $problems = @()
        if ($ExpectFound -and $later.Count -eq 0) { $problems += 'did not notice a commit changing the file' }
        if (-not $ExpectFound -and $later.Count -gt 0) { $problems += "reported a commit touching other files: $($later -join '; ')" }
        if ($later.Count -gt 1) { $problems += "reported $($later.Count) commits; only the first should be reported" }
        if ($ExpectSubject -and $later.Count -gt 0 -and $later[0] -notmatch [regex]::Escape($ExpectSubject)) {
            $problems += "reported '$($later[0])', expected the one containing '$ExpectSubject'"
        }
    } finally {
        Pop-Location
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'later') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'later') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

function Test-RevertedSet {
    param([string] $Name, [bool] $ActuallyRevert, [bool] $ExpectMarked)

    $repo = Join-Path $tempRoot ('revset-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
    New-Item -ItemType Directory -Path $repo -Force | Out-Null
    Push-Location $repo
    try {
        git init --quiet .
        Assert-NoRemote
        Set-Content -Path 'a.txt' -Value 'base' -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'base'

        Set-Content -Path 'a.txt' -Value 'feature' -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'feat: the change (#1629)'
        $feature = (git rev-parse HEAD).Trim()
        $short = (git rev-parse --short HEAD).Trim()

        if ($ActuallyRevert) {
            git -c user.email=t@t -c user.name=Test revert --no-edit $feature | Out-Null
        } else {
            Set-Content -Path 'a.txt' -Value 'feature plus' -Encoding utf8
            git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'chore: follow-up'
        }
        $head = (git rev-parse HEAD).Trim()

        $libraries = Split-Path $toolPath -Parent
        . (Join-Path $libraries 'lib/ReadOnly.ps1')
        . (Join-Path $libraries 'lib/History.ps1')

        $reverted = Get-RevertedCommits -DeployedSha $head
        $marked = Test-CommitWasReverted -ShortSha $short -RevertedCommits $reverted

        $problems = @()
        if ($ExpectMarked -and -not $marked) { $problems += 'a reverted commit was not marked' }
        if (-not $ExpectMarked -and $marked) { $problems += 'a live commit was marked as reverted' }
    } finally {
        Pop-Location
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'revertset') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'revertset') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

function Test-TargetLabel {
    param([string] $JobName, [string] $Expect)

    # Pure string handling: no repo, no API.
    $libraries = Split-Path $toolPath -Parent
    . (Join-Path $libraries 'lib/ReadOnly.ps1')
    . (Join-Path $libraries 'lib/Detection.ps1')
    . (Join-Path $libraries 'lib/GitHub.ps1')

    $actual = Get-TargetLabel -JobName $JobName
    if ($actual -eq $Expect) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'label') + "$JobName -> $actual") -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'label') + "$JobName -> $actual, expected $Expect") -ForegroundColor Red
        $script:failed++
    }
}

function Test-Concerns {
    param([string] $Name, $NewerRun, [bool] $ExpectConcern, [bool] $JobsStarted = $true)

    $libraries = Split-Path $toolPath -Parent
    . (Join-Path $libraries 'lib/ReadOnly.ps1')
    . (Join-Path $libraries 'lib/Detection.ps1')
    . (Join-Path $libraries 'lib/GitHub.ps1')

    # Stubbed: a cancelled run is only a concern if its jobs actually started.
    $script:stubStarted = $JobsStarted
    function Get-RunJobs {
        param([long] $RunId)
        if ($script:stubStarted) {
            return @([pscustomobject]@{ conclusion = 'cancelled'; started_at = '2025-05-14T11:00:05Z' })
        }
        return @([pscustomobject]@{ conclusion = 'cancelled'; started_at = $null })
    }

    # Hand-built run objects, so nothing is fetched.
    $chosenRun = [pscustomobject]@{
        databaseId = 100; createdAt = '2025-05-14T10:00:00Z'; status = 'completed'
        conclusion = 'success'; WorkflowFile = 'deploy-api.yml'; WorkflowToken = 'deployapi'
    }
    $target = [pscustomobject]@{ Label = 'API'; Run = $chosenRun }
    $targets = [ordered]@{ 'API' = $target; 'WEB' = [pscustomobject]@{ Label = 'WEB' } }

    $concerns = @(Get-Concerns -Target $target -Runs @($chosenRun, $NewerRun) -Targets $targets)

    $problems = @()
    if ($ExpectConcern -and $concerns.Count -eq 0) { $problems += 'expected a concern' }
    if (-not $ExpectConcern -and $concerns.Count -gt 0) { $problems += "unexpected concern: $($concerns -join '; ')" }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'concerns') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'concerns') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

function New-TestRun {
    param([long] $Id, [string] $CreatedAt, [string] $Status, [string] $Conclusion, [string] $File, [string] $Token)
    return [pscustomobject]@{
        databaseId = $Id; createdAt = $CreatedAt; status = $Status
        conclusion = $Conclusion; WorkflowFile = $File; WorkflowToken = $Token
    }
}

function Test-TargetBuilding {
    param([string] $Name, $Jobs, [string[]] $ExpectLabels, [string[]] $ExpectSkipped = @(), [bool] $ExpectFailure = $false, $JobEnvironments = @{}, $Scopes = @{}, [string[]] $ExpectScope = @(), [bool] $NeedsLog = $false, $Log = @(), [bool] $CommitExists = $true)

    $libraries = Split-Path $toolPath -Parent
    . (Join-Path $libraries 'lib/ReadOnly.ps1')
    . (Join-Path $libraries 'lib/Detection.ps1')
    . (Join-Path $libraries 'lib/GitHub.ps1')
    . (Join-Path $libraries 'lib/Context.ps1')
    . (Join-Path $libraries 'lib/Deployments.ps1')
    . (Join-Path $libraries 'lib/Targets.ps1')

    # Stubs replace the two functions that would reach out. Nothing in this test can
    # contact GitHub or touch a repository.
    $script:stubJobs = $Jobs
    function Get-RunJobs { param([long] $RunId) return $script:stubJobs }
    $script:stubCommitExists = $CommitExists
    function Test-CommitExists { param([string] $Sha) return $script:stubCommitExists }
    $script:stubLog = $Log
    function Get-RunLog { param([long] $RunId) return $script:stubLog }

    $context = [pscustomobject]@{
        Environment = 'production'
        Aliases     = $script:environmentAliases
        Scopes      = $Scopes
        IgnoreJobs  = @('merge', 'environment', 'notify', 'trigger', 'lint', 'test', 'summary', 'setup', 'prepare', 'complete')
    }
    $run = [pscustomobject]@{
        databaseId = 1; createdAt = '2025-05-14T10:00:00Z'; conclusion = 'success'
        status = 'completed'; headSha = 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678'
        NeedsLog = $NeedsLog; WorkingDirectories = @(); url = 'https://example.invalid/1'
        WorkflowFile = 'deploy.yml'; WorkflowToken = 'deploy'; JobEnvironments = $JobEnvironments
        CalledWorkflows = @{}
    }

    $problems = @()

    if ($ExpectFailure) {
        # Stop-WithMessage exits rather than throwing, and exit cannot be caught, so
        # this case runs in its own process.
        $probePath = Join-Path $tempRoot ('probe-' + [guid]::NewGuid().ToString('N').Substring(0, 6) + '.ps1')
        $jobNames = ($Jobs | ForEach-Object { "'" + $_.name + "'" }) -join ','
        $probe = @"
. '$libraries/lib/ReadOnly.ps1'
. '$libraries/lib/Detection.ps1'
. '$libraries/lib/GitHub.ps1'
. '$libraries/lib/Context.ps1'
. '$libraries/lib/Deployments.ps1'
. '$libraries/lib/Targets.ps1'

function Get-RunJobs {
    param([long] `$RunId)
    return @($jobNames | ForEach-Object {
        [pscustomobject]@{
            name = `$_
            conclusion = 'success'
            steps = @(1, 2, 3 | ForEach-Object { [pscustomobject]@{ name = "s`$_"; conclusion = 'success' } })
        }
    })
}
function Test-CommitExists { param([string] `$Sha) return `$true }

`$context = [pscustomobject]@{
    Environment = 'production'
    Aliases     = `$script:environmentAliases
    IgnoreJobs  = @('notify', 'summary')
}
`$run = [pscustomobject]@{
    databaseId = 1; createdAt = '2025-05-14T10:00:00Z'; conclusion = 'success'
    status = 'completed'; headSha = 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678'
    NeedsLog = `$false; WorkingDirectories = @(); JobEnvironments = @{}; CalledWorkflows = @{}
}
`$null = Get-DeployTargets -Context `$context -Runs @(`$run)
"@
        Set-Content -Path $probePath -Value $probe -Encoding utf8

        $previousPreference = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        $output = (& $script:shellPath @script:shellArguments -File $probePath 2>&1 | Out-String)
        $exitCode = $LASTEXITCODE
        $ErrorActionPreference = $previousPreference

        if ($exitCode -eq 0) { $problems += 'expected a non-zero exit code' }
        if ($output -notmatch 'No deploy jobs recognised') { $problems += "missing the explanation. Output: $output" }
        if ($output -notmatch 'These jobs were skipped') { $problems += 'did not list the skipped jobs' }
    } else {
        $targets = Get-DeployTargets -Context $context -Runs @($run)
        $actual = @($targets.Keys)
        foreach ($label in $ExpectLabels) {
            if ($actual -notcontains $label) { $problems += "missing target '$label' (got: $($actual -join ', '))" }
        }
        if ($actual.Count -ne $ExpectLabels.Count) {
            $problems += "expected $($ExpectLabels.Count) target(s), got $($actual.Count): $($actual -join ', ')"
        }
        foreach ($expected in $ExpectScope) {
            if ($actual.Count -gt 0 -and $targets[$actual[0]].Scope -notcontains $expected) {
                $problems += "expected scope '$expected', got: $($targets[$actual[0]].Scope -join ', ')"
            }
        }
        foreach ($label in $ExpectSkipped) {
            if ($actual.Count -gt 0 -and $targets[$actual[0]].Skipped -notcontains $label) {
                $problems += "expected skipped step '$label'"
            }
        }
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'targets') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'targets') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

function New-TestJob {
    param([string] $Name, [string] $Conclusion = 'success', [int] $StepCount = 3, [string[]] $SkippedSteps = @())
    $steps = @()
    for ($i = 1; $i -le $StepCount; $i++) {
        $steps += [pscustomobject]@{ name = "step $i"; conclusion = 'success' }
    }
    foreach ($skipped in $SkippedSteps) {
        $steps += [pscustomobject]@{ name = $skipped; conclusion = 'skipped' }
    }
    return [pscustomobject]@{ name = $Name; conclusion = $Conclusion; steps = $steps }
}

function Test-SkipReason {
    param([string] $Name, [bool] $NeedsLog, $Log, [bool] $CommitExists, [string] $ExpectReason)

    # Everything stubbed: no API, no repository, nothing fetched.
    $libraries = Split-Path $toolPath -Parent
    $probePath = Join-Path $tempRoot ('reason-' + [guid]::NewGuid().ToString('N').Substring(0, 6) + '.ps1')
    $logLiteral = '@(' + (($Log | ForEach-Object { "'" + $_ + "'" }) -join ',') + ')'

    $probe = @"
. '$libraries/lib/ReadOnly.ps1'
. '$libraries/lib/Detection.ps1'
. '$libraries/lib/GitHub.ps1'
. '$libraries/lib/Context.ps1'
. '$libraries/lib/Deployments.ps1'
. '$libraries/lib/Targets.ps1'

function Get-RunJobs {
    param([long] `$RunId)
    return @([pscustomobject]@{
        name = 'deploy-api'
        conclusion = 'success'
        steps = @(1, 2, 3 | ForEach-Object { [pscustomobject]@{ name = "s`$_"; conclusion = 'success' } })
    })
}
function Get-RunLog { param([long] `$RunId) return $logLiteral }
function Test-CommitExists { param([string] `$Sha) return `$$($CommitExists.ToString().ToLowerInvariant()) }

`$context = [pscustomobject]@{
    Environment = 'production'; Aliases = `$script:environmentAliases; Scopes = @{}
    IgnoreJobs = @('notify')
}
`$run = [pscustomobject]@{
    databaseId = 7; createdAt = '2025-05-14T10:00:00Z'; conclusion = 'success'
    status = 'completed'; headSha = 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678'
    NeedsLog = `$$($NeedsLog.ToString().ToLowerInvariant()); WorkingDirectories = @(); JobEnvironments = @{}; CalledWorkflows = @{}
}
`$null = Get-DeployTargets -Context `$context -Runs @(`$run)
"@
    Set-Content -Path $probePath -Value $probe -Encoding utf8

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = (& $script:shellPath @script:shellArguments -File $probePath 2>&1 | Out-String)
    $ErrorActionPreference = $previousPreference

    if ($output -match [regex]::Escape($ExpectReason)) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'skipreason') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'skipreason') + $Name) -ForegroundColor Red
        Write-Host "          expected '$ExpectReason'" -ForegroundColor Red
        foreach ($line in ($output -split "`r?`n")) { if ($line.Trim()) { Write-Host "          $line" -ForegroundColor DarkGray } }
        $script:failed++
    }
}

function Test-JobEnvironmentMap {
    param([string] $Name, [string] $Yaml, [hashtable] $Expect)

    $libraries = Split-Path $toolPath -Parent
    . (Join-Path $libraries 'lib/ReadOnly.ps1')
    . (Join-Path $libraries 'lib/Detection.ps1')

    $map = (Get-JobMaps -Text $Yaml).Environments
    $problems = @()
    foreach ($job in $Expect.Keys) {
        if ($map[$job] -ne $Expect[$job]) {
            $problems += "$job mapped to '$($map[$job])', expected '$($Expect[$job])'"
        }
    }
    if ($Expect.Count -eq 0 -and $map.Count -gt 0) { $problems += "expected no mapping, got $($map.Count)" }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'jobmap') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'jobmap') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

# Written by the test rather than committed as a fixture: git normalises line endings on
# checkout, so a CRLF file in the repo would not stay CRLF on every machine.
function Test-LineEndings {
    param([string] $Name, [string] $Newline, [string[]] $Expect)

    $libraries = Split-Path $toolPath -Parent
    . (Join-Path $libraries 'lib/ReadOnly.ps1')
    . (Join-Path $libraries 'lib/Detection.ps1')

    if (-not (Test-Path $tempRoot)) { New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null }
    $file = Join-Path $tempRoot ('eol-' + [guid]::NewGuid().ToString('N').Substring(0, 6) + '.yml')
    $yaml = @(
        'name: Deploy Canary'
        'on:'
        '  workflow_dispatch:'
        'jobs:'
        '  ship:'
        '    environment: canary'
        '    steps:'
        '      - uses: actions/checkout@v4'
        '      - name: Publish'
        '        working-directory: services/api'
        '        run: echo x'
    ) -join $Newline
    [System.IO.File]::WriteAllText($file, $yaml)

    $facts = Get-WorkflowFacts -File (Get-Item $file)
    $actual = @(
        "env=$($facts.DeclaredEnvironments -join ',')"
        "name=$($facts.Name)"
        "dir=$($facts.WorkingDirectories -join ',')"
    ) -join ' '

    $problems = @()
    foreach ($expected in $Expect) {
        if ($actual -notmatch [regex]::Escape($expected)) { $problems += "missing '$expected' in: $actual" }
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'lineendings') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'lineendings') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

function Test-CalledWorkflowMap {
    param([string] $Name, [string] $Yaml, [hashtable] $Expect)

    $libraries = Split-Path $toolPath -Parent
    . (Join-Path $libraries 'lib/ReadOnly.ps1')
    . (Join-Path $libraries 'lib/Detection.ps1')

    $maps = Get-JobMaps -Text $Yaml
    $problems = @()
    foreach ($job in $Expect.Keys) {
        if ($maps.CalledWorkflows[$job] -ne $Expect[$job]) {
            $problems += "$job called '$($maps.CalledWorkflows[$job])', expected '$($Expect[$job])'"
        }
    }
    # The two maps are separate so that neither has to be read past the other's keys.
    foreach ($job in $maps.CalledWorkflows.Keys) {
        if ($maps.Environments.ContainsKey($job) -and -not $Expect.ContainsKey($job)) {
            $problems += "$job leaked into the environment map"
        }
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'calledmap') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'calledmap') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

function Test-LocationPreserved {
    param([string] $Name, [string[]] $Arguments)

    $repo = New-FixtureRepo -Fixture 'conventional'
    $before = (Get-Location).Path
    $output = (& $script:shellPath @script:shellArguments -File $toolPath @Arguments -RepoPath $repo -Author 'Fixture Author' 2>&1 | Out-String)
    $after = (Get-Location).Path

    if ($before -eq $after) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'location') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'location') + $Name) -ForegroundColor Red
        Write-Host "          left the caller in $after instead of $before" -ForegroundColor Red
        $script:failed++
    }
}

function Test-FetchOnGuess {
    param([string] $Name, [string[]] $Log, [bool] $ExpectFetch)

    # Nothing here touches a remote: git itself is replaced, so a fetch would only
    # increment a counter.
    $libraries = Split-Path $toolPath -Parent
    . (Join-Path $libraries 'lib/ReadOnly.ps1')
    . (Join-Path $libraries 'lib/Detection.ps1')
    . (Join-Path $libraries 'lib/GitHub.ps1')

    $script:fetchCount = 0
    $script:hasFetched = $false
    function Invoke-ReadOnly {
        param([string] $Command, [string[]] $Arguments)
        if ($Arguments -contains 'fetch') { $script:fetchCount++; return }
        $global:LASTEXITCODE = 1     # nothing resolves, so every candidate misses
        return
    }

    $null = Resolve-CheckoutSha -Log $Log -JobName 'deploy-api'

    $fetched = ($script:fetchCount -gt 0)
    $problems = @()
    if ($ExpectFetch -and -not $fetched) { $problems += 'expected a fetch' }
    if (-not $ExpectFetch -and $fetched) { $problems += "fetched $($script:fetchCount) time(s) while guessing" }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'fetch') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'fetch') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

function Test-DefaultBranchRef {
    param([string] $Name, [string[]] $Refs, [string] $Expect)

    $repo = Join-Path $tempRoot ('ref-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
    New-Item -ItemType Directory -Path $repo -Force | Out-Null
    Push-Location $repo
    try {
        git init --quiet .
        Assert-NoRemote
        Set-Content -Path 'a.txt' -Value 'base' -Encoding utf8
        git add -A; git -c user.email=t@t -c user.name=Test commit --quiet -m 'base'
        $sha = (git rev-parse HEAD).Trim()

        # Fake remote-tracking refs, written directly. Nothing is fetched or contacted.
        foreach ($ref in $Refs) {
            git update-ref "refs/remotes/$ref" $sha
        }

        $libraries = Split-Path $toolPath -Parent
        . (Join-Path $libraries 'lib/ReadOnly.ps1')
        . (Join-Path $libraries 'lib/GitHub.ps1')
        $script:defaultBranchRef = ''

        $actual = Get-DefaultBranchRef
    } finally {
        Pop-Location
    }

    if ($actual -eq $Expect) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'branchref') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'branchref') + $Name) -ForegroundColor Red
        Write-Host "          resolved '$actual', expected '$Expect'" -ForegroundColor Red
        $script:failed++
    }
}

function Test-LabelColumn {
    param([string] $Name, $Targets, [bool] $Expect)

    $libraries = Split-Path $toolPath -Parent
    . (Join-Path $libraries 'lib/ReadOnly.ps1')
    . (Join-Path $libraries 'lib/Deployments.ps1')
    . (Join-Path $libraries 'lib/Targets.ps1')

    $actual = Test-LabelsAreInformative -Targets $Targets
    if ($actual -eq $Expect) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'labelcol') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'labelcol') + $Name) -ForegroundColor Red
        Write-Host "          got $actual, expected $Expect" -ForegroundColor Red
        $script:failed++
    }
}

function Test-Guard {
    param([string] $Name, [string] $InjectedLine, [string[]] $Expect)

    $copyDirectory = Join-Path $tempRoot ('guard-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
    New-Item -ItemType Directory -Path $copyDirectory -Force | Out-Null

    # The whole tool is copied, libraries included, so the tampered copy is runnable.
    Copy-Item -Path (Join-Path (Split-Path $toolPath -Parent) 'deplyd.ps1') -Destination $copyDirectory
    Copy-Item -Path (Join-Path (Split-Path $toolPath -Parent) 'lib') -Destination $copyDirectory -Recurse
    $copy = Join-Path $copyDirectory 'deplyd.ps1'

    # The injected call is wrapped in a function that is never invoked. The audit is
    # textual, so it must still catch it - but if the audit itself ever regresses, a
    # test run cannot execute a real write.
    $source = @(Get-Content -Path $toolPath)
    $wrapped = @('', 'function Deplyd-TamperProbe {', "    $InjectedLine", '}')
    Set-Content -Path $copy -Value ($source + $wrapped) -Encoding utf8

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = (& $script:shellPath @script:shellArguments -File $copy check 2>&1 | Out-String)
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $previousPreference

    $problems = @()
    if ($exitCode -eq 0) { $problems += 'expected a non-zero exit code' }
    foreach ($expected in $Expect) {
        if ($output -notmatch [regex]::Escape($expected)) { $problems += "missing '$expected'" }
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'guard') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'guard') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        Write-Host '        --- output ---' -ForegroundColor DarkGray
        foreach ($line in ($output -split "`r?`n")) {
            if ($line.Trim()) { Write-Host "        $line" -ForegroundColor DarkGray }
        }
        $script:failed++
    }
}

$script:groupWidth = 2 + (@(
    @($cases | ForEach-Object { $_.Fixture }) +
    @([regex]::Matches(
        (Get-Content -Path $PSCommandPath -Raw), "Format-TestGroup '([a-z-]+)'"
    ) | ForEach-Object { $_.Groups[1].Value })
) | Measure-Object -Property Length -Maximum).Maximum

Write-Host ''
Write-Host "deplyd detection tests" -ForegroundColor Cyan
Write-Host ''

foreach ($case in $cases) {
    $repo = New-FixtureRepo -Fixture $case.Fixture
    # An explicit author keeps the suite independent of whatever git identity the
    # machine happens to have. Without it these fail anywhere git config user.name is
    # unset, which is most CI containers and a fresh WSL install.
    $arguments = $script:shellArguments + @('-File', $toolPath) + $case.Arguments + @('-RepoPath', $repo, '-Author', 'Fixture Author')
    # Redirecting a native command's stderr while ErrorActionPreference is Stop turns
    # each line into a terminating NativeCommandError, so relax it around the call.
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = (& $script:shellPath @arguments 2>&1 | Out-String)
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $previousPreference

    $problems = @()

    $shouldFail = $false
    if ($case.ContainsKey('ExpectFail')) { $shouldFail = [bool] $case.ExpectFail }

    if ($shouldFail -and $exitCode -eq 0) {
        $problems += 'expected a non-zero exit code'
    }
    if (-not $shouldFail -and $exitCode -ne 0) {
        $problems += "exited with $exitCode"
    }
    foreach ($expected in $case.Expect) {
        if ($output -notmatch [regex]::Escape($expected)) {
            $problems += "missing '$expected'"
        }
    }
    foreach ($unwanted in $case.Reject) {
        if ($output -match [regex]::Escape($unwanted)) {
            $problems += "should not print '$unwanted'"
        }
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup $case.Fixture) + $case.Name) -ForegroundColor Green
        $passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup $case.Fixture) + $case.Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        Write-Host '        --- output ---' -ForegroundColor DarkGray
        foreach ($line in ($output -split "`r?`n")) {
            if ($line.Trim()) { Write-Host "        $line" -ForegroundColor DarkGray }
        }
        $failed++
    }
}


# The real shape: gh exports the job name, a step column, then a timestamped message.
$checkoutLog = @(
    "deploy-back-end`tUNKNOWN STEP`t2026-09-24T07:22:20.0000000Z Syncing repository",
    "deploy-back-end`tUNKNOWN STEP`t2026-09-24T07:22:48.0000000Z SECOND",
    "deploy-front-end`tUNKNOWN STEP`t2026-09-24T07:38:25.0000000Z FIRST"
)

Test-ShaResolution -Name 'takes the bare sha printed by checkout in that job' `
    -JobName 'deploy-back-end' -LogTemplate $checkoutLog -ExpectWhich 'second' -ExpectExact $true

Test-ShaResolution -Name 'does not take another job checkout sha' `
    -JobName 'deploy-front-end' -LogTemplate $checkoutLog -ExpectWhich 'first' -ExpectExact $true

# One matrix leg's name tokenises to a prefix of the other's, so matching the job name
# anywhere in the line hands "deploy (api)" whatever "deploy (api, eu-west-1)" checked
# out. Only the first column may decide whose line it is.
$matrixLog = @(
    "deploy (api, eu-west-1)`tUNKNOWN STEP`t2026-09-24T07:22:20.0000000Z Syncing repository",
    "deploy (api, eu-west-1)`tUNKNOWN STEP`t2026-09-24T07:22:48.0000000Z FIRST",
    "deploy (api)`tUNKNOWN STEP`t2026-09-24T07:38:25.0000000Z SECOND"
)

Test-ShaResolution -Name 'a matrix leg does not take a longer leg sha' `
    -JobName 'deploy (api)' -LogTemplate $matrixLog -ExpectWhich 'second' -ExpectExact $true

Test-ShaResolution -Name 'the longer matrix leg keeps its own' `
    -JobName 'deploy (api, eu-west-1)' -LogTemplate $matrixLog -ExpectWhich 'first' -ExpectExact $true

# Jobs from a called workflow arrive as "Deploy API / deploy-api" from the jobs API,
# while the log column carries only one of the two.
Test-ShaResolution -Name 'a called workflow job matches on its last segment' `
    -JobName 'Deploy API / deploy-api' `
    -LogTemplate @("deploy-api`tUNKNOWN STEP`t2026-09-24T07:22:48.0000000Z SECOND") `
    -ExpectWhich 'second' -ExpectExact $true

# A sha mentioned mid-sentence is not a checkout line and must not be trusted.
Test-ShaResolution -Name 'ignores a sha embedded in prose, falling back and saying so' `
    -JobName 'deploy-back-end' `
    -LogTemplate @("deploy-back-end`tUNKNOWN STEP`t2026-09-24T07:22:09.0000000Z Commit: SECOND is being merged") `
    -ExpectWhich 'second' -ExpectExact $false

Test-ShaResolution -Name 'returns nothing when no known commit appears' `
    -JobName 'deploy-back-end' `
    -LogTemplate @("deploy-back-end`tUNKNOWN STEP`t2026-09-24T07:22:09.0000000Z nothing here") `
    -ExpectWhich 'none' -ExpectExact $true

Test-RevertDetection -Name 'detects a git revert of the commit' -ActuallyRevert $true -ExpectDetected $true
Test-RevertDetection -Name 'does not cry revert for an unrelated follow-up' -ActuallyRevert $false -ExpectDetected $false

Test-CherryPickDetection -Name 'finds a plain cherry-pick by its diff' -Mode 'plain' -ExpectFound $true
Test-CherryPickDetection -Name 'finds a cherry-pick -x by its recorded origin' -Mode 'recorded' -ExpectFound $true
Test-CherryPickDetection -Name 'does not match an unrelated commit' -Mode 'unrelated' -ExpectFound $false

Test-LocalGitGuard -Name 'refuses git push' -Arguments @('push', 'origin', 'main') -ShouldRefuse $true
Test-LocalGitGuard -Name 'refuses git fetch' -Arguments @('fetch', 'origin') -ShouldRefuse $true
Test-LocalGitGuard -Name 'refuses git clone' -Arguments @('clone', 'some-path') -ShouldRefuse $true
Test-LocalGitGuard -Name 'refuses git remote add' -Arguments @('remote', 'add', 'origin', 'somewhere') -ShouldRefuse $true
Test-LocalGitGuard -Name 'refuses an https URL argument' -Arguments @('log', 'https://example.com/x.git') -ShouldRefuse $true
Test-LocalGitGuard -Name 'refuses an scp-style remote argument' -Arguments @('log', 'git@example.com:org/repo.git') -ShouldRefuse $true
Test-LocalGitGuard -Name 'allows listing remotes' -Arguments @('remote') -ShouldRefuse $false
Test-LocalGitGuard -Name 'allows local commit' -Arguments @('commit', '--quiet', '-m', 'x') -ShouldRefuse $false
Test-LocalGitGuard -Name 'allows cherry-pick' -Arguments @('cherry-pick', 'abc1234') -ShouldRefuse $false

Test-AuthorGuard -Name 'refuses an empty author, which would match everyone' -Author '' -ShouldRefuse $true

Test-CommitFiles -Name 'lists files of an ordinary commit' -Mode 'plain' -Expect @('from-target.txt')
Test-CommitFiles -Name 'lists files of a merge commit, which show alone omits' -Mode 'merge' -Expect @('from-feature.txt')

Test-LaterCommits -Name 'notices a later commit changing the same file' -Mode 'same-file' `
    -ExpectFound $true -ExpectSubject 'first edit'

Test-LaterCommits -Name 'ignores a later commit touching only other files' -Mode 'other-file' `
    -ExpectFound $false

Test-TargetLabel -JobName 'deploy (api)' -Expect 'API'
Test-TargetLabel -JobName 'deploy-api (eu-west-1)' -Expect 'API-EU-WEST-1'
Test-TargetLabel -JobName 'quick-deploy (api, staging)' -Expect 'API-STAGING'
Test-TargetLabel -JobName 'Deploy Back-End / deploy-back-end' -Expect 'BACK-END'
Test-TargetLabel -JobName 'deploy' -Expect 'DEPLOY'
Test-TargetLabel -JobName 'build-and-deploy' -Expect 'BUILD'
Test-TargetLabel -JobName 'deploy-to-production' -Expect 'PRODUCTION'

Test-Concerns -Name 'a newer failed run for the same workflow' `
    -NewerRun (New-TestRun -Id 101 -CreatedAt '2025-05-14T11:00:00Z' -Status 'completed' -Conclusion 'failure' -File 'deploy-api.yml' -Token 'deployapi') `
    -ExpectConcern $true

Test-Concerns -Name 'a newer run still in progress' `
    -NewerRun (New-TestRun -Id 102 -CreatedAt '2025-05-14T11:00:00Z' -Status 'in_progress' -Conclusion '' -File 'deploy-api.yml' -Token 'deployapi') `
    -ExpectConcern $true

Test-Concerns -Name 'a newer failed run for a different target' `
    -NewerRun (New-TestRun -Id 103 -CreatedAt '2025-05-14T11:00:00Z' -Status 'completed' -Conclusion 'failure' -File 'deploy-web.yml' -Token 'deployweb') `
    -ExpectConcern $false

Test-Concerns -Name 'a newer successful run' `
    -NewerRun (New-TestRun -Id 104 -CreatedAt '2025-05-14T11:00:00Z' -Status 'completed' -Conclusion 'success' -File 'deploy-api.yml' -Token 'deployapi') `
    -ExpectConcern $false

Test-Concerns -Name 'an older failed run' `
    -NewerRun (New-TestRun -Id 99 -CreatedAt '2025-05-14T09:00:00Z' -Status 'completed' -Conclusion 'failure' -File 'deploy-api.yml' -Token 'deployapi') `
    -ExpectConcern $false

Test-Concerns -Name 'a newer failed combined run naming neither target' `
    -NewerRun (New-TestRun -Id 105 -CreatedAt '2025-05-14T11:00:00Z' -Status 'completed' -Conclusion 'failure' -File 'deploy.yml' -Token 'deploy') `
    -ExpectConcern $true

Test-Concerns -Name 'a newer run cancelled after its jobs started' `
    -NewerRun (New-TestRun -Id 106 -CreatedAt '2025-05-14T11:00:00Z' -Status 'completed' -Conclusion 'cancelled' -File 'deploy-api.yml' -Token 'deployapi') `
    -JobsStarted $true -ExpectConcern $true

Test-Concerns -Name 'a newer run cancelled while still queued' `
    -NewerRun (New-TestRun -Id 107 -CreatedAt '2025-05-14T11:00:00Z' -Status 'completed' -Conclusion 'cancelled' -File 'deploy-api.yml' -Token 'deployapi') `
    -JobsStarted $false -ExpectConcern $false

Test-TargetBuilding -Name 'one deploy job becomes one target' `
    -Jobs @((New-TestJob -Name 'deploy-api')) -ExpectLabels @('API')

Test-TargetBuilding -Name 'matrix legs become separate targets' `
    -Jobs @((New-TestJob -Name 'deploy (api)'), (New-TestJob -Name 'deploy (web)')) `
    -ExpectLabels @('API', 'WEB')

Test-TargetBuilding -Name 'plumbing jobs are not targets' `
    -Jobs @((New-TestJob -Name 'deploy-api'), (New-TestJob -Name 'notify-slack'), (New-TestJob -Name 'summary')) `
    -ExpectLabels @('API')

Test-TargetBuilding -Name 'a failed job is not a target' `
    -Jobs @((New-TestJob -Name 'deploy-api'), (New-TestJob -Name 'deploy-web' -Conclusion 'failure')) `
    -ExpectLabels @('API')

Test-TargetBuilding -Name 'a job with fewer than three steps is not a target' `
    -Jobs @((New-TestJob -Name 'deploy-api'), (New-TestJob -Name 'deploy-web' -StepCount 2)) `
    -ExpectLabels @('API')

Test-TargetBuilding -Name 'skipped steps are recorded on the target' `
    -Jobs @((New-TestJob -Name 'deploy-api' -SkippedSteps @('Publish web bundle'))) `
    -ExpectLabels @('API') -ExpectSkipped @('Publish web bundle')

Test-TargetBuilding -Name 'stops when every job is plumbing' `
    -Jobs @((New-TestJob -Name 'notify-slack'), (New-TestJob -Name 'summary')) `
    -ExpectLabels @() -ExpectFailure $true

Test-TargetBuilding -Name 'a staged pipeline reports only the asked-for environment' `
    -Jobs @((New-TestJob -Name 'deploy-staging'), (New-TestJob -Name 'deploy-production')) `
    -JobEnvironments @{ 'deploy-staging' = 'staging'; 'deploy-production' = 'production' } `
    -ExpectLabels @('PRODUCTION')

Test-TargetBuilding -Name 'a job with no declared environment is still a target' `
    -Jobs @((New-TestJob -Name 'deploy-api')) -JobEnvironments @{ 'other-job' = 'staging' } `
    -ExpectLabels @('API')

Test-TargetBuilding -Name 'a scopes override sets the target scope' `
    -Jobs @((New-TestJob -Name 'deploy-api')) -Scopes @{ 'API' = @('services/api') } `
    -ExpectLabels @('API') -ExpectScope @('services/api')

Test-SkipReason -Name 'says so when the run log has expired' `
    -NeedsLog $true -Log @() -CommitExists $true `
    -ExpectReason 'has no readable log'

Test-SkipReason -Name 'says so when the log holds no usable commit' `
    -NeedsLog $true -Log @("deploy-api`tUNKNOWN STEP`t2025-05-14T10:00:00Z nothing useful") -CommitExists $false `
    -ExpectReason 'no commit in the run log'

Test-SkipReason -Name 'says so when the deployed commit is not in the clone' `
    -NeedsLog $false -Log @() -CommitExists $false `
    -ExpectReason 'is not in this clone'

$twoSpaceJobs = @'
jobs:
  deploy-staging:
    environment:
      name: staging
  deploy-production:
    environment:
      name: production
'@

$fourSpaceJobs = @'
jobs:
    deploy-staging:
        environment:
            name: staging
    deploy-production:
        environment:
            name: production
'@

$inlineJobs = @'
jobs:
  deploy-web:
    environment: production
'@

Test-JobEnvironmentMap -Name 'two-space indentation' -Yaml $twoSpaceJobs `
    -Expect @{ 'deploy-staging' = 'staging'; 'deploy-production' = 'production' }

Test-JobEnvironmentMap -Name 'four-space indentation' -Yaml $fourSpaceJobs `
    -Expect @{ 'deploy-staging' = 'staging'; 'deploy-production' = 'production' }

Test-JobEnvironmentMap -Name 'an inline environment' -Yaml $inlineJobs `
    -Expect @{ 'deploy-web' = 'production' }

$callingJobs = @'
name: Deploy
jobs:
  deploy-api:
    name: Deploy API
    uses: ./.github/workflows/reusable-deploy.yml
  deploy-web:
    environment: production
    runs-on: ubuntu-latest
'@

Test-LineEndings -Name 'a unix workflow file is read' -Newline "`n" `
    -Expect @('env=canary', 'name=Deploy Canary', 'dir=services/api')

Test-LineEndings -Name 'a windows workflow file is read the same way' -Newline "`r`n" `
    -Expect @('env=canary', 'name=Deploy Canary', 'dir=services/api')

Test-CalledWorkflowMap -Name 'a job that only calls another workflow' -Yaml $callingJobs `
    -Expect @{ 'deploy-api' = 'reusable-deploy.yml'; 'Deploy API' = 'reusable-deploy.yml' }

Test-JobEnvironmentMap -Name 'a calling job contributes no environment' -Yaml $callingJobs `
    -Expect @{ 'deploy-web' = 'production' }

Test-LocationPreserved -Name 'a full run leaves the caller where it was' -Arguments @('config')
Test-LocationPreserved -Name 'an early exit leaves the caller where it was' -Arguments @('help')

# A pinned action SHA and a cache key are 40-hex but are not commits in this repo.
$noisyLog = @(
    "deploy-api`tUNKNOWN STEP`t2025-05-14T10:00:00Z uses actions/checkout@1111111111111111111111111111111111111111",
    "deploy-api`tUNKNOWN STEP`t2025-05-14T10:00:01Z cache key 2222222222222222222222222222222222222222"
)
Test-FetchOnGuess -Name 'does not fetch while guessing at hex in a log' -Log $noisyLog -ExpectFetch $false

Test-RevertedSet -Name 'marks a reverted commit in the list' -ActuallyRevert $true -ExpectMarked $true
Test-RevertedSet -Name 'leaves an unreverted commit unmarked' -ActuallyRevert $false -ExpectMarked $false

Test-DefaultBranchRef -Name 'prefers origin/HEAD when it is set' `
    -Refs @('origin/HEAD', 'origin/main') -Expect 'origin/HEAD'

Test-DefaultBranchRef -Name 'falls back to origin/main' `
    -Refs @('origin/main') -Expect 'origin/main'

Test-DefaultBranchRef -Name 'falls back to origin/master' `
    -Refs @('origin/master') -Expect 'origin/master'

Test-DefaultBranchRef -Name 'reports nothing when no default branch exists' `
    -Refs @() -Expect ''

Test-LabelColumn -Name 'two targets with scopes: labels mean something' `
    -Targets ([ordered]@{
        'API' = [pscustomobject]@{ Scope = @('services/api') }
        'WEB' = [pscustomobject]@{ Scope = @('services/web') }
    }) -Expect $true

Test-LabelColumn -Name 'two targets without scopes: every row would say COMBINED' `
    -Targets ([ordered]@{
        'API' = [pscustomobject]@{ Scope = @() }
        'WEB' = [pscustomobject]@{ Scope = @() }
    }) -Expect $false

Test-LabelColumn -Name 'a single target: the label never varies' `
    -Targets ([ordered]@{ 'DEPLOY' = [pscustomobject]@{ Scope = @('src') } }) -Expect $false

Test-Guard -Name 'refuses a raw git push added to the source' `
    -InjectedLine 'git push origin main' `
    -Expect @('Refused', 'outside the read-only gateway', 'git push origin main')

Test-Guard -Name 'refuses a gh write added to the source' `
    -InjectedLine 'gh pr merge 1 --squash' `
    -Expect @('Refused', 'outside the read-only gateway', 'gh pr merge')

Test-Guard -Name 'refuses Invoke-Expression' `
    -InjectedLine 'Invoke-Expression "git push"' `
    -Expect @('Refused', 'Invoke-Expression can run anything')

Test-Guard -Name 'refuses Start-Process' `
    -InjectedLine 'Start-Process git -ArgumentList push' `
    -Expect @('Refused', 'Start-Process can launch any executable')

Test-Guard -Name 'refuses the call operator on a variable' `
    -InjectedLine '$tool = 1; & $tool push' `
    -Expect @('Refused', 'hides what runs')

Test-Guard -Name 'refuses spawning another shell' `
    -InjectedLine 'cmd.exe /c "git push"' `
    -Expect @('Refused', 'spawning a shell bypasses this audit')

Test-Allowlist -Name 'refuses gh pr comment' -Command 'gh' `
    -Arguments @('pr', 'comment', '1', '--body', 'hi') -ShouldRefuse $true `
    -Expect @('not on the read-only allowlist')

Test-Allowlist -Name 'refuses gh pr merge' -Command 'gh' `
    -Arguments @('pr', 'merge', '1', '--squash') -ShouldRefuse $true

Test-Allowlist -Name 'refuses gh api -X POST' -Command 'gh' `
    -Arguments @('api', '-X', 'POST', 'repos/o/r/deployments') -ShouldRefuse $true `
    -Expect @('is a write')

Test-Allowlist -Name 'refuses gh api with a field' -Command 'gh' `
    -Arguments @('api', 'repos/o/r/issues', '-f', 'title=x') -ShouldRefuse $true `
    -Expect @('implies a POST')

Test-Allowlist -Name 'refuses git push' -Command 'git' `
    -Arguments @('push', 'origin', 'main') -ShouldRefuse $true `
    -Expect @('not on the read-only allowlist')

Test-Allowlist -Name 'refuses git config assignment' -Command 'git' `
    -Arguments @('config', 'user.name', 'someone') -ShouldRefuse $true `
    -Expect @('would write a value')

Test-Allowlist -Name 'refuses git fetch --prune' -Command 'git' `
    -Arguments @('fetch', 'origin', '--prune') -ShouldRefuse $true

Test-Allowlist -Name 'allows git log' -Command 'git' `
    -Arguments @('log', '-1') -ShouldRefuse $false

Test-Allowlist -Name 'allows a plain gh api GET' -Command 'gh' `
    -Arguments @('api', 'repos/o/r/actions/runs') -ShouldRefuse $false

Test-Allowlist -Name 'allows reading git config' -Command 'git' `
    -Arguments @('config', 'user.name') -ShouldRefuse $false


# --- the shell wrapper ------------------------------------------------------------
#
# The profile defines deplyd as a function, and it must hand every word over untouched.
# Two ways of getting that wrong have already shipped: an advanced function claims
# -ErrorAction and makes -E ambiguous, and splatting an array passes every element
# positionally, so "-Author" arrives as a value and lands in the next positional
# parameter with nothing complaining. The second is why these run the real script and
# read what it bound, not a stub echoing the argument array.

function New-WrapperProbe {
    param([string] $Target, [string[]] $Invocation)

    if (-not (Test-Path $tempRoot)) { New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null }
    $shellInit = Join-Path (Split-Path $PSScriptRoot -Parent) 'shell-init.ps1'
    $probePath = Join-Path $tempRoot ('wrapper-' + [guid]::NewGuid().ToString('N').Substring(0, 6) + '.ps1')
    Set-Content -Path $probePath -Encoding UTF8 -Value @(
        "`$ErrorActionPreference = 'Stop'"
        ". '$shellInit'"
        "`$script:deplydScript = '$Target'"
        $Invocation
    )
    return $probePath
}

function Write-WrapperResult {
    param([string] $Name, [string[]] $Problems, [string] $Output)

    if ($Problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'wrapper') + $Name) -ForegroundColor Green
        $script:passed++
        return
    }
    Write-Host ('  FAIL  ' + (Format-TestGroup 'wrapper') + $Name) -ForegroundColor Red
    foreach ($problem in $Problems) { Write-Host "          $problem" -ForegroundColor Red }
    foreach ($line in ($Output -split "`r?`n")) {
        if ($line.Trim()) { Write-Host "        $line" -ForegroundColor DarkGray }
    }
    $script:failed++
}

# Positional words only, where echoing the array is enough to see them in order.
function Test-WrapperWords {
    param([string] $Name, [string] $Invocation, [string] $Expect)

    $stub = Join-Path $tempRoot 'stub-words.ps1'
    if (-not (Test-Path $tempRoot)) { New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null }
    Set-Content -Path $stub -Value "'ARGS:' + (`$args -join '|')" -Encoding UTF8

    $probePath = New-WrapperProbe -Target $stub -Invocation $Invocation
    $output = (& $script:shellPath @script:shellArguments -File $probePath 2>&1 | Out-String)

    $actual = ''
    foreach ($line in ($output -split "`r?`n")) { if ($line -match '^ARGS:') { $actual = $line.Trim() } }

    $problems = @()
    if ($actual -ne $Expect) { $problems += "expected $Expect, got $actual" }
    Write-WrapperResult -Name $Name -Problems $problems -Output $output
}

# Flags, checked by what deplyd.ps1 itself bound and printed.
function Test-WrapperBinding {
    param([string] $Name, [string] $Invocation, [string[]] $Expect, [string[]] $Reject = @())

    $repo = New-FixtureRepo -Fixture 'conventional'
    $probePath = New-WrapperProbe -Target $toolPath -Invocation ($Invocation + " -RepoPath '$repo'")

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = (& $script:shellPath @script:shellArguments -File $probePath 2>&1 | Out-String)
    $ErrorActionPreference = $previousPreference

    $problems = @()
    foreach ($expected in $Expect) {
        if ($output -notmatch [regex]::Escape($expected)) { $problems += "missing '$expected'" }
    }
    foreach ($unwanted in $Reject) {
        if ($output -match [regex]::Escape($unwanted)) { $problems += "should not print '$unwanted'" }
    }
    Write-WrapperResult -Name $Name -Problems $problems -Output $output
}

Test-WrapperWords -Name 'passes a verb and its argument through' `
    -Invocation 'deplyd pr 1629' -Expect 'ARGS:pr|1629'

Test-WrapperWords -Name 'passes a third word through, still positional' `
    -Invocation "deplyd remember author 'Ada Lovelace'" -Expect 'ARGS:remember|author|Ada Lovelace'

Test-WrapperWords -Name 'passes nothing when nothing was given' `
    -Invocation 'deplyd' -Expect 'ARGS:'

Test-WrapperBinding -Name '-A reaches the script as an author, not as a word' `
    -Invocation "deplyd config -A 'Fixture Person'" `
    -Expect @('Fixture Person') -Reject @('Unknown command', 'Ambiguous')

Test-WrapperBinding -Name '-E reaches the script as an environment' `
    -Invocation 'deplyd config -E stag -A Someone' `
    -Expect @('Selected       staging') -Reject @('Selected       production')

Test-WrapperBinding -Name 'both short flags at once' `
    -Invocation "deplyd config -E staging -A 'Fixture Person'" `
    -Expect @('staging', 'Fixture Person')

Test-WrapperBinding -Name 'the long names work too' `
    -Invocation "deplyd config -Environment staging -Author 'Fixture Person'" `
    -Expect @('staging', 'Fixture Person')

Test-WrapperBinding -Name 'a verb and a flag together' `
    -Invocation "deplyd environments -A 'Fixture Person'" `
    -Expect @('production', 'staging')

Test-WrapperBinding -Name 'answers to dp as well' `
    -Invocation "dp config -A 'Fixture Person'" `
    -Expect @('Fixture Person')

# A completer that prints is worse than one that returns nothing: deplyd explains
# itself to a person when it cannot read a repo, and that lands across the prompt.
function Test-CompleterIsQuiet {
    param([string] $Name)

    $outside = Join-Path $tempRoot ('not-a-repo-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
    New-Item -ItemType Directory -Path $outside -Force | Out-Null

    $probePath = New-WrapperProbe -Target $toolPath -Invocation @(
        "Set-Location '$outside'"
        "`$found = @(Get-DeplydName -What 'environments')"
        "Write-Output ('COUNT:' + `$found.Count)"
    )

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = (& $script:shellPath @script:shellArguments -File $probePath 2>&1 | Out-String)
    $ErrorActionPreference = $previousPreference

    $noise = @($output -split "`r?`n" | Where-Object { $_.Trim() -and $_ -notmatch '^COUNT:' })
    $problems = @()
    if ($output -notmatch 'COUNT:0') { $problems += 'expected no completions outside a repo' }
    if ($noise.Count -gt 0) { $problems += "printed $($noise.Count) line(s) it should have swallowed" }
    Write-WrapperResult -Name $Name -Problems $problems -Output $output
}

Test-CompleterIsQuiet -Name 'a completer outside a repo prints nothing at all'

Test-WrapperBinding -Name 'the self-check says what share of the source it audited' `
    -Invocation "deplyd check" `
    -Expect @('Files audited', ' of ', 'Not audited', 'install.ps1')

Test-WrapperBinding -Name 'an unknown flag is refused in the tool own voice' `
    -Invocation "deplyd config -Bogus 1 -A 'Fixture Person'" `
    -Expect @('Unknown option -Bogus', 'Options:', '-RepoPath') `
    -Reject @('Deploy workflows', 'ParameterBindingException', 'CategoryInfo')

Test-WrapperBinding -Name 'a common parameter is not mistaken for an unknown flag' `
    -Invocation "deplyd config -A 'Fixture Person' -Verbose" `
    -Expect @('Fixture Person') -Reject @('Unknown option')

# A failed run and a repo with nothing to offer both come back empty. Only one of them
# may be remembered, or a half-broken install reads as "this repo has no environments".
function Test-CompletionCache {
    param([string] $Name)

    $repo = New-FixtureRepo -Fixture 'conventional'
    $outside = Join-Path $tempRoot ('not-a-repo-' + [guid]::NewGuid().ToString('N').Substring(0, 6))
    New-Item -ItemType Directory -Path $outside -Force | Out-Null

    $probePath = New-WrapperProbe -Target $toolPath -Invocation @(
        "Set-Location '$outside'"
        "`$null = Get-DeplydName -What 'environments'"
        "Write-Output ('AFTER-FAILURE:' + `$script:deplydNames.Count)"
        "Set-Location '$repo'"
        "Write-Output ('FOUND:' + ((Get-DeplydName -What 'environments') -join ','))"
        "Write-Output ('CACHED:' + `$script:deplydNames.Count)"
        # A workflow added afterwards must not be hidden by what was cached before it.
        "Set-Content -Path (Join-Path '$repo' '.github/workflows/deploy-canary.yml') -Encoding UTF8 -Value @("
        "    'name: Deploy Canary'"
        "    'on:'"
        "    '  workflow_dispatch:'"
        "    'jobs:'"
        "    '  ship:'"
        "    '    environment: canary'"
        "    '    steps:'"
        "    '      - uses: actions/checkout@v4'"
        ")"
        "Write-Output ('AGAIN:' + ((Get-DeplydName -What 'environments') -join ','))"
        # The override replaces the list outright, and is the edit most likely to
        # follow seeing a wrong one. Writing it must not leave the wrong one up.
        "Set-Content -Path (Join-Path '$repo' '.deplyd.json') -Encoding UTF8 -Value '{ `"environments`": { `"sandbox`": { `"workflows`": [`"deploy-canary.yml`"] } } }'"
        "Write-Output ('OVERRIDDEN:' + ((Get-DeplydName -What 'environments') -join ','))"
    )

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = (& $script:shellPath @script:shellArguments -File $probePath 2>&1 | Out-String)
    $ErrorActionPreference = $previousPreference

    $problems = @()
    if ($output -notmatch 'AFTER-FAILURE:0') { $problems += 'a failed lookup was cached' }
    if ($output -notmatch 'FOUND:production,staging') { $problems += 'did not read the repo' }
    if ($output -notmatch 'CACHED:1') { $problems += 'a successful lookup was not cached' }
    if ($output -notmatch 'AGAIN:.*canary') { $problems += 'a new workflow stayed hidden behind the cache' }
    if ($output -notmatch 'OVERRIDDEN:sandbox') { $problems += 'a new .deplyd.json stayed hidden behind the cache' }
    Write-WrapperResult -Name $Name -Problems $problems -Output $output
}

Test-CompletionCache -Name 'only successes are remembered, and not past a workflow or override edit'

# --- init -------------------------------------------------------------------------
#
# The one file deplyd writes inside a repo. It must hold what detection found, must
# not overwrite by accident, and what it writes must be read back the same way.

function Get-InitPath {
    param([string] $Repo)

    $libraries = Split-Path $toolPath -Parent
    . (Join-Path $libraries 'lib/ReadOnly.ps1')
    . (Join-Path $libraries 'lib/Context.ps1')
    return (Get-PrivateOverridePath -RepoRoot $Repo)
}

function Clear-InitFile {
    param([string] $Path)
    if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Force }
}

function Test-Init {
    param([string] $Name, [string[]] $Arguments, [bool] $Pre, [string[]] $Expect, [string[]] $Reject = @())

    $repo = New-FixtureRepo -Fixture 'conventional'
    $path = Get-InitPath -Repo $repo
    Clear-InitFile -Path $path
    if ($Pre) {
        $directory = Split-Path $path -Parent
        if (-not (Test-Path -LiteralPath $directory)) { New-Item -ItemType Directory -Path $directory -Force | Out-Null }
        Set-Content -LiteralPath $path -Value '{ "deployPattern": "kept" }' -Encoding UTF8
    }

    $arguments = $script:shellArguments + @('-File', $toolPath) + $Arguments +
        @('-RepoPath', $repo, '-Author', 'Fixture Author')

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = (& $script:shellPath @arguments 2>&1 | Out-String)
    $ErrorActionPreference = $previousPreference

    $written = ''
    if (Test-Path -LiteralPath $path) { $written = (Get-Content -LiteralPath $path -Raw) }
    $subject = $output + "`n---WRITTEN---`n" + $written

    $problems = @()
    foreach ($expected in $Expect) {
        if ($subject -notmatch [regex]::Escape($expected)) { $problems += "missing '$expected'" }
    }
    foreach ($unwanted in $Reject) {
        if ($subject -match [regex]::Escape($unwanted)) { $problems += "should not contain '$unwanted'" }
    }
    # The whole point of the location: nothing appears in the working tree.
    if (Test-Path -LiteralPath (Join-Path $repo '.deplyd.json')) {
        $problems += 'it wrote into the repository'
    }
    Clear-InitFile -Path $path

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'init') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'init') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        foreach ($line in ($subject -split "`r?`n")) {
            if ($line.Trim()) { Write-Host "        $line" -ForegroundColor DarkGray }
        }
        $script:failed++
    }
}

Test-Init -Name 'writes what detection found, outside the repo' -Arguments @('init') -Pre $false `
    -Expect @('deployPattern', 'production', 'staging', 'deploy-production-api.yml', 'ignoreJobs', 'scopes')

# The name is the whole mechanism: a file moved into the repo keeping its own name is
# invisible to deplyd, so the instruction has to carry the rename, not imply it.
Test-Init -Name 'says how to share it, with the rename spelled out' -Arguments @('init') -Pre $false `
    -Expect @('.deplyd.json', 'the only name deplyd looks for', '\.deplyd.json"')

Test-Init -Name 'refuses to overwrite one that is already there' -Arguments @('init') -Pre $true `
    -Expect @('already a settings file', 'deplyd init -Force', '"deployPattern": "kept"') `
    -Reject @('deploy-production-api.yml')

# -Force rewrites from the current conclusion, so a hand-set value survives and the rest
# is filled in around it. Regenerating from bare detection would delete the corrections
# the file exists to hold.
Test-Init -Name '-Force rewrites it, keeping what it already set' -Arguments @('init', '-Force') -Pre $true `
    -Expect @('Wrote', 'deploy-production-api.yml', '"kept"')

function Test-InitEncoding {
    param([string] $Name)

    $repo = New-FixtureRepo -Fixture 'conventional'
    $path = Get-InitPath -Repo $repo
    Clear-InitFile -Path $path

    $arguments = $script:shellArguments + @('-File', $toolPath, 'init') +
        @('-RepoPath', $repo, '-Author', 'Fixture Author')
    $null = (& $script:shellPath @arguments 2>&1 | Out-String)

    $problems = @()
    if (-not (Test-Path -LiteralPath $path)) {
        $problems += 'nothing was written'
    } else {
        # Set-Content -Encoding utf8 writes a BOM on 5.1 and none on 7, so the same
        # command would produce different bytes on the two versions deplyd supports.
        $bytes = [System.IO.File]::ReadAllBytes($path)
        if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
            $problems += 'it starts with a UTF-8 BOM'
        }
        # Line feeds throughout, including the last byte. Environment::NewLine would
        # put the version difference back in the one place nobody looks.
        for ($i = 1; $i -lt $bytes.Length; $i++) {
            if ($bytes[$i] -eq 10 -and $bytes[$i - 1] -eq 13) {
                $problems += "it has a CRLF at byte $i"
                break
            }
        }
        if ($bytes[-1] -ne 10) { $problems += 'it does not end with a line feed' }
        Clear-InitFile -Path $path
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'init') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'init') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        $script:failed++
    }
}

Test-InitEncoding -Name 'writes the same bytes on 5.1 and 7, with no BOM'

# A file committed at the repo root is the one the team sees, so it has to win.
function Test-SharedOverrideWins {
    param([string] $Name)

    $repo = New-FixtureRepo -Fixture 'conventional'
    $path = Get-InitPath -Repo $repo
    $directory = Split-Path $path -Parent
    if (-not (Test-Path -LiteralPath $directory)) { New-Item -ItemType Directory -Path $directory -Force | Out-Null }
    Set-Content -LiteralPath $path -Value '{ "deployPattern": "private-only" }' -Encoding UTF8
    Set-Content -LiteralPath (Join-Path $repo '.deplyd.json') -Value '{ "deployPattern": "shared-wins" }' -Encoding UTF8

    $arguments = $script:shellArguments + @('-File', $toolPath, 'config') +
        @('-RepoPath', $repo, '-Author', 'Fixture Author')
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = (& $script:shellPath @arguments 2>&1 | Out-String)
    $ErrorActionPreference = $previousPreference
    Clear-InitFile -Path $path

    $problems = @()
    if ($output -notmatch 'committed in the repo') { $problems += 'did not report the repo file as the one in use' }
    if ($output -match [regex]::Escape($path)) { $problems += 'used the private file instead' }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'init') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'init') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        foreach ($line in ($output -split "`r?`n")) {
            if ($line.Trim()) { Write-Host "        $line" -ForegroundColor DarkGray }
        }
        $script:failed++
    }
}

Test-SharedOverrideWins -Name 'a .deplyd.json committed in the repo takes precedence'

# The one path where deplyd edits a file git is tracking. It takes an explicit -Force,
# but the refusal and the confirmation have to say which file they mean and what git
# makes of it, or the warning that matters reads exactly like the one that does not.
# Tracked-ness is asked of git, never inferred from where the file sits.
function Test-SharedInit {
    param([string] $Name, [string[]] $Arguments, [bool] $Commit, [string[]] $Expect, [string[]] $Reject = @())

    $repo = New-FixtureRepo -Fixture 'conventional'
    $shared = Join-Path $repo '.deplyd.json'
    Set-Content -LiteralPath $shared -Value '{ "deployPattern": "shared" }' -Encoding UTF8

    if ($Commit) {
        # Relaxed around the calls: git warns about line endings on Windows, and under
        # the Stop preference that warning would end the suite rather than the commit.
        $committing = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        Push-Location $repo
        git -c core.autocrlf=false add -A 2>&1 | Out-Null
        git -c core.autocrlf=false -c user.name=Fixture -c user.email=fixture@example.com commit -m 'add settings' 2>&1 | Out-Null
        Pop-Location
        $ErrorActionPreference = $committing
    }

    $arguments = $script:shellArguments + @('-File', $toolPath) + $Arguments +
        @('-RepoPath', $repo, '-Author', 'Fixture Author')

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = (& $script:shellPath @arguments 2>&1 | Out-String)
    $ErrorActionPreference = $previousPreference

    $written = (Get-Content -LiteralPath $shared -Raw)
    $subject = $output + "`n---WRITTEN---`n" + $written

    $problems = @()
    foreach ($expected in $Expect) {
        if ($subject -notmatch [regex]::Escape($expected)) { $problems += "missing '$expected'" }
    }
    foreach ($unwanted in $Reject) {
        if ($subject -match [regex]::Escape($unwanted)) { $problems += "should not contain '$unwanted'" }
    }
    # A copy beside deplyd would never take effect while this file exists.
    if (Test-Path -LiteralPath (Get-InitPath -Repo $repo)) {
        $problems += 'it wrote a private file that the repo file would shadow'
    }
    # git's own complaint about an untracked path is deplyd's question, not the user's.
    if ($subject -match 'did not match any file') {
        $problems += 'it leaked the tracked-ness probe to the console'
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'init') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'init') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        foreach ($line in ($subject -split "`r?`n")) {
            if ($line.Trim()) { Write-Host "        $line" -ForegroundColor DarkGray }
        }
        $script:failed++
    }
}

Test-SharedInit -Name 'an untracked repo file is not called committed' -Commit $false `
    -Arguments @('init') `
    -Expect @('already a .deplyd.json in this repository', 'git is not tracking it', '"deployPattern": "shared"') `
    -Reject @('already a settings file', 'git is tracking', 'git diff', 'deploy-production-api.yml')

Test-SharedInit -Name 'a tracked repo file is named as tracked' -Commit $true `
    -Arguments @('init') `
    -Expect @('git is tracking it', 'shows up as a modification', 'git diff', '"deployPattern": "shared"') `
    -Reject @('already a settings file', 'deploy-production-api.yml')

Test-SharedInit -Name '-Force on a tracked file says it now shows as modified' -Commit $true `
    -Arguments @('init', '-Force') `
    -Expect @('git is tracking that file', 'shows as modified', 'git diff', 'deploy-production-api.yml') `
    -Reject @('changes nothing on its own', 'your repository is left alone')

Test-SharedInit -Name '-Force on an untracked file does not claim git tracks it' -Commit $false `
    -Arguments @('init', '-Force') `
    -Expect @('not tracking it', 'deploy-production-api.yml') `
    -Reject @('shows as modified', 'your repository is left alone')

# What it writes has to be a file deplyd itself accepts, or the scaffold is a trap.
function Test-InitRoundTrip {
    param([string] $Name)

    $repo = New-FixtureRepo -Fixture 'conventional'
    $path = Get-InitPath -Repo $repo
    Clear-InitFile -Path $path

    $base = $script:shellArguments + @('-File', $toolPath)
    $tail = @('-RepoPath', $repo, '-Author', 'Fixture Author')

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $before = (& $script:shellPath @base config @tail 2>&1 | Out-String)
    $null = (& $script:shellPath @base init @tail 2>&1 | Out-String)
    $after = (& $script:shellPath @base config @tail 2>&1 | Out-String)
    $ErrorActionPreference = $previousPreference
    Clear-InitFile -Path $path

    # Config reports the settings file once it exists. That is it saying what it read,
    # not a different conclusion, so those lines are not part of the comparison.
    $strip = { param($text) (($text -split "`r?`n" | Where-Object {
        $_ -notmatch '^(Repo|Overrides|No overrides)\s' -and $_ -notmatch '^Scope override'
    }) -join "`n").Trim() }

    $problems = @()
    if ((& $strip $before) -ne (& $strip $after)) {
        $problems += 'config changed after writing the file it says it found'
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'init') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'init') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        Write-Host '        --- before ---' -ForegroundColor DarkGray
        foreach ($line in ((& $strip $before) -split "`n")) { Write-Host "        $line" -ForegroundColor DarkGray }
        Write-Host '        --- after ---' -ForegroundColor DarkGray
        foreach ($line in ((& $strip $after) -split "`n")) { Write-Host "        $line" -ForegroundColor DarkGray }
        $script:failed++
    }
}

Test-InitRoundTrip -Name 'writing it changes nothing about what deplyd concludes'

# --- the deployment record as a second source -------------------------------------
#
# The run log says what checkout resolved; the deployment record says what the
# deployment was created for. Two records made differently, so agreement is worth
# something and disagreement is worth saying. The record also outlives the log, which
# GitHub deletes, so it is the only source left for an older deploy.

function Test-DeploymentCrossCheck {
    param(
        [string] $Name,
        [string] $LogSha,          # 'first', 'second', or '' for a log with nothing usable
        [string] $DeploymentSha,   # 'first', 'second', or '' for no record at all
        [bool] $Cached,            # whether the deployment walk already happened
        [string] $RecordEnvironment = 'production',  # which environment the record is for
        [string[]] $Expect,
        [string[]] $Reject = @()
    )

    # New-FixtureRepo only runs git init, so both commits are made here. Two of them,
    # so the log and the deployment record can disagree about something real.
    $repo = New-FixtureRepo -Fixture 'conventional'
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    Push-Location $repo
    git -c core.autocrlf=false add -A 2>&1 | Out-Null
    git -c core.autocrlf=false -c user.name=Fixture -c user.email=f@example.com commit -m first 2>&1 | Out-Null
    $first = (git rev-parse HEAD 2>$null | Out-String).Trim()

    Set-Content -LiteralPath (Join-Path $repo 'second.txt') -Value 'second' -Encoding UTF8
    git -c core.autocrlf=false add -A 2>&1 | Out-Null
    git -c core.autocrlf=false -c user.name=Fixture -c user.email=f@example.com commit -m second 2>&1 | Out-Null
    $second = (git rev-parse HEAD 2>$null | Out-String).Trim()
    Pop-Location
    $ErrorActionPreference = $previous

    if (-not $first -or -not $second -or $first -eq $second) {
        throw "fixture commits not created: first=[$first] second=[$second]"
    }

    $shas = @{ first = $first; second = $second; '' = '' }
    $logLine = ''
    if ($LogSha -eq 'loose') {
        # A sha in prose rather than on a line of its own: taken, but marked a guess.
        $logLine = "deploy-api`tUNKNOWN STEP`t2026-09-24T07:22:48.0000000Z Commit: $first is being merged"
    } elseif ($LogSha) {
        $logLine = "deploy-api`tUNKNOWN STEP`t2026-09-24T07:22:48.0000000Z $($shas[$LogSha])"
    } else {
        $logLine = "deploy-api`tUNKNOWN STEP`t2026-09-24T07:22:48.0000000Z nothing usable here"
    }

    $libraries = Split-Path $toolPath -Parent
    $probePath = Join-Path $tempRoot ('deploy-' + [guid]::NewGuid().ToString('N').Substring(0, 6) + '.ps1')
    Set-Content -Path $probePath -Encoding UTF8 -Value @(
        "`$ErrorActionPreference = 'Stop'"
        ". '$libraries/lib/ReadOnly.ps1'"
        ". '$libraries/lib/Detection.ps1'"
        ". '$libraries/lib/GitHub.ps1'"
        ". '$libraries/lib/Deployments.ps1'"
        ". '$libraries/lib/Targets.ps1'"
        ". '$libraries/lib/Context.ps1'"
        "Set-Location '$repo'"
        # The deployment walk is the only thing allowed to reach the API here; whether
        # it has already run is the difference between free corroboration and none.
        "`$script:deploymentShas = @{}"
        # Cached means the environment walk already happened and corroboration is free.
        # Uncached with a record means only a fallback may pay for the call.
        $(if ($Cached -and $DeploymentSha) { "Add-DeploymentSha -RunId 77 -Environment '$RecordEnvironment' -Sha '$($shas[$DeploymentSha])'" } else { '' })
        $(if (-not $Cached -and $DeploymentSha) {
            "function Request-Deployments { param([string] `$Environment) Add-DeploymentSha -RunId 77 -Environment '$RecordEnvironment' -Sha '$($shas[$DeploymentSha])'; return @(77) }"
        } else {
            "function Request-Deployments { param([string] `$Environment) return @() }"
        })
        "function Get-RunLog { param([long] `$RunId) return @('$logLine') }"
        "`$run = [pscustomobject]@{ databaseId = 77; url = 'u'; headSha = '$first'; createdAt = '2026-09-24T07:00:00Z'"
        "    WorkflowFile = 'deploy.yml'; WorkflowToken = 'deploy'; JobEnvironments = @{}; CalledWorkflows = @{}"
        "    NeedsLog = `$true; WorkingDirectories = @() }"
        "`$job = [pscustomobject]@{ name = 'deploy-api'; conclusion = 'success'; steps = @("
        "    [pscustomobject]@{ name = 'a'; conclusion = 'success' }"
        "    [pscustomobject]@{ name = 'b'; conclusion = 'success' }"
        "    [pscustomobject]@{ name = 'c'; conclusion = 'success' }) }"
        "`$context = [pscustomobject]@{ Environment = 'production'; Aliases = @{}; IgnoreJobs = @();"
        "    Scopes = @{}; Facts = @() }"
        "`$script:ignoredJobs = @{}"
        "`$target = New-TargetFromJob -Context `$context -Run `$run -Job `$job -Known ([ordered]@{})"
        "if (-not `$target) { Write-Output ('SKIPPED:' + (`$script:ignoredJobs.Values -join '; ')); exit 0 }"
        "`$which = 'other'"
        "if (`$target.Sha -eq '$first') { `$which = 'first' } elseif (`$target.Sha -eq '$second') { `$which = 'second' }"
        "Write-Output ('SHA:' + `$which)"
        "Write-Output ('SOURCE:' + `$target.ShaSource)"
        "Write-Output ('EXACT:' + `$target.ShaIsExact)"
        "Write-Output ('CORROBORATED:' + `$target.Corroborated)"
        "Write-Output ('WARNING:' + `$target.ShaWarning)"
    )

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = (& $script:shellPath @script:shellArguments -File $probePath 2>&1 | Out-String)
    $ErrorActionPreference = $previousPreference

    $problems = @()
    foreach ($expected in $Expect) {
        if ($output -notmatch [regex]::Escape($expected)) { $problems += "missing '$expected'" }
    }
    foreach ($unwanted in $Reject) {
        if ($output -match [regex]::Escape($unwanted)) { $problems += "should not contain '$unwanted'" }
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'deployment') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'deployment') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        foreach ($line in ($output -split "`r?`n")) {
            if ($line.Trim()) { Write-Host "        $line" -ForegroundColor DarkGray }
        }
        $script:failed++
    }
}

# Agreement between two records made differently is the confidence signal.
Test-DeploymentCrossCheck -Name 'the deployment record agreeing is reported as corroboration' `
    -LogSha 'first' -DeploymentSha 'first' -Cached $true `
    -Expect @('SHA:first', 'SOURCE:the run log', 'EXACT:True', 'CORROBORATED:True', 'WARNING:')

# Neither record is simply wrong, so deplyd says so rather than picking one.
Test-DeploymentCrossCheck -Name 'a disagreement is surfaced and drops the certainty' `
    -LogSha 'first' -DeploymentSha 'second' -Cached $true `
    -Expect @('SHA:first', 'EXACT:False', 'CORROBORATED:False', 'deployment record names') `
    -Reject @('CORROBORATED:True')

# Corroboration is never worth an API call of its own.
Test-DeploymentCrossCheck -Name 'no record already in hand means no lookup and no claim' `
    -LogSha 'first' -DeploymentSha '' -Cached $false `
    -Expect @('SHA:first', 'SOURCE:the run log', 'EXACT:True', 'CORROBORATED:False', 'WARNING:')

# The record outlives the log, so it is the only source left for an older deploy.
Test-DeploymentCrossCheck -Name 'an unusable log falls back to the deployment record' `
    -LogSha '' -DeploymentSha 'second' -Cached $false `
    -Expect @('SHA:second', 'SOURCE:the deployment record', 'EXACT:False', 'run log could not name the commit')

# A guessed commit is where a second record earns its API call.
Test-DeploymentCrossCheck -Name 'a loose guess is worth fetching a record to check' `
    -LogSha 'loose' -DeploymentSha 'second' -Cached $false `
    -Expect @('EXACT:False', 'deployment record names')

# One run, two environments, two checkouts: the staged-pipeline shape. A run-wide key
# would hand this target the other environment's commit and invent a disagreement.
Test-DeploymentCrossCheck -Name 'another environment record is not borrowed to compare against' `
    -LogSha 'first' -DeploymentSha 'second' -Cached $true -RecordEnvironment 'staging' `
    -Expect @('SHA:first', 'EXACT:True', 'CORROBORATED:False', 'WARNING:') `
    -Reject @('deployment record names')

# With neither source, saying nothing is the only honest answer.
Test-DeploymentCrossCheck -Name 'no log and no record skips the job and says why' `
    -LogSha '' -DeploymentSha '' -Cached $false `
    -Expect @('SKIPPED:', 'deployment record')

# --- not covered ------------------------------------------------------------------
#
# "It changed nothing inside any deployed target" is true of a genuinely unrelated
# change and of a wrong scope, and those need different actions. Showing the comparison
# that was made - every target, what it covers, the files - is what tells them apart,
# and a wrong scope is the most common way detection misleads.

function Test-NotCovered {
    param([string] $Name, $Scopes, [string[]] $Files, [string[]] $Expect, [string[]] $Reject = @())

    $libraries = Split-Path $toolPath -Parent
    $probePath = Join-Path $tempRoot ('covered-' + [guid]::NewGuid().ToString('N').Substring(0, 6) + '.ps1')

    $targetLines = @()
    foreach ($label in $Scopes.Keys) {
        $scope = @($Scopes[$label])
        $rendered = '@()'
        if ($scope.Count -gt 0) { $rendered = "@('" + ($scope -join "','") + "')" }
        $targetLines += "`$targets['$label'] = [pscustomobject]@{ Label = '$label'; Scope = $rendered }"
    }

    Set-Content -Path $probePath -Encoding UTF8 -Value (@(
        "`$ErrorActionPreference = 'Stop'"
        ". '$libraries/lib/ReadOnly.ps1'"
        ". '$libraries/lib/Report.ps1'"
        ". '$libraries/lib/Targets.ps1'"
        ". '$libraries/commands/Show-PullRequest.ps1'"
        "`$targets = [ordered]@{}"
    ) + $targetLines + @(
        "`$report = [ordered]@{ number = 412; title = 'a change'; status = 'not covered'"
        "    commit = '0123456789abcdef0123456789abcdef01234567'; commitSource = 'api'"
        "    files = @('" + ($Files -join "','") + "') }"
        "Show-PullRequest -Targets `$targets -Report `$report"
    ))

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = (& $script:shellPath @script:shellArguments -File $probePath 2>&1 | Out-String)
    $ErrorActionPreference = $previousPreference

    $problems = @()
    foreach ($expected in $Expect) {
        if ($output -notmatch [regex]::Escape($expected)) { $problems += "missing '$expected'" }
    }
    foreach ($unwanted in $Reject) {
        if ($output -match [regex]::Escape($unwanted)) { $problems += "should not contain '$unwanted'" }
    }

    if ($problems.Count -eq 0) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'notcovered') + $Name) -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'notcovered') + $Name) -ForegroundColor Red
        foreach ($problem in $problems) { Write-Host "          $problem" -ForegroundColor Red }
        foreach ($line in ($output -split "`r?`n")) {
            if ($line.Trim()) { Write-Host "        $line" -ForegroundColor DarkGray }
        }
        $script:failed++
    }
}

# The comparison itself, so a wrong scope is visible rather than deduced.
Test-NotCovered -Name 'lists every target, what it covers, and the files' `
    -Scopes ([ordered]@{ API = @('services/api'); WEB = @('services/web') }) `
    -Files @('services/worker/main.go', 'README.md') `
    -Expect @(
        'NOT COVERED', 'Targets considered:',
        'API', 'covers services/api', 'WEB', 'covers services/web',
        'Files in this pull request:', 'services/worker/main.go', 'README.md',
        'its scope is wrong', 'deplyd init', '"scopes"'
    )

# The example points at the directory of a file that was actually in the diff, so it
# can be pasted rather than translated.
Test-NotCovered -Name 'the scopes example names a path from this pull request' `
    -Scopes ([ordered]@{ API = @('services/api') }) `
    -Files @('services/worker/main.go') `
    -Expect @('services/worker')

# Split-Path -Parent on a root-level file returns nothing, which would print an empty
# scope: a line that looks pasteable and silently does nothing.
Test-NotCovered -Name 'a root-level file does not produce an empty scope' `
    -Scopes ([ordered]@{ API = @('services/api') }) `
    -Files @('README.md') `
    -Expect @('README.md') -Reject @('[""]')

Test-NotCovered -Name 'it prefers the first file that has a directory above it' `
    -Scopes ([ordered]@{ API = @('services/api') }) `
    -Files @('README.md', 'services/worker/main.go') `
    -Expect @('["services/worker"]') -Reject @('[""]')

# A target covering everything cannot fail to cover a file, so saying "fix your scope"
# there would send the reader after something that is not wrong.
Test-NotCovered -Name 'a target covering everything is called a bug, not a scope' `
    -Scopes ([ordered]@{ ALL = @() }) `
    -Files @('anything.txt') `
    -Expect @('covers everything, yet matched nothing', 'a bug in deplyd') `
    -Reject @('its scope is wrong')

# --- machine-readable output ------------------------------------------------------

function Test-VerdictExitCode {
    param([string] $Status, [bool] $Uncertain, [int] $Expect)

    $libraries = Split-Path $toolPath -Parent
    . (Join-Path $libraries 'lib/ReadOnly.ps1')
    . (Join-Path $libraries 'lib/Json.ps1')

    $actual = Get-VerdictExitCode -Status $Status -Uncertain $Uncertain
    $label = $Status
    if ($Uncertain) { $label += ' (uncertain)' }

    if ($actual -eq $Expect) {
        Write-Host ('  PASS  ' + (Format-TestGroup 'exitcode') + "$label -> $actual") -ForegroundColor Green
        $script:passed++
    } else {
        Write-Host ('  FAIL  ' + (Format-TestGroup 'exitcode') + "$label -> $actual, expected $Expect") -ForegroundColor Red
        $script:failed++
    }
}

# Only a sound "live" may be zero. Anything a gate should stop on has to be non-zero,
# including the states that are nobody's fault, like a pull request never merged.
Test-VerdictExitCode -Status 'live' -Uncertain $false -Expect 0
Test-VerdictExitCode -Status 'not live' -Uncertain $false -Expect 2
Test-VerdictExitCode -Status 'not covered' -Uncertain $false -Expect 2
Test-VerdictExitCode -Status 'commit missing locally' -Uncertain $false -Expect 2
Test-VerdictExitCode -Status 'reverted' -Uncertain $false -Expect 3
Test-VerdictExitCode -Status 'not merged' -Uncertain $false -Expect 4
Test-VerdictExitCode -Status 'not found' -Uncertain $false -Expect 5

# The one the report already flags and the gate used to be told nothing about.
Test-VerdictExitCode -Status 'live' -Uncertain $true -Expect 6

# Uncertainty cannot rescue a verdict, only qualify a live one.
Test-VerdictExitCode -Status 'not live' -Uncertain $true -Expect 2
Test-VerdictExitCode -Status 'reverted' -Uncertain $true -Expect 3

Write-Host ''
if ($failed -eq 0) {
    Write-Host "$passed passed" -ForegroundColor Green
} else {
    Write-Host "$passed passed, $failed failed" -ForegroundColor Red
}

if ($KeepTemp) {
    Write-Host "Fixtures kept in $tempRoot" -ForegroundColor DarkGray
} elseif (Test-Path $tempRoot) {
    Remove-Item -Path $tempRoot -Recurse -Force
}

Write-Host ''
if ($failed -gt 0) { exit 1 }
