<#
.SYNOPSIS
  Drive and screenshot the real Vexus window (WebView2) from PowerShell via the Chrome DevTools
  Protocol. No Node, no browser driver - just a WebSocket.

.DESCRIPTION
  Used for UI development and smoke tests:

    Import-Module .\scripts\Vexus.Cdp.psm1
    $app = Start-VexusApp -Sandbox .\sandbox\ui
    $cdp = Connect-Cdp -Port $app.Port
    Invoke-CdpEval $cdp "document.title"
    Save-CdpScreenshot $cdp .\shot.png
    Stop-VexusApp $app

  The debugging port is opened only for the process this script starts (through the
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS environment variable); a normal Vexus launch never
  exposes one.
#>

Set-StrictMode -Version Latest

function Start-VexusApp {
    [CmdletBinding()]
    param(
        [string]$Exe = (Join-Path $PSScriptRoot '..\target\debug\vexus.exe'),
        [string]$Sandbox,
        [int]$Port = 9333,
        [string[]]$ExtraArgs = @()
    )
    $Exe = (Resolve-Path $Exe).Path
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port --remote-allow-origins=*"
    $argList = @()
    if ($Sandbox) {
        $full = $PSCmdlet.GetUnresolvedProviderPathFromPSPath($Sandbox)
        $argList += '--sandbox'
        $argList += ('"{0}"' -f $full)
    }
    $argList += $ExtraArgs
    $proc = Start-Process -FilePath $Exe -ArgumentList $argList -PassThru
    Remove-Item Env:\WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
    # Wait for the DevTools endpoint to come up.
    $deadline = (Get-Date).AddSeconds(40)
    while ((Get-Date) -lt $deadline) {
        try {
            $targets = Invoke-RestMethod "http://127.0.0.1:$Port/json/list" -TimeoutSec 2
            if ($targets | Where-Object { $_.type -eq 'page' }) { break }
        } catch { Start-Sleep -Milliseconds 300 }
    }
    [pscustomobject]@{ Process = $proc; Port = $Port }
}

function Stop-VexusApp {
    param([Parameter(Mandatory)]$App)
    if ($App.Process -and -not $App.Process.HasExited) {
        $App.Process.CloseMainWindow() | Out-Null
        if (-not $App.Process.WaitForExit(2500)) { Stop-Process -Id $App.Process.Id -Force }
    }
}

