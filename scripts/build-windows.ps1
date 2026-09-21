param([switch]$Release)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent

Push-Location $repoRoot
try {
    $buildArgs = @('build', '--locked', '-p', 'daruda')
    if ($Release) { $buildArgs += '--release' }
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) {
        throw "Cargo build failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}
