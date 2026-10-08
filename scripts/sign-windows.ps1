param(
    [Parameter(Mandatory)][string]$Path,
    [Parameter(Mandatory)][string]$CertificateThumbprint
)
$ErrorActionPreference = 'Stop'
$certificate = Get-Item -LiteralPath "Cert:/CurrentUser/My/$CertificateThumbprint"
if (-not $certificate.HasPrivateKey) { throw 'The signing certificate has no private key' }
$signature = Set-AuthenticodeSignature -LiteralPath $Path -Certificate $certificate `
    -HashAlgorithm SHA256 -TimestampServer 'http://timestamp.digicert.com'
if ($signature.Status -ne 'Valid') { throw "Signing failed: $($signature.StatusMessage)" }
if ((Get-AuthenticodeSignature -LiteralPath $Path).Status -ne 'Valid') { throw 'Signature verification failed' }
