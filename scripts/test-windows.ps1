param(
    [switch]$CheckOnly,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$CargoArgs
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
$previousPath = $env:PATH

Push-Location $repoRoot
try {
    $candidates = @(Get-Command git.exe -All -ErrorAction SilentlyContinue)
    $supported = @($candidates | Where-Object {
        $versionText = & $_.Source --version
        $LASTEXITCODE -eq 0 -and $versionText -match 'git version (\d+\.\d+\.\d+)' -and
            [version]$Matches[1] -ge [version]'2.28.0'
    })
    if ($supported.Count -eq 0) {
        throw 'Tests require Git for Windows 2.28 or newer on PATH.'
    }
    $git = $supported[0].Source
    $shellTools = @($candidates | ForEach-Object {
        $root = Split-Path (Split-Path $_.Source -Parent) -Parent
        Join-Path $root 'usr/bin'
    } | Where-Object { Test-Path -LiteralPath (Join-Path $_ 'sh.exe') })
    if ($shellTools.Count -eq 0) {
        throw 'Tests require the POSIX shell tools included with Git for Windows.'
    }
    $env:PATH = (Split-Path $git -Parent) + ';' + $shellTools[0] + ';' + $previousPath
    Write-Host "Test Git: $git"

    # Probe the real capability; an administrator account alone is not proof.
    $probe = Join-Path ([IO.Path]::GetTempPath()) ('daruda-symlink-probe-' + [Guid]::NewGuid().ToString('N'))
    $file = Join-Path $probe 'target'
    $link = Join-Path $probe 'link'
    New-Item -ItemType Directory -Path $probe | Out-Null
    try {
        [IO.File]::WriteAllText($file, 'probe')
        try {
            New-Item -ItemType SymbolicLink -Path $link -Target $file -ErrorAction Stop | Out-Null
        } catch {
            throw 'Tests require symbolic-link permission. Enable Windows Developer Mode or run from a shell with the symlink privilege; no tests were skipped.'
        }
    } finally {
        if (Test-Path -LiteralPath $link) { Remove-Item -LiteralPath $link -Force }
        Remove-Item -LiteralPath $file -Force
        Remove-Item -LiteralPath $probe
    }
    if ($CheckOnly) { return }

    $testArgs = @('test', '--locked', '--no-fail-fast')
    if ($CargoArgs.Count -gt 0) {
        $testArgs += $CargoArgs
    } else {
        foreach ($package in @(
            'ghostty_vt', 'ghostty_vt_sys', 'daruda_terminal', 'daruda',
            'daruda_config', 'daruda_store', 'daruda_agent', 'daruda_update',
            'daruda_acp', 'daruda_core', 'daruda_flow', 'daruda_project',
            'daruda_ui', 'daruda_control_types', 'daruda_content', 'daruda_flow_edit',
            'ferrum_flow', 'gpui_component', 'strings_gen', 'vendor_gpui', 'test_process'
        )) { $testArgs += @('-p', $package) }
    }
    & cargo @testArgs
    if ($LASTEXITCODE -ne 0) { throw "Cargo tests failed with exit code $LASTEXITCODE" }
} finally {
    $env:PATH = $previousPath
    Pop-Location
}
