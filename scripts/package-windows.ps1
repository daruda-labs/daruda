param(
    [ValidateSet('debug', 'release')][string]$Profile = 'release',
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
Push-Location $repoRoot
try {
    $metadata = & cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read Cargo workspace metadata' }
    $metadata = $metadata | ConvertFrom-Json
    $version = ($metadata.packages | Where-Object name -EQ 'daruda').version
    if (-not $version) { throw 'Cannot determine the daruda version' }
    if (-not $SkipBuild) {
        & "$PSScriptRoot/build-windows.ps1" -Release:($Profile -eq 'release')
    }

    $binary = Join-Path $metadata.target_directory "$Profile/daruda.exe"
    if (-not (Test-Path -LiteralPath $binary)) { throw "Build output is missing: $binary" }
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($binary))
    try {
        $reader.BaseStream.Position = 0x3c
        $peOffset = $reader.ReadInt32()
        $reader.BaseStream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) {
            throw 'Windows packaging requires an x86_64 PE executable'
        }
    } finally { $reader.Dispose() }

    # App-local CRT deployment lets the ZIP run without a Visual Studio install.
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($LASTEXITCODE -ne 0 -or -not $installation) { throw 'Visual Studio C++ tools are required for packaging' }
    $redistVersion = (Get-Content -LiteralPath (Join-Path $installation 'VC/Auxiliary/Build/Microsoft.VCRedistVersion.default.txt') -Raw).Trim()
    $crt = @(Get-ChildItem -LiteralPath (Join-Path $installation "VC/Redist/MSVC/$redistVersion/x64") -Directory |
        Where-Object Name -Like 'Microsoft.VC*.CRT')
    if ($crt.Count -ne 1) { throw 'Cannot identify the x86_64 MSVC runtime directory' }
    $runtime = @(Get-ChildItem -LiteralPath $crt[0].FullName -Filter '*.dll' -File)
    if (-not ($runtime.Name -contains 'vcruntime140.dll')) { throw 'MSVC runtime DLLs are missing' }

    $outputRoot = [IO.Path]::GetFullPath((Join-Path $metadata.target_directory 'packages'))
    New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
    $suffix = if ($Profile -eq 'debug') { '-debug' } else { '' }
    $name = "daruda-$version-windows-x86_64$suffix"
    $stage = Join-Path $outputRoot ('.staging-' + [Guid]::NewGuid().ToString('N'))
    $bundle = Join-Path $stage $name
    New-Item -ItemType Directory -Path $bundle | Out-Null
    try {
        Copy-Item -LiteralPath $binary -Destination (Join-Path $bundle 'daruda.exe')
        Copy-Item -LiteralPath (Join-Path $repoRoot 'LICENSE') -Destination $bundle
        Copy-Item -LiteralPath (Join-Path $repoRoot 'licenses') -Destination $bundle -Recurse
        foreach ($dll in $runtime) { Copy-Item -LiteralPath $dll.FullName -Destination $bundle }
        @"
daruda $version ($Profile, Windows x86_64)

Extract the entire ZIP, then run daruda.exe. Keep the runtime DLLs beside it.
This is a portable, unsigned build. Windows GUI support is experimental.
Install Git for Windows to use repositories, shell flows, and ACP agents.
Source and build instructions: https://github.com/daruda-labs/daruda
"@ | Set-Content -LiteralPath (Join-Path $bundle 'README.txt') -Encoding utf8
        $archive = Join-Path $outputRoot "$name.zip"
        Compress-Archive -LiteralPath $bundle -DestinationPath $archive -Force
        Write-Host "Packaged: $archive"
        if ($env:GITHUB_OUTPUT) {
            "archive=$archive" | Add-Content -LiteralPath $env:GITHUB_OUTPUT -Encoding utf8
        }
    } finally {
        $resolved = [IO.Path]::GetFullPath($stage)
        if (-not $resolved.StartsWith($outputRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Staging directory escaped the package output root'
        }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
} finally { Pop-Location }
