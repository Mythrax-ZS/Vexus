<#
.SYNOPSIS
  Renders the Vexus app icon (PNG set + multi-size ICO) with System.Drawing.

.DESCRIPTION
  No external tools required. Output goes to src-tauri/icons by default. The same mark is
  used as an inline SVG in the app (ui/assets/logo.svg); keep them in sync if you change it.
#>
param([string]$OutDir = (Join-Path $PSScriptRoot '..\src-tauri\icons'))

Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = 'Stop'

function P([double]$x, [double]$y) { [System.Drawing.PointF]::new([single]$x, [single]$y) }
function C([int]$a, [int]$r, [int]$g, [int]$b) { [System.Drawing.Color]::FromArgb($a, $r, $g, $b) }

function New-Logo([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)
    $s = $size / 512.0

    # Rounded tile
    $inset = 14 * $s; $w = $size - 2 * $inset; $r = 112 * $s; $d = 2 * $r
    $tile = New-Object System.Drawing.Drawing2D.GraphicsPath
    $tile.AddArc([single]$inset, [single]$inset, [single]$d, [single]$d, 180, 90)
    $tile.AddArc([single]($inset + $w - $d), [single]$inset, [single]$d, [single]$d, 270, 90)
    $tile.AddArc([single]($inset + $w - $d), [single]($inset + $w - $d), [single]$d, [single]$d, 0, 90)
    $tile.AddArc([single]$inset, [single]($inset + $w - $d), [single]$d, [single]$d, 90, 90)
    $tile.CloseFigure()
    $bg = New-Object System.Drawing.Drawing2D.LinearGradientBrush (P 0 0), (P $size $size), (C 255 36 43 60), (C 255 12 14 19)
    $g.FillPath($bg, $tile)

    # Soft glow behind the mark
    $glow = New-Object System.Drawing.Drawing2D.GraphicsPath
    $glow.AddEllipse([single](96 * $s), [single](96 * $s), [single](320 * $s), [single](320 * $s))
    $gb = New-Object System.Drawing.Drawing2D.PathGradientBrush $glow
    $gb.CenterColor = C 70 94 200 255
    $gb.SurroundColors = @((C 0 94 200 255))
    $g.SetClip($tile)
    $g.FillPath($gb, $glow)
    $g.ResetClip()

    # Hairline highlight on the tile edge
    $edge = New-Object System.Drawing.Pen (C 46 255 255 255), ([single][Math]::Max(1, 3 * $s))
    $g.DrawPath($edge, $tile)

    # The V: two heavy rounded strokes, cyan to violet
    $vb = New-Object System.Drawing.Drawing2D.LinearGradientBrush (P (120 * $s) (120 * $s)), (P (392 * $s) (392 * $s)), (C 255 98 214 255), (C 255 150 120 255)
    $vp = New-Object System.Drawing.Pen $vb, ([single](62 * $s))
    $vp.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $vp.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $vp.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round
    $pts = [System.Drawing.PointF[]]@((P (148 * $s) (160 * $s)), (P (256 * $s) (360 * $s)), (P (364 * $s) (160 * $s)))
    $g.DrawLines($vp, $pts)

    # A small "clean" sparkle
    $sp = New-Object System.Drawing.SolidBrush (C 235 255 255 255)
    $g.FillEllipse($sp, [single](392 * $s), [single](86 * $s), [single](26 * $s), [single](26 * $s))

    $g.Dispose(); $bg.Dispose(); $gb.Dispose(); $edge.Dispose(); $vp.Dispose(); $vb.Dispose(); $sp.Dispose()
    return $bmp
}

function Get-PngBytes($bmp) {
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bytes = $ms.ToArray(); $ms.Dispose(); return $bytes
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path

# PNG set used by Tauri's bundler and the window/tray icon
foreach ($entry in @(@('32x32.png', 32), @('128x128.png', 128), @('128x128@2x.png', 256), @('icon.png', 512))) {
    $bmp = New-Logo $entry[1]
    [System.IO.File]::WriteAllBytes((Join-Path $OutDir $entry[0]), (Get-PngBytes $bmp))
    $bmp.Dispose()
}

# Multi-size ICO with PNG-compressed images (Vista+)
$sizes = 16, 24, 32, 48, 64, 128, 256
$images = New-Object 'System.Collections.Generic.List[byte[]]'
foreach ($sz in $sizes) {
    $b = New-Logo $sz
    $images.Add([byte[]](Get-PngBytes $b))
    $b.Dispose()
}
$ms = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter $ms
$bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $sz = $sizes[$i]; $len = $images[$i].Length
    $bw.Write([byte]($(if ($sz -ge 256) { 0 } else { $sz })))
    $bw.Write([byte]($(if ($sz -ge 256) { 0 } else { $sz })))
    $bw.Write([byte]0); $bw.Write([byte]0)
    $bw.Write([uint16]1); $bw.Write([uint16]32)
    $bw.Write([uint32]$len); $bw.Write([uint32]$offset)
    $offset += $len
}
foreach ($img in $images) { $bw.Write($img) }
$bw.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $OutDir 'icon.ico'), $ms.ToArray())
$bw.Dispose(); $ms.Dispose()

Get-ChildItem $OutDir | Select-Object Name, Length | Format-Table -AutoSize