function Connect-Cdp {
    [CmdletBinding()]
    param([int]$Port = 9333, [string]$UrlLike = '*')
    $targets = Invoke-RestMethod "http://127.0.0.1:$Port/json/list" -TimeoutSec 5
    $page = $targets | Where-Object { $_.type -eq 'page' -and $_.url -like $UrlLike } | Select-Object -First 1
    if (-not $page) { throw "No page target found on port $Port" }
    $ws = [System.Net.WebSockets.ClientWebSocket]::new()
    $ws.ConnectAsync([Uri]$page.webSocketDebuggerUrl, [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
    $cdp = [pscustomobject]@{ Socket = $ws; NextId = 0; Events = [System.Collections.Generic.List[object]]::new(); Url = $page.url }
    Invoke-Cdp $cdp 'Runtime.enable' | Out-Null
    Invoke-Cdp $cdp 'Log.enable' | Out-Null
    Invoke-Cdp $cdp 'Page.enable' | Out-Null
    $cdp
}

function Receive-CdpMessage {
    param($Cdp, [int]$TimeoutMs = 20000)
    $buffer = New-Object byte[] 1048576
    $ms = New-Object System.IO.MemoryStream
    $cts = [Threading.CancellationTokenSource]::new($TimeoutMs)
    try {
        do {
            $seg = [ArraySegment[byte]]::new($buffer)
            $r = $Cdp.Socket.ReceiveAsync($seg, $cts.Token).GetAwaiter().GetResult()
            $ms.Write($buffer, 0, $r.Count)
        } until ($r.EndOfMessage)
    } finally { $cts.Dispose() }
    [Text.Encoding]::UTF8.GetString($ms.ToArray()) | ConvertFrom-Json -Depth 50
}

function Invoke-Cdp {
    [CmdletBinding()]
    param([Parameter(Mandatory)]$Cdp, [Parameter(Mandatory)][string]$Method, $Params = @{}, [int]$TimeoutMs = 20000)
    $id = ++$Cdp.NextId
    $json = @{ id = $id; method = $Method; params = $Params } | ConvertTo-Json -Depth 20 -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($json)
    $Cdp.Socket.SendAsync([ArraySegment[byte]]::new($bytes), 'Text', $true, [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
    while ($true) {
        $msg = Receive-CdpMessage $Cdp $TimeoutMs
        if ($msg.PSObject.Properties['id'] -and $msg.id -eq $id) {
            if ($msg.PSObject.Properties['error']) { throw "CDP $Method failed: $($msg.error.message)" }
            return $msg.result
        }
        if ($msg.PSObject.Properties['method']) { $Cdp.Events.Add($msg) }
    }
}

function Invoke-CdpEval {
    [CmdletBinding()]
    param([Parameter(Mandatory)]$Cdp, [Parameter(Mandatory)][string]$Expression, [int]$TimeoutMs = 30000)
    $r = Invoke-Cdp $Cdp 'Runtime.evaluate' @{ expression = $Expression; awaitPromise = $true; returnByValue = $true; userGesture = $true } $TimeoutMs
    if ($r.PSObject.Properties['exceptionDetails']) {
        $d = $r.exceptionDetails
        $text = if ($d.PSObject.Properties['exception'] -and $d.exception.PSObject.Properties['description']) { $d.exception.description } else { $d.text }
        throw "JS error: $text"
    }
    if ($r.result.PSObject.Properties['value']) { $r.result.value } else { $null }
}

function Save-CdpScreenshot {
    [CmdletBinding()]
    param([Parameter(Mandatory)]$Cdp, [Parameter(Mandatory)][string]$Path, [int]$Width = 0, [int]$Height = 0, [double]$Scale = 1)
    if ($Width -gt 0) {
        Invoke-Cdp $Cdp 'Emulation.setDeviceMetricsOverride' @{ width = $Width; height = $Height; deviceScaleFactor = $Scale; mobile = $false } | Out-Null
        Start-Sleep -Milliseconds 250
    }
    $r = Invoke-Cdp $Cdp 'Page.captureScreenshot' @{ format = 'png'; fromSurface = $true } 40000
    if ($Width -gt 0) { Invoke-Cdp $Cdp 'Emulation.clearDeviceMetricsOverride' | Out-Null }
    $full = $PSCmdlet.GetUnresolvedProviderPathFromPSPath($Path)
    [IO.File]::WriteAllBytes($full, [Convert]::FromBase64String($r.data))
    $full
}

function Get-CdpConsole {
    <# Console output and exceptions collected so far (drains the event queue). #>
    param([Parameter(Mandatory)]$Cdp, [int]$WaitMs = 150)
    $deadline = (Get-Date).AddMilliseconds($WaitMs)
    while ((Get-Date) -lt $deadline) {
        if ($Cdp.Socket.State -ne 'Open') { break }
        try { $msg = Receive-CdpMessage $Cdp 60; if ($msg.PSObject.Properties['method']) { $Cdp.Events.Add($msg) } } catch { break }
    }
    $out = foreach ($e in $Cdp.Events) {
        switch ($e.method) {
            'Runtime.consoleAPICalled' { '[console.{0}] {1}' -f $e.params.type, (($e.params.args | ForEach-Object { if ($_.PSObject.Properties['value']) { $_.value } else { $_.description } }) -join ' ') }
            'Runtime.exceptionThrown'  { '[exception] {0}' -f $e.params.exceptionDetails.exception.description }
            'Log.entryAdded'           { '[log.{0}] {1}' -f $e.params.entry.level, $e.params.entry.text }
        }
    }
    $Cdp.Events.Clear()
    $out
}

function Close-Cdp {
    param([Parameter(Mandatory)]$Cdp)
    try { $Cdp.Socket.CloseAsync('NormalClosure', '', [Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null } catch {}
    $Cdp.Socket.Dispose()
}

Export-ModuleMember -Function Start-VexusApp, Stop-VexusApp, Connect-Cdp, Invoke-Cdp, Invoke-CdpEval, Save-CdpScreenshot, Get-CdpConsole, Close-Cdp
