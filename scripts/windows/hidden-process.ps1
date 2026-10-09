#Requires -Version 5.1

if (-not ('Daruda.Scripts.HiddenConsole' -as [type])) {
    Add-Type -Path "$PSScriptRoot/hidden-process.cs"
}

function Invoke-HiddenConsoleCommand {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [AllowEmptyString()][string[]]$ArgumentList = @(),
        [switch]$CaptureOutput
    )
    $executable = @(Get-Command $FilePath -CommandType Application -ErrorAction Stop)[0].Source
    $result = [Daruda.Scripts.HiddenConsole]::Run(
        $executable, $ArgumentList, (Get-Location).ProviderPath, $CaptureOutput.IsPresent)
    if ($CaptureOutput) { return $result }
    $result.ExitCode
}
