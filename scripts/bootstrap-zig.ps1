$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
$version = '0.14.1'
$destination = Join-Path $repoRoot ".context/zig/$version"
$executable = Join-Path $destination 'zig.exe'

if (Test-Path -LiteralPath $executable) {
    $installed = & $executable version
    if ($LASTEXITCODE -ne 0 -or $installed -ne $version) {
        throw "Expected Zig $version at $executable"
    }
    Write-Host "Zig $version already installed."
    return
}

# Archive checksums from https://ziglang.org/download/index.json.
$archive = switch ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
    'X64' { @{ Arch = 'x86_64'; Sha256 = '554f5378228923ffd558eac35e21af020c73789d87afeabf4bfd16f2e6feed2c' } }
    'Arm64' { @{ Arch = 'aarch64'; Sha256 = 'b5aac0ccc40dd91e8311b1f257717d8e3903b5fefb8f659de6d65a840ad1d0e7' } }
    default { throw 'This installer supports Windows x86_64 and ARM64. Install Zig manually on other architectures.' }
}

$temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$staging = Join-Path $temporaryRoot ("daruda-zig-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $staging | Out-Null
try {
    $archiveName = "zig-$($archive.Arch)-windows-$version"
    $zip = Join-Path $staging 'zig.zip'
    Invoke-WebRequest -Uri "https://ziglang.org/download/$version/$archiveName.zip" -OutFile $zip
    if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant() -ne $archive.Sha256) {
        throw 'Zig archive SHA256 mismatch'
    }
    Expand-Archive -LiteralPath $zip -DestinationPath $staging
    $extracted = [IO.Path]::GetFullPath((Join-Path $staging $archiveName))
    if (-not $extracted.StartsWith($staging + [IO.Path]::DirectorySeparatorChar) -or
        -not (Test-Path -LiteralPath (Join-Path $extracted 'zig.exe'))) {
        throw 'Unexpected Zig archive layout'
    }
    New-Item -ItemType Directory -Force -Path (Split-Path $destination -Parent) | Out-Null
    if (Test-Path -LiteralPath $destination) { throw "Installation directory already exists: $destination" }
    Move-Item -LiteralPath $extracted -Destination $destination
    $installed = & $executable version
    if ($LASTEXITCODE -ne 0 -or $installed -ne $version) { throw 'Installed Zig version is invalid' }
    Write-Host "Installed Zig $version at $destination"
} finally {
    $resolved = [IO.Path]::GetFullPath($staging)
    if (-not $resolved.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Temporary directory escaped the temporary root'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
