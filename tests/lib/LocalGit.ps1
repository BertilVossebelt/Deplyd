<#
    Local-only git for the test suite.

    Tests build throwaway repositories, so unlike deplyd itself they must be allowed to
    write - commit, checkout, cherry-pick. What they must never do is reach a remote.

    Dot-sourcing this file defines a `git` function that shadows the real executable, so
    every git call in the suite passes through it without any call site being rewritten,
    including ones added later. Anything that could contact a remote is refused before
    it runs.
#>

$script:realGit = (Get-Command git -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1)
if (-not $script:realGit) { throw 'git was not found on PATH' }
$script:realGitPath = $script:realGit.Source

# Verbs that can talk to a server, or can point the repository at one.
$script:remoteCapableVerbs = @(
    'push', 'pull', 'fetch', 'clone', 'ls-remote', 'submodule', 'send-pack', 'receive-pack',
    'upload-pack', 'upload-archive', 'request-pull', 'daemon', 'credential', 'svn', 'p4',
    'imap-send', 'send-email', 'format-patch', 'archive', 'bundle'
)

# update-ref writes a local ref. Fixtures use it to fake remote-tracking branches
# without a remote; it cannot reach anything.
function Assert-LocalOnlyGit {
    param([string[]] $Arguments)

    $positional = @($Arguments | Where-Object { $_ -notlike '-*' })
    $verb = ''
    if ($positional.Count -gt 0) { $verb = $positional[0] }

    # Listing remotes is a read and is used to assert there are none.
    $isRemoteListing = ($verb -eq 'remote' -and $positional.Count -eq 1)

    if (-not $isRemoteListing) {
        if ($verb -eq 'remote') {
            throw "tests refuse 'git remote $($positional[1])': it would point the fixture at a remote"
        }
        if ($script:remoteCapableVerbs -contains $verb) {
            throw "tests refuse 'git $verb': it can reach a remote"
        }
    }

    foreach ($argument in $Arguments) {
        if ($argument -match '^(https?|ssh|git|ftp)://') {
            throw "tests refuse an argument naming a remote URL: $argument"
        }
        if ($argument -match '^[\w.+-]+@[\w.-]+:') {
            throw "tests refuse an argument naming an scp-style remote: $argument"
        }
    }
}

function git {
    $arguments = @($args)
    Assert-LocalOnlyGit -Arguments $arguments
    & $script:realGitPath @arguments
}

# Belt and braces: even if something slipped past the check above, git itself is told
# that no transport protocol is permitted, so any remote operation fails outright.
$env:GIT_ALLOW_PROTOCOL = 'none'
