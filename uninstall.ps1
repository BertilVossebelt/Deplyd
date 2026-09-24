#Requires -Version 5.1
<#
    Removes deplyd from the PowerShell profile. A thin wrapper so the file is easy to
    find; the work lives in install.ps1, which keeps one copy of the profile handling.
#>
[CmdletBinding()]
param()

& (Join-Path $PSScriptRoot 'install.ps1') -Uninstall
