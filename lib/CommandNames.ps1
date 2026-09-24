<#
    The command names, and nothing else, so the shell wrapper can read them without
    loading the tool. Two lists would drift the day one of them grows.
#>

$script:knownCommands = [ordered]@{
    status       = 'the last deployed commit, and your changes in it'
    pr           = 'whether one pull request is live'
    authors      = 'names that -A accepts'
    environments = 'environments that -E accepts'
    config       = 'what detection concluded about this repo'
    init         = 'write that conclusion to a settings file, to correct by hand'
    remember     = 'keep a default author, environment or repo'
    check        = 'prove it can only read'
    help         = 'this text'
}

# Not listed anywhere, and not reachable by prefix: the shell calls it, people do not.
$script:hiddenCommands = @('complete')

$script:knownOptions = [ordered]@{
    '-E  <env>'     = 'environment, or a prefix of one: -E prod, -E stag'
    '-A  <name>'    = 'author to filter on, default: git config user.name'
    '-T  <n>'       = 'how many changes to list, default 10'
    '-S  <n>'       = 'skip this many, for paging'
    '-RepoPath <p>' = 'repo to inspect, default: the current directory'
    '-Json'         = 'machine-readable output, for status and pr'
    '-Force'        = 'let init rewrite an existing .deplyd.json'
}
