$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/windows/package-version.ps1"
foreach ($version in @('0.2.16', '0.2.17-rc.1', '1.0.0-beta.2+build.7', '1.0.0+build.7')) {
    foreach ($suffix in @('', '-debug')) {
        $actual = Get-DarudaPackageVersion "daruda-$version-windows-x86_64$suffix"
        if ($actual -cne $version) { throw "Version did not round-trip: $version" }
    }
}
foreach ($name in @('daruda-1.2-windows-x86_64', 'daruda-1.2.3-rc..1-windows-x86_64', 'daruda-1.2.3-linux-x86_64')) {
    $rejected = $false
    try { Get-DarudaPackageVersion $name | Out-Null } catch { $rejected = $true }
    if (-not $rejected) { throw "Accepted invalid package: $name" }
}
Write-Host 'Windows package version checks passed (8 valid, 3 invalid).'
