<#
.SYNOPSIS
  Builds Vexus (release) and packs a portable zip: vexus.exe plus a `vexus.portable` marker.

.DESCRIPTION
  In portable mode Vexus keeps *everything* - settings, history, logs, the Recovery store and the
  WebView2 profile - in a `data` folder next to the executable, and writes nowhere else.
  Requires the WebView2 runtime (included with Windows 11; the MSI/NSIS installers bootstrap it).

.EXAMPLE
  .\scripts\package-portable.ps1            # build + zip into .\dist
  .\scripts\package-portable.ps1 -SkipBuild # reuse target\release\vexus.exe
#>
param(
    [string]$OutDir = (Join-Path $PSScriptRoot '..\dist'),
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    if (-not $SkipBuild) {
        # `custom-protocol` embeds the UI (this is what `cargo tauri build` enables too).
        cargo build --release -p vexus-app --features custom-protocol
        if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    }
    $exe = Join-Path $root 'target\release\vexus.exe'
    if (-not (Test-Path $exe)) { throw "Not found: $exe (run without -SkipBuild)" }

    $version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
    $OutDir = (Resolve-Path $OutDir).Path
    $stage = Join-Path $OutDir "Vexus-$version-portable"
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Path $stage | Out-Null

    Copy-Item $exe (Join-Path $stage 'vexus.exe')
    New-Item -ItemType File -Path (Join-Path $stage 'vexus.portable') | Out-Null
    @"
Vexus $version - portable
=========================

Run vexus.exe. Everything Vexus stores (settings, history, logs, the Recovery store and the
WebView2 profile) lives in the "data" folder next to it, so you can move or delete the whole
folder without leaving anything behind.

Needs the Microsoft Edge WebView2 runtime (built into Windows 11).
Delete "vexus.portable" to switch back to the normal per-user data folder.
"@ | Set-Content -Path (Join-Path $stage 'README.txt') -Encoding UTF8

    $zip = Join-Path $OutDir "Vexus-$version-portable-x64.zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
    Get-Item $zip | Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 2) } }
}
finally { Pop-Location }
