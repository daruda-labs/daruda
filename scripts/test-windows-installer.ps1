param([Parameter(Mandatory)][string]$Installer)
$ErrorActionPreference = 'Stop'
$installerPath = (Resolve-Path -LiteralPath $Installer).Path
$desktopShortcut = Join-Path ([Environment]::GetFolderPath('Desktop')) 'daruda.lnk'
$menuDirectory = Join-Path ([Environment]::GetFolderPath('Programs')) 'daruda'
$registry = 'HKCU:\Software\daruda'
$uninstallRegistry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\daruda'
# The installer owns per-user integration. Never overwrite an existing install.
foreach ($existing in @($registry, $uninstallRegistry, $desktopShortcut, $menuDirectory)) {
    if (Test-Path -LiteralPath $existing) { throw "Existing desktop integration prevents an isolated installer test: $existing" }
}
$root = Join-Path ([IO.Path]::GetTempPath()) ('daruda-installer-test-' + [Guid]::NewGuid().ToString('N'))
$install = Join-Path $root 'application'
$data = Join-Path $root 'state'
New-Item -ItemType Directory -Path $root | Out-Null
$oldData = $env:DARUDA_DATA_DIR
$shortcutsBefore = @(Get-ChildItem -LiteralPath ([Environment]::GetFolderPath('Programs')) -Filter 'daruda-*.lnk' | ForEach-Object FullName)
try {
    $setup = Start-Process -FilePath $installerPath -ArgumentList @('/S', "/D=$install") -WindowStyle Hidden -PassThru
    if (-not $setup.WaitForExit(120000)) { throw 'Installer timed out' }
    if ($setup.ExitCode -ne 0) { throw "Installation failed: $($setup.ExitCode)" }
    $binary = Join-Path $install 'daruda.exe'
    if (-not (Test-Path -LiteralPath $binary)) { throw 'Installed executable is missing' }
    if ((Get-ItemProperty -LiteralPath $registry).InstallDir -ne $install) { throw 'Install directory registration differs' }
    if (-not (Test-Path -LiteralPath $desktopShortcut)) { throw 'Desktop shortcut is missing' }
    $env:DARUDA_DATA_DIR = $data
    $smoke = Start-Process -FilePath $binary -ArgumentList '--smoke' -WindowStyle Hidden -PassThru
    if (-not $smoke.WaitForExit(60000)) { throw 'Installed application smoke test timed out' }
    if ($smoke.ExitCode -ne 0) { throw "Installed application failed its smoke test: $($smoke.ExitCode)" }
    $sentinel = Join-Path $install 'user-created.txt'
    Set-Content -LiteralPath $sentinel -Value 'preserve user-created files'
    $dataSentinel = Join-Path $data 'preserve.txt'
    Set-Content -LiteralPath $dataSentinel -Value 'preserve application data'
    $uninstaller = Join-Path $install 'uninstall.exe'
    # NSIS's _?= mode waits in this process instead of spawning a temporary copy.
    $remove = Start-Process -FilePath $uninstaller -ArgumentList @('/S', "_?=$install") -WindowStyle Hidden -PassThru
    if (-not $remove.WaitForExit(60000)) { throw 'Uninstaller timed out' }
    if ($remove.ExitCode -ne 0) { throw "Uninstallation failed: $($remove.ExitCode)" }
    if (Test-Path -LiteralPath $binary) { throw 'Uninstaller left the shipped executable' }
    foreach ($removed in @($registry, $uninstallRegistry, $desktopShortcut, $menuDirectory)) {
        if (Test-Path -LiteralPath $removed) { throw "Uninstaller left desktop integration: $removed" }
    }
    $shell = New-Object -ComObject WScript.Shell
    foreach ($link in Get-ChildItem -LiteralPath ([Environment]::GetFolderPath('Programs')) -Filter 'daruda-*.lnk') {
        if ($link.FullName -notin $shortcutsBefore -and $shell.CreateShortcut($link.FullName).TargetPath -eq $binary) {
            throw "Uninstaller left a generated notification shortcut: $($link.FullName)"
        }
    }
    if (-not (Test-Path -LiteralPath $sentinel) -or -not (Test-Path -LiteralPath $dataSentinel)) {
        throw 'Uninstaller removed user-created files or application data'
    }
    Write-Host 'PASS: per-user install, installed window smoke test, uninstall, and user-data preservation'
} finally {
    $env:DARUDA_DATA_DIR = $oldData
    # Keep failed installations for diagnosis rather than deleting a live binary.
    if (-not (Test-Path -LiteralPath (Join-Path $install 'daruda.exe'))) {
        $resolvedRoot = [IO.Path]::GetFullPath($root)
        $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
        if (-not $resolvedRoot.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or
            [IO.Path]::GetFileName($resolvedRoot) -notlike 'daruda-installer-test-*') { throw 'Unsafe test cleanup path' }
        Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
    }
}
