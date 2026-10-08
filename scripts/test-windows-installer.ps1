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
$install = Join-Path $root 'first application'
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
    New-Item -ItemType Directory -Path $data | Out-Null
    Set-Content -LiteralPath (Join-Path $data 'recent-workspaces.json') -Value '[]'
    $smoke = Start-Process -FilePath $binary -ArgumentList '--smoke' -WindowStyle Hidden -PassThru
    if (-not $smoke.WaitForExit(60000)) { throw 'Installed application smoke test timed out' }
    if ($smoke.ExitCode -ne 0) { throw "Installed application failed its smoke test: $($smoke.ExitCode)" }
    foreach ($expected in @('logs', 'state/workspace/.workspace-layout.json', 'state/workspace/recent-workspaces.json')) {
        if (-not (Test-Path -LiteralPath (Join-Path $data $expected))) { throw "Installed application missed its storage location: $expected" }
    }
    $sentinel = Join-Path $install 'user-created.txt'
    Set-Content -LiteralPath $sentinel -Value 'preserve user-created files'
    $dataSentinel = Join-Path $data 'preserve.txt'
    Set-Content -LiteralPath $dataSentinel -Value 'preserve application data'
    # Locked files must be detected before any shipped file or registration is removed.
    foreach ($lockedName in @('daruda.exe', 'vcruntime140.dll')) {
        $locked = [IO.File]::Open((Join-Path $install $lockedName), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        try {
            $blocked = Start-Process -FilePath (Join-Path $install 'uninstall.exe') -ArgumentList @('/S', "_?=$install") -WindowStyle Hidden -PassThru
            if (-not $blocked.WaitForExit(60000)) { throw 'Locked-file uninstall timed out' }
            if ($blocked.ExitCode -ne 2) { throw "Locked $lockedName was not refused: $($blocked.ExitCode)" }
            if (-not (Test-Path -LiteralPath $binary) -or -not (Test-Path -LiteralPath $uninstallRegistry)) { throw 'Refused uninstall partially removed the installation' }
            $blocked = Start-Process -FilePath $installerPath -ArgumentList @('/S', "/D=$install") -WindowStyle Hidden -PassThru
            if (-not $blocked.WaitForExit(60000)) { throw 'Locked-file reinstall timed out' }
            if ($blocked.ExitCode -ne 2) { throw "Reinstall overwrote locked $lockedName" }
        } finally { $locked.Dispose() }
    }
    # A staged update must wait for the old process to release the executable.
    $ready = Join-Path $root 'update-ready.txt'
    $helper = Join-Path $root 'hold-installation.ps1'
    @'
param([string]$Binary, [string]$Ready)
$locked = [IO.File]::Open($Binary, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
try { Set-Content -LiteralPath $Ready -Value 'ready'; Start-Sleep -Seconds 3 } finally { $locked.Dispose() }
'@ | Set-Content -LiteralPath $helper -Encoding utf8
    $holder = Start-Process -FilePath (Join-Path $PSHOME 'pwsh.exe') -ArgumentList @('-NoProfile', '-File', "`"$helper`"", '-Binary', "`"$binary`"", '-Ready', "`"$ready`"") -WindowStyle Hidden -PassThru
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    while (-not (Test-Path -LiteralPath $ready)) {
        if ($holder.HasExited -or [DateTime]::UtcNow -gt $deadline) { throw 'Update holder did not become ready' }
        Start-Sleep -Milliseconds 50
    }
    $update = Start-Process -FilePath $installerPath -ArgumentList @('/S', "/WAITPID=$($holder.Id)", "/D=$install") -WindowStyle Hidden -PassThru
    if (-not $update.WaitForExit(60000) -or $update.ExitCode -ne 0) { throw 'Installer update failed to wait and reinstall' }
    if (-not $holder.HasExited) { throw 'Installer completed before the old process exited' }
    if (-not (Test-Path -LiteralPath $sentinel)) { throw 'Update removed a user-created file' }
    # Changing location leaves an older uninstaller: it must not own the new registration.
    $secondInstall = Join-Path $root 'second application'
    $relocate = Start-Process -FilePath $installerPath -ArgumentList @('/S', "/D=$secondInstall") -WindowStyle Hidden -PassThru
    if (-not $relocate.WaitForExit(60000) -or $relocate.ExitCode -ne 0) { throw 'Relocated installation failed' }
    $oldRemove = Start-Process -FilePath (Join-Path $install 'uninstall.exe') -ArgumentList @('/S', "_?=$install") -WindowStyle Hidden -PassThru
    if (-not $oldRemove.WaitForExit(60000) -or $oldRemove.ExitCode -ne 0) { throw 'Old installation could not be removed' }
    if ((Get-ItemProperty -LiteralPath $registry).InstallDir -ne $secondInstall -or -not (Test-Path -LiteralPath $desktopShortcut)) { throw 'Old uninstaller removed new integration' }
    $shell = New-Object -ComObject WScript.Shell
    if ($shell.CreateShortcut($desktopShortcut).TargetPath -ne (Join-Path $secondInstall 'daruda.exe')) { throw 'New shortcut target differs' }
    $install = $secondInstall
    $binary = Join-Path $install 'daruda.exe'
    $secondSentinel = Join-Path $install 'user-created.txt'
    $ownedFiles = @(Get-ChildItem -LiteralPath $install -Recurse -File | Where-Object Name -NE 'uninstall.exe' | ForEach-Object FullName)
    Set-Content -LiteralPath $secondSentinel -Value 'preserve relocated user files'
    $uninstaller = Join-Path $install 'uninstall.exe'
    # NSIS's _?= mode waits in this process instead of spawning a temporary copy.
    $remove = Start-Process -FilePath $uninstaller -ArgumentList @('/S', "_?=$install") -WindowStyle Hidden -PassThru
    if (-not $remove.WaitForExit(60000)) { throw 'Uninstaller timed out' }
    if ($remove.ExitCode -ne 0) { throw "Uninstallation failed: $($remove.ExitCode)" }
    if (Test-Path -LiteralPath $binary) { throw 'Uninstaller left the shipped executable' }
    foreach ($ownedFile in $ownedFiles) {
        if (Test-Path -LiteralPath $ownedFile) { throw "Uninstaller left a shipped file: $ownedFile" }
    }
    foreach ($removed in @($registry, $uninstallRegistry, $desktopShortcut, $menuDirectory)) {
        if (Test-Path -LiteralPath $removed) { throw "Uninstaller left desktop integration: $removed" }
    }
    $shell = New-Object -ComObject WScript.Shell
    foreach ($link in Get-ChildItem -LiteralPath ([Environment]::GetFolderPath('Programs')) -Filter 'daruda-*.lnk') {
        if ($link.FullName -notin $shortcutsBefore -and $shell.CreateShortcut($link.FullName).TargetPath -eq $binary) {
            throw "Uninstaller left a generated notification shortcut: $($link.FullName)"
        }
    }
    if (-not (Test-Path -LiteralPath $sentinel) -or -not (Test-Path -LiteralPath $secondSentinel) -or -not (Test-Path -LiteralPath $dataSentinel) -or -not (Test-Path -LiteralPath (Join-Path $data 'state/workspace/recent-workspaces.json'))) {
        throw 'Uninstaller removed user-created files or application data'
    }
    Write-Host 'PASS: install, native storage migration, smoke, locked-file refusal, waited update, relocation ownership, uninstall, and data preservation'
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
