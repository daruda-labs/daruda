#Requires -Version 5.1
param(
    [Parameter(Mandatory)][string]$FilePath,
    [AllowEmptyString()][string[]]$ArgumentList = @()
)

$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/windows/hidden-process.ps1"
exit (Invoke-HiddenConsoleCommand -FilePath $FilePath -ArgumentList $ArgumentList)
