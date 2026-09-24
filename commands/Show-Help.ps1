function Show-Help {
    Write-Host ''
    Write-Host 'deplyd - which commit an environment was last deployed from' -ForegroundColor Cyan
    Write-Host ''
    Write-Host 'COMMANDS' -ForegroundColor Cyan
    Write-Host '  deplyd status               the last deployed commit, and your changes in it'
    Write-Host '  deplyd pr 412               whether one pull request is live'
    Write-Host '  deplyd authors              names that -A accepts'
    Write-Host '  deplyd environments         environments that -E accepts'
    Write-Host '  deplyd config               what detection concluded about this repo'
    Write-Host '  deplyd init                 write that to a settings file, to correct by hand'
    Write-Host '  deplyd check                prove it can only read'
    Write-Host '  deplyd help                 this text, also shown when you run deplyd alone'
    Write-Host ''
    Write-Host '  deplyd remember author "Ada"'
    Write-Host '  deplyd remember environment staging'
    Write-Host '  deplyd remember repo <path>'
    Write-Host ''
    Write-Host 'Commands can be shortened while they stay unambiguous: env, auth, conf.' -ForegroundColor DarkGray
    Write-Host 'The tool itself shortens to dp, so: dp status.' -ForegroundColor DarkGray
    Write-Host ''
    Write-Host 'OPTIONS' -ForegroundColor Cyan
    foreach ($option in $script:knownOptions.Keys) {
        Write-Host ("  {0,-15}{1}" -f $option, $script:knownOptions[$option])
    }
    Write-Host ''
    Write-Host 'PER TARGET' -ForegroundColor Cyan
    Write-Host '  PUBLISHED   built and released; no newer deploy failing or in flight'
    Write-Host '  UNCERTAIN   a newer deploy for that target did not complete'
    Write-Host '  SKIPPED     steps the run skipped - changes to those are not live'
    Write-Host ''
    Write-Host 'PER PULL REQUEST' -ForegroundColor Cyan
    Write-Host '  LIVE        the commit, or an equivalent cherry-pick, is in the deployed commit'
    Write-Host '  REVERTED    it shipped, then was undone before the deployed commit'
    Write-Host '  NOT LIVE    neither the commit nor an equivalent change is there'
    Write-Host '  NOT MERGED  still open, or closed without merging'
    Write-Host '  NOT COVERED it changed no path any target covers; names them so you can check'
    Write-Host ''
    Write-Host 'EXIT CODES for deplyd pr' -ForegroundColor Cyan
    Write-Host '  0  live            3  reverted        5  no such pull request'
    Write-Host '  2  not live        4  not merged      6  live, but see UNCERTAIN'
    Write-Host '  1  deplyd could not run'
    Write-Host ''
    Write-Host 'Everything is detected from .github/workflows. Run "deplyd config" to see'
    Write-Host 'what it found, and "deplyd init" to write it somewhere you can correct it.'
    Write-Host ''
    Write-Host "Requires the GitHub CLI: $(Get-GhInstallHint), then gh auth login." -ForegroundColor DarkGray
    Write-Host ''
}
