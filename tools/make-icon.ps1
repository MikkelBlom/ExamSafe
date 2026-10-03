<#
.SYNOPSIS
    Generates the ExamSafe app icon: assets/icon.png (256 px) and assets/icon.ico (16-256 px).

.DESCRIPTION
    Rounded square with a blue-to-indigo gradient (the app's accent and exam colours) and a white
    check mark. Drawn in code so it is reproducible and easy to tweak; re-run after changes.
#>
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$assets = Join-Path $PSScriptRoot '..\assets'
New-Item -ItemType Directory -Force -Path $assets | Out-Null

function New-IconBitmap([int] $Size) {
    $bitmap = New-Object System.Drawing.Bitmap $Size, $Size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bitmap)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)

    # Rounded square ("squircle"-like) with a small inset so it doesn't touch the edges.
    $inset = [Math]::Max(1.0, $Size * 0.04)
    $box = New-Object System.Drawing.RectangleF $inset, $inset, ($Size - 2 * $inset), ($Size - 2 * $inset)
    $radius = $box.Width * 0.27
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $radius * 2
    $path.AddArc($box.X, $box.Y, $d, $d, 180, 90)
    $path.AddArc($box.Right - $d, $box.Y, $d, $d, 270, 90)
    $path.AddArc($box.Right - $d, $box.Bottom - $d, $d, $d, 0, 90)
    $path.AddArc($box.X, $box.Bottom - $d, $d, $d, 90, 90)
    $path.CloseFigure()
    $top = [System.Drawing.Color]::FromArgb(255, 0x0a, 0x84, 0xff)
    $bottom = [System.Drawing.Color]::FromArgb(255, 0x5e, 0x5c, 0xe6)
    $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush $box, $top, $bottom, 90.0
    $g.FillPath($brush, $path)

    # White check mark with round caps.
    $pen = New-Object System.Drawing.Pen ([System.Drawing.Color]::White), ([float]($Size * 0.1))
    $pen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $pen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $pen.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round
    $points = [System.Drawing.PointF[]]@(
        (New-Object System.Drawing.PointF ($Size * 0.29), ($Size * 0.52)),
        (New-Object System.Drawing.PointF ($Size * 0.44), ($Size * 0.67)),
        (New-Object System.Drawing.PointF ($Size * 0.72), ($Size * 0.36))
    )
    $g.DrawLines($pen, $points)

    $pen.Dispose(); $brush.Dispose(); $path.Dispose(); $g.Dispose()
    $bitmap
}

function Get-PngBytes([System.Drawing.Bitmap] $Bitmap) {
    $stream = New-Object System.IO.MemoryStream
    $Bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    # The leading comma stops PowerShell from unrolling the byte array into single bytes.
    , [byte[]]$stream.ToArray()
}

# PNG for the Slint window icon.
$large = New-IconBitmap 256
$large.Save((Join-Path $assets 'icon.png'), [System.Drawing.Imaging.ImageFormat]::Png)

# ICO with PNG-compressed entries (supported since Windows Vista).
$sizes = 16, 24, 32, 48, 64, 128, 256
$images = [System.Collections.Generic.List[byte[]]]::new()
foreach ($size in $sizes) { $images.Add((Get-PngBytes (New-IconBitmap $size))) }
$out = New-Object System.IO.MemoryStream
$writer = New-Object System.IO.BinaryWriter $out
$writer.Write([UInt16]0); $writer.Write([UInt16]1); $writer.Write([UInt16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $dimension = if ($sizes[$i] -ge 256) { 0 } else { $sizes[$i] }   # 0 means 256 in ICO headers
    $writer.Write([byte]$dimension); $writer.Write([byte]$dimension)
    $writer.Write([byte]0); $writer.Write([byte]0)                     # palette, reserved
    $writer.Write([UInt16]1); $writer.Write([UInt16]32)               # planes, bits per pixel
    $writer.Write([UInt32]$images[$i].Length); $writer.Write([UInt32]$offset)
    $offset += $images[$i].Length
}
foreach ($bytes in $images) { $writer.Write($bytes) }
$writer.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $assets 'icon.ico'), $out.ToArray())
Write-Host "Wrote assets/icon.png and assets/icon.ico ($($sizes -join ', ') px)" -ForegroundColor Green
