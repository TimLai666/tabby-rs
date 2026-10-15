<#
.SYNOPSIS
Checks native desktop rendering on an isolated Windows CI runner.
.DESCRIPTION
Captures only the owned test window and verifies native maximize, restore and
cleanup. This does not accept Tabby, WebView2 or Windows 11 Snap behavior.
.PARAMETER OutputDirectory
Directory for result.json and client-area.png. Existing output files are refused.
.EXAMPLE
pwsh -NoProfile -File scripts/ci/probe-windows-desktop.ps1 -OutputDirectory "$env:RUNNER_TEMP/tabby-rs-windows-desktop-probe"
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'

$Kind = 'tabby-rs-windows-desktop-probe'
$FixedR = 19
$FixedG = 119
$FixedB = 209
$TotalTimeoutSeconds = 10

$script:checks = [System.Collections.Generic.List[object]]::new()
$script:errors = [System.Collections.Generic.List[string]]::new()
$script:ok = $true
$script:deadline = [DateTime]::UtcNow.AddSeconds($TotalTimeoutSeconds)

function Add-Check {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][bool]$Ok,
        [string]$Detail = ''
    )
    $script:checks.Add([ordered]@{ name = $Name; ok = $Ok; detail = $Detail })
    if (-not $Ok) { $script:ok = $false }
}

function Add-Error {
    param([Parameter(Mandatory = $true)][string]$Message)
    $script:errors.Add($Message)
    $script:ok = $false
}

function Wait-Until {
    param(
        [Parameter(Mandatory = $true)][scriptblock]$Condition,
        [int]$PollMilliseconds = 25
    )
    while ([DateTime]::UtcNow -lt $script:deadline) {
        [System.Windows.Forms.Application]::DoEvents()
        if (& $Condition) { return $true }
        Start-Sleep -Milliseconds $PollMilliseconds
    }
    return $false
}

$nativeSource = @'
using System;
using System.Runtime.InteropServices;

[StructLayout(LayoutKind.Sequential)]
public struct Win32Rect
{
    public int Left;
    public int Top;
    public int Right;
    public int Bottom;
}

[StructLayout(LayoutKind.Sequential)]
public struct Win32Point
{
    public int X;
    public int Y;
}

