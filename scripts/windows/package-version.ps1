function Get-DarudaPackageVersion {
    param([Parameter(Mandatory)][string]$Name)
    if ($Name -notmatch '^daruda-(?<version>.+)-windows-x86_64(?:-debug)?$') {
        throw 'Unsupported package name'
    }
    $version = $Matches.version
    if ($version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:[-+].+)?$') {
        throw 'Unsupported package version'
    }
    $parsed = $null
    if (-not [System.Management.Automation.SemanticVersion]::TryParse($version, [ref]$parsed)) {
        throw 'Unsupported package version'
    }
    return $version
}
