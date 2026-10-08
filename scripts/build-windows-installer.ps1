param(
    [Parameter(Mandatory)][string]$Archive,
    [string]$Makensis = 'makensis.exe',
    [string]$CertificateThumbprint
)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/windows/package-version.ps1"
$archivePath = (Resolve-Path -LiteralPath $Archive).Path
$name = [IO.Path]::GetFileNameWithoutExtension($archivePath)
$version = Get-DarudaPackageVersion -Name $name
$outputRoot = Split-Path $archivePath -Parent
$stage = Join-Path $outputRoot ('.installer-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage | Out-Null
try {
    Expand-Archive -LiteralPath $archivePath -DestinationPath $stage
    $bundles = @(Get-ChildItem -LiteralPath $stage -Directory)
    if ($bundles.Count -ne 1) { throw 'Expected one application bundle in the archive' }
    $bundle = $bundles[0].FullName
    $binary = Join-Path $bundle 'daruda.exe'
    if (-not (Test-Path -LiteralPath $binary)) { throw 'The archive contains no daruda executable' }
    if ($CertificateThumbprint) {
        & "$PSScriptRoot/sign-windows.ps1" -Path $binary -CertificateThumbprint $CertificateThumbprint
    }
    $output = Join-Path $outputRoot "$name-setup.exe"
    $manifest = Join-Path $stage 'uninstall-manifest.nsh'
    $lines = [Collections.Generic.List[string]]::new()
    foreach ($file in Get-ChildItem -LiteralPath $bundle -Recurse -File) {
        $relative = [IO.Path]::GetRelativePath($bundle, $file.FullName)
        if ($relative.Contains('$') -or $relative.Contains('"')) { throw 'Unsupported package filename' }
        $lines.Add('Delete "$INSTDIR\' + $relative + '"')
    }
    foreach ($directory in Get-ChildItem -LiteralPath $bundle -Recurse -Directory | Sort-Object { $_.FullName.Length } -Descending) {
        $relative = [IO.Path]::GetRelativePath($bundle, $directory.FullName)
        if ($relative.Contains('$') -or $relative.Contains('"')) { throw 'Unsupported package directory' }
        $lines.Add('RMDir "$INSTDIR\' + $relative + '"')
    }
    $lines | Set-Content -LiteralPath $manifest -Encoding utf8
    $definitions = @("/DBUNDLE=$bundle", "/DOUTPUT=$output", "/DVERSION=$version", "/DUNINSTALL_MANIFEST=$manifest")
    if ($CertificateThumbprint) {
        if ($CertificateThumbprint -notmatch '^[A-Fa-f0-9]{40}$') { throw 'Invalid certificate thumbprint' }
        $signer = Join-Path $PSScriptRoot 'sign-windows.ps1'
        $pwsh = (Get-Command pwsh -ErrorAction Stop).Source
        $definitions += "/DUNINSTALL_SIGN_COMMAND=`"$pwsh`" -NoProfile -File `"$signer`" -Path `"%1`" -CertificateThumbprint $CertificateThumbprint"
    }
    & $Makensis @definitions "$PSScriptRoot/windows/installer.nsi"
    if ($LASTEXITCODE -ne 0) { throw 'NSIS installer build failed' }
    if ($CertificateThumbprint) {
        & "$PSScriptRoot/sign-windows.ps1" -Path $output -CertificateThumbprint $CertificateThumbprint
    }
    Write-Host "Installer: $output"
    if ($env:GITHUB_OUTPUT) { "installer=$output" | Add-Content -LiteralPath $env:GITHUB_OUTPUT -Encoding utf8 }
} finally {
    $resolvedStage = [IO.Path]::GetFullPath($stage)
    if (-not $resolvedStage.StartsWith($outputRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Installer staging escaped the package output root'
    }
    Remove-Item -LiteralPath $resolvedStage -Recurse -Force
}