public static class Win32Native
{
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool IsWindow(IntPtr hWnd);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool IsZoomed(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool IsIconic(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);

    [DllImport("user32.dll")]
    public static extern uint GetDpiForWindow(IntPtr hWnd);

    [DllImport("dwmapi.dll")]
    public static extern int DwmFlush();

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool GetClientRect(IntPtr hWnd, out Win32Rect lpRect);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool ClientToScreen(IntPtr hWnd, ref Win32Point lpPoint);
}
'@

$environment = [ordered]@{
    probeKind              = $Kind
    timestampUtc           = (Get-Date).ToUniversalTime().ToString('o')
    osDescription          = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
    osArchitecture         = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    processArchitecture    = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
    psVersion              = $PSVersionTable.PSVersion.ToString()
    psEdition              = $PSVersionTable.PSEdition
    sessionName            = $env:SESSIONNAME
    userInteractive        = $null
    primaryScreenBounds    = $null
    virtualScreenBounds    = $null
    outputDirectory        = $OutputDirectory
    clientAreaPng          = $null
    totalTimeoutSeconds    = $TotalTimeoutSeconds
    scope                  = 'CI Windows desktop capability only. Does not verify Tauri, WebView2, Windows 11 Snap, or product parity.'
}

$resultPath = Join-Path $OutputDirectory 'result.json'
$pngPath = Join-Path $OutputDirectory 'client-area.png'
if ((Test-Path -LiteralPath $resultPath) -or (Test-Path -LiteralPath $pngPath)) {
    throw 'Probe output files already exist. Select a fresh output directory.'
}

$form = $null
$bitmap = $null
$graphics = $null
$handle = [IntPtr]::Zero
$previousDpiContext = [IntPtr]::Zero

try {
    if (-not (Test-Path -LiteralPath $OutputDirectory)) {
        New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
    }
    if (-not (Test-Path -LiteralPath $OutputDirectory -PathType Container)) {
        throw "OutputDirectory '$OutputDirectory' could not be created or is not a directory."
    }

    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    Add-Type -TypeDefinition $nativeSource -Language CSharp

    $previousDpiContext = [Win32Native]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
    if ($previousDpiContext -eq [IntPtr]::Zero) {
        throw 'Could not select per-monitor DPI awareness for the owned probe thread.'
    }
    $environment['dpiAwareness'] = 'PerMonitorV2 (owned thread only)'
    $environment['sessionId'] = [System.Diagnostics.Process]::GetCurrentProcess().SessionId

    $environment.userInteractive = [System.Windows.Forms.SystemInformation]::UserInteractive
    $environment.primaryScreenBounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds.ToString()
    $environment.virtualScreenBounds = [System.Windows.Forms.SystemInformation]::VirtualScreen.ToString()

    Add-Check -Name 'interactive-desktop' -Ok ([bool]$environment.userInteractive) `
        -Detail "SystemInformation.UserInteractive=$($environment.userInteractive); SESSIONNAME='$($env:SESSIONNAME)'"
    if (-not $environment.userInteractive) {
        Add-Error 'No interactive desktop session is available; native capture cannot be attempted.'
    } else {
        [System.Windows.Forms.Application]::EnableVisualStyles()

        $form = New-Object System.Windows.Forms.Form
        $form.Text = "tabby-rs-windows-desktop-probe-$([guid]::NewGuid().ToString('N'))"
        $form.BackColor = [System.Drawing.Color]::FromArgb($FixedR, $FixedG, $FixedB)
        $form.ForeColor = [System.Drawing.Color]::FromArgb($FixedR, $FixedG, $FixedB)
        $form.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::Sizable
        $form.StartPosition = [System.Windows.Forms.FormStartPosition]::Manual
        $form.Location = [System.Drawing.Point]::new(80, 80)
        $form.ClientSize = [System.Drawing.Size]::new(360, 240)
        $form.ShowInTaskbar = $true
        $form.TopMost = $false

        $form.Show()
        [System.Windows.Forms.Application]::DoEvents()
        $handle = $form.Handle

        $hwndOk = Wait-Until { [Win32Native]::IsWindow($handle) }
        Add-Check -Name 'hwnd-exists' -Ok $hwndOk `
            -Detail ("user32.IsWindow(handle=0x{0:X}) = {1}" -f $handle.ToInt64(), $hwndOk)

        $visible = Wait-Until { [Win32Native]::IsWindowVisible($handle) }
        Add-Check -Name 'hwnd-visible' -Ok $visible -Detail "user32.IsWindowVisible=$visible"

        $form.WindowState = [System.Windows.Forms.FormWindowState]::Maximized
        $form.Activate()
        $form.TopMost = $true
        $zoomed = Wait-Until { [Win32Native]::IsZoomed($handle) }
        Add-Check -Name 'hwnd-maximized' -Ok $zoomed `
            -Detail ("user32.IsZoomed(handle=0x{0:X}) after WindowState=Maximized = {1}" -f $handle.ToInt64(), $zoomed)

        $clientRect = [Win32Rect]::new()
        $rectOk = [Win32Native]::GetClientRect($handle, [ref]$clientRect)
        $origin = [Win32Point]::new()
        $origin.X = $clientRect.Left
        $origin.Y = $clientRect.Top
        $originOk = [Win32Native]::ClientToScreen($handle, [ref]$origin)
        $width = $clientRect.Right - $clientRect.Left
        $height = $clientRect.Bottom - $clientRect.Top
        $geometryOk = $rectOk -and $originOk -and $width -gt 0 -and $height -gt 0
        Add-Check -Name 'client-geometry' -Ok $geometryOk `
            -Detail "GetClientRect=$rectOk; ClientToScreen=$originOk; size=${width}x${height}; screenOrigin=($($origin.X),$($origin.Y))"

        if ($geometryOk) {
            try {
                $form.Refresh()
                [System.Windows.Forms.Application]::DoEvents()
                $flushResult = [Win32Native]::DwmFlush()
                Add-Check -Name 'compositor-flush' -Ok ($flushResult -eq 0) -Detail "dwmapi.DwmFlush HRESULT=$flushResult"
                $environment['clientArea'] = [ordered]@{
                    width = $width; height = $height; screenX = $origin.X; screenY = $origin.Y
                    dpi = [Win32Native]::GetDpiForWindow($handle)
                }
                $bitmap = [System.Drawing.Bitmap]::new([int]$width, [int]$height)
                $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
                $graphics.CopyFromScreen($origin.X, $origin.Y, 0, 0, [System.Drawing.Size]::new([int]$width, [int]$height))
                $pngStream = [System.IO.File]::Open($pngPath, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
                try {
                    $bitmap.Save($pngStream, [System.Drawing.Imaging.ImageFormat]::Png)
                } finally {
                    $pngStream.Dispose()
                }
                $captured = Test-Path -LiteralPath $pngPath -PathType Leaf
                $environment.clientAreaPng = $pngPath
                Add-Check -Name 'client-capture' -Ok $captured `
                    -Detail "Graphics.CopyFromScreen(own client area ${width}x${height}) -> $pngPath"

                if ($captured) {
                    $samples = [System.Collections.Generic.List[object]]::new()
                    $xs = @([int]($width / 4), [int]($width / 2), [int]($width * 3 / 4))
                    $ys = @([int]($height / 4), [int]($height / 2), [int]($height * 3 / 4))
                    $mismatches = [System.Collections.Generic.List[string]]::new()
                    foreach ($y in $ys) {
                        foreach ($x in $xs) {
                            $pixel = $bitmap.GetPixel($x, $y)
                            $isMatch = ($pixel.R -eq $FixedR) -and ($pixel.G -eq $FixedG) -and ($pixel.B -eq $FixedB)
                            $samples.Add([ordered]@{ x = $x; y = $y; rgb = "$($pixel.R),$($pixel.G),$($pixel.B)"; match = $isMatch })
                            if (-not $isMatch) {
                                $mismatches.Add("($x,$y)=$($pixel.R),$($pixel.G),$($pixel.B)")
                            }
                        }
                    }
                    $environment['captureSamples'] = $samples.ToArray()
                    Add-Check -Name 'background-color' -Ok ($mismatches.Count -eq 0) `
                        -Detail "expected RGB($FixedR,$FixedG,$FixedB); mismatches=$($mismatches.Count) $(($mismatches -join '; '))"
                }
            } catch {
                Add-Error "Client-area capture failed: $($_.Exception.Message)"
                Add-Check -Name 'client-capture' -Ok $false -Detail "CopyFromScreen threw: $($_.Exception.Message)"
            }
        } else {
            Add-Error 'Owned form client geometry could not be resolved; capture skipped.'
        }

        $form.TopMost = $false
        $form.WindowState = [System.Windows.Forms.FormWindowState]::Normal
        $restored = Wait-Until { [Win32Native]::IsWindow($handle) -and -not [Win32Native]::IsZoomed($handle) -and -not [Win32Native]::IsIconic($handle) }
        Add-Check -Name 'hwnd-restored' -Ok $restored `
            -Detail ("user32.IsZoomed(handle=0x{0:X}) after WindowState=Normal = {1}" -f $handle.ToInt64(), (-not $restored))
    }
} catch {
    Add-Error "Probe failed: $($_.Exception.Message)"
} finally {
    if ($null -ne $graphics) {
        try { $graphics.Dispose() } catch { Add-Error "Graphics.Dispose failed: $($_.Exception.Message)" }
        $graphics = $null
    }
    if ($null -ne $bitmap) {
        try { $bitmap.Dispose() } catch { Add-Error "Bitmap.Dispose failed: $($_.Exception.Message)" }
        $bitmap = $null
    }

    $destroyedOk = $false
    if ($null -ne $form) {
        try {
            if (-not $form.IsDisposed) { $form.Close() }
        } catch {
            Add-Error "Form.Close failed: $($_.Exception.Message)"
        }
        try { $form.Dispose() } catch { Add-Error "Form.Dispose failed: $($_.Exception.Message)" }
        if ($handle -ne [IntPtr]::Zero) {
            try { $destroyedOk = -not [Win32Native]::IsWindow($handle) } catch { $destroyedOk = $false }
        }
        $form = $null
    }

    if ($handle -ne [IntPtr]::Zero) {
        Add-Check -Name 'hwnd-destroyed' -Ok $destroyedOk `
            -Detail ("user32.IsWindow(handle=0x{0:X}) after Close/Dispose = {1}" -f $handle.ToInt64(), (-not $destroyedOk))
    } else {
        Add-Check -Name 'hwnd-destroyed' -Ok $false -Detail 'No owned HWND was created; destruction could not be verified.'
    }

    if ($previousDpiContext -ne [IntPtr]::Zero) {
        try {
            $restoredDpi = [Win32Native]::SetThreadDpiAwarenessContext($previousDpiContext)
            if ($restoredDpi -eq [IntPtr]::Zero) { Add-Error 'Could not restore the owned thread DPI context.' }
        } catch { Add-Error "DPI context restore failed: $($_.Exception.Message)" }
    }

    $result = [ordered]@{
        kind                   = $Kind
        ok                     = [bool]$script:ok
        productionTabbyVerified = $false
        win11SnapVerified      = $false
        environment            = $environment
        checks                 = $script:checks
        errors                 = $script:errors
    }

    try {
        $json = $result | ConvertTo-Json -Depth 8
        $resultStream = [System.IO.File]::Open($resultPath, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
        try {
            $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
            $resultStream.Write($bytes, 0, $bytes.Length)
        } finally {
            $resultStream.Dispose()
        }
    } catch {
        Write-Error "Could not write result.json: $($_.Exception.Message)"
    }
}

if ($script:ok) {
    Write-Output "Windows native desktop probe passed; result written to $resultPath"
    exit 0
}

Write-Output "Windows native desktop probe failed; see $resultPath"
exit 1
