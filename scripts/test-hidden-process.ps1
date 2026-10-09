#Requires -Version 5.1
param(
    [ValidateSet('Test', 'Host', 'Console', 'Descendant', 'Arguments', 'Streams', 'Flood', 'Runner')]
    [string]$Probe = 'Test',
    [Parameter(ValueFromRemainingArguments = $true)][AllowEmptyString()][string[]]$Values
)

$ErrorActionPreference = 'Stop'
$executable = (Get-Process -Id $PID).Path

if ($Probe -eq 'Runner') {
    & "$PSScriptRoot/run-hidden.ps1" -FilePath $executable -ArgumentList @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath, '-Probe', 'Streams')
    exit $LASTEXITCODE
}

if ($Probe -eq 'Arguments') {
    [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
    [Console]::Out.Write((ConvertTo-Json -InputObject $Values -Compress))
    [Console]::Error.Write('stderr: ' + [char]0xD55C + [char]0xAE00)
    exit 7
}
if ($Probe -eq 'Streams') {
    [Console]::Out.WriteLine('streamed stdout')
    [Console]::Error.WriteLine('streamed stderr')
    exit 9
}
if ($Probe -eq 'Flood') {
    for ($index = 0; $index -lt 10000; $index++) {
        [Console]::Out.WriteLine('out' + $index)
        [Console]::Error.WriteLine('err' + $index)
    }
    exit 0
}

if ($Probe -eq 'Test' -or $Probe -eq 'Console') {
    $start = [Diagnostics.ProcessStartInfo]::new($executable)
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $nextProbe = if ($Probe -eq 'Test') { 'Host' } else { 'Descendant' }
    $start.Arguments = '-NoProfile -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" -Probe ' + $nextProbe
    $start.CreateNoWindow = $Probe -eq 'Test'
    $process = [Diagnostics.Process]::Start($start)
    try {
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) { throw ($stdout.Result + $stderr.Result) }
        if ($Probe -eq 'Test') {
            if (-not $stdout.Result.Contains('streamed stdout') -or -not $stderr.Result.Contains('streamed stderr')) {
                throw 'Streaming output was lost'
            }
            Write-Host $stdout.Result.Trim()
            Write-Host 'Both streamed outputs passed; parent had no console.'
            exit 0
        }
        $descendant = $stdout.Result | ConvertFrom-Json
    } finally {
        $process.Dispose()
    }
}

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class HiddenRunnerProbe {
    [DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow();
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
}
'@
$window = [HiddenRunnerProbe]::GetConsoleWindow()
if ($Probe -eq 'Console' -or $Probe -eq 'Descendant') {
    $state = @{ Window = $window.ToInt64(); Visible = [HiddenRunnerProbe]::IsWindowVisible($window) }
    if ($Probe -eq 'Console') { $state.Descendant = $descendant }
    $state | ConvertTo-Json -Compress
    exit 0
}

if ($window -ne [IntPtr]::Zero) { throw 'Host unexpectedly has a console' }
. "$PSScriptRoot/windows/hidden-process.ps1"
$probeArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath, '-Probe')
$result = Invoke-HiddenConsoleCommand -FilePath $executable -ArgumentList ($probeArgs + 'Console') -CaptureOutput
if ($result.ExitCode -ne 0) { throw $result.Stderr }
$console = $result.Stdout | ConvertFrom-Json
if ($console.Window -eq 0 -or $console.Visible -or $console.Descendant.Visible -or $console.Window -ne $console.Descendant.Window) {
    throw "Console inheritance failed: $($result.Stdout)"
}
Write-Host 'Parent and raw descendant share one hidden console.'

$unicode = [string][char]0xD55C + [char]0xAE00
$expected = @('', 'a b', 'c"d', 'C:\a b\', $unicode, '&|<>^%!')
$result = Invoke-HiddenConsoleCommand -FilePath $executable -ArgumentList ($probeArgs + 'Arguments' + $expected) -CaptureOutput
if ($result.ExitCode -ne 7 -or $result.Stderr.Trim() -ne ('stderr: ' + $unicode)) { throw 'Exit code or stderr changed' }
$actual = ConvertFrom-Json -InputObject $result.Stdout
if ($actual.Count -ne $expected.Count) {
    throw "Argument count changed: $($actual.Count), expected $($expected.Count); type=$($actual.GetType().FullName)"
}
for ($index = 0; $index -lt $expected.Count; $index++) {
    if ($actual[$index] -cne $expected[$index]) { throw "Argument $index changed" }
}
Write-Host 'Exit code, Unicode, empty/quoted/spaced arguments and shell characters passed.'

$result = Invoke-HiddenConsoleCommand -FilePath $executable -ArgumentList ($probeArgs + 'Flood') -CaptureOutput
if ($result.ExitCode -ne 0 -or $result.Stdout.Trim().Split("`n").Count -ne 10000 -or $result.Stderr.Trim().Split("`n").Count -ne 10000) {
    throw 'Concurrent output was lost'
}
Write-Host 'Both streams drained 10,000 lines without deadlock.'
$result = Invoke-HiddenConsoleCommand -FilePath $executable -ArgumentList ($probeArgs + 'Runner') -CaptureOutput
if ($result.ExitCode -ne 9 -or $result.Stdout.Trim() -ne 'streamed stdout' -or $result.Stderr.Trim() -ne 'streamed stderr') {
    throw 'run-hidden.ps1 lost output or the exit code'
}
Write-Host 'run-hidden.ps1 preserved output and exit code 9.'
$code = Invoke-HiddenConsoleCommand -FilePath $executable -ArgumentList ($probeArgs + 'Streams')
if ($code -ne 9) { throw 'Streaming exit code changed' }
