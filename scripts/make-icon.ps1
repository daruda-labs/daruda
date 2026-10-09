# Convert the application PNG to a multi-size Windows ICO without extra tools.
param([string]$Source = "$PSScriptRoot/../assets/icon.png")

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$sourcePath = (Resolve-Path -LiteralPath $Source).Path
$outputPath = [IO.Path]::GetFullPath("$PSScriptRoot/../assets/icon.ico")
$image = [Drawing.Image]::FromFile($sourcePath)
try {
    if ($image.Width -ne $image.Height) { throw 'The application icon must be square' }
    $sizes = @(16, 24, 32, 48, 64, 128, 256)
    $frames = @(
        foreach ($size in $sizes) {
            $bitmap = [Drawing.Bitmap]::new($size, $size, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
            $graphics = [Drawing.Graphics]::FromImage($bitmap)
            $stream = [IO.MemoryStream]::new()
            $writer = [IO.BinaryWriter]::new($stream)
            try {
                $graphics.CompositingMode = [Drawing.Drawing2D.CompositingMode]::SourceCopy
                $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $graphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                $graphics.DrawImage($image, [Drawing.Rectangle]::new(0, 0, $size, $size))
                $maskStride = [int]([Math]::Ceiling($size / 32.0) * 4)
                $mask = [byte[]]::new($maskStride * $size)
                # ICO DIB height includes both the BGRA pixels and the AND mask.
                $writer.Write([uint32]40)
                $writer.Write([int]$size)
                $writer.Write([int]($size * 2))
                $writer.Write([uint16]1)
                $writer.Write([uint16]32)
                $writer.Write([uint32]0)
                $writer.Write([uint32]($size * $size * 4 + $mask.Length))
                $writer.Write([byte[]]::new(16))
                for ($y = $size - 1; $y -ge 0; $y--) {
                    for ($x = 0; $x -lt $size; $x++) {
                        $pixel = $bitmap.GetPixel($x, $y)
                        $writer.Write([byte[]]@($pixel.B, $pixel.G, $pixel.R, $pixel.A))
                        if ($pixel.A -eq 0) {
                            $index = ($size - 1 - $y) * $maskStride + [int][Math]::Floor($x / 8)
                            $mask[$index] = $mask[$index] -bor (0x80 -shr ($x % 8))
                        }
                    }
                }
                $writer.Write($mask)
                ,$stream.ToArray()
            } finally {
                $writer.Dispose()
                $stream.Dispose()
                $graphics.Dispose()
                $bitmap.Dispose()
            }
        }
    )
    $output = [IO.BinaryWriter]::new([IO.File]::Create($outputPath))
    try {
        $output.Write([uint16]0)
        $output.Write([uint16]1)
        $output.Write([uint16]$sizes.Count)
        $offset = 6 + 16 * $sizes.Count
        for ($i = 0; $i -lt $sizes.Count; $i++) {
            $dimension = if ($sizes[$i] -eq 256) { 0 } else { $sizes[$i] }
            $output.Write([byte[]]@($dimension, $dimension, 0, 0))
            $output.Write([uint16]1)
            $output.Write([uint16]32)
            $output.Write([uint32]$frames[$i].Length)
            $output.Write([uint32]$offset)
            $offset += $frames[$i].Length
        }
        foreach ($frame in $frames) { $output.Write([byte[]]$frame) }
    } finally { $output.Dispose() }
    Write-Host "Created: $outputPath ($($sizes -join ', ') px)"
} finally { $image.Dispose() }
