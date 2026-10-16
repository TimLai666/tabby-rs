<#
.SYNOPSIS
Capture the existing visible native window belonging to an owned app process.
.DESCRIPTION
Captures the largest visible top-level HWND owned by -AppProcessId to
native-window.png and native-window.json inside the owned evidence directory.

This diagnostic only observes an already-visible window of an already-running
application. It never creates a test form, never calls ShowWindow, and never
makes a hidden window visible. It may briefly raise the existing owned visible
window to the top to obtain a composited frame and restores the previous
topmost state afterwards.

The native observation deadline starts after cold assembly preparation. All
native resources (bitmap, graphics, per-monitor DPI context) are restored in a
finally block. The result JSON records ok, HWND/title/outer/client geometry,
interactive session/OS/arch/elevation, the owned process tree, and the
screenshot path/hash/dimensions, or the exact error.

No parity pass is claimed: captured output is diagnostic evidence for manual
visual review only. This process stops nothing; cleanup of the app tree is the
caller's responsibility.
.PARAMETER AppProcessId
Positive PID of the full application process that owns the visible window.
.PARAMETER OutputDirectory
Owned app evidence directory. Existing native-window output files are refused.
.EXAMPLE
pwsh -NoProfile -File scripts/ci/capture-windows-app-window.ps1 -AppProcessId 1234 -OutputDirectory C:\owned\evidence
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [int]$AppProcessId,

    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'

$Kind = 'tabby-rs-windows-app-window-capture'
$ObservationSeconds = 30

# Native APIs and the Graphics capture path mirror scripts/ci/probe-windows-desktop.ps1.
$nativeSource = @'
using System;
using System.Runtime.InteropServices;
using System.Text;

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
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool IsWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool IsIconic(IntPtr hWnd);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool GetWindowRect(IntPtr hWnd, out Win32Rect lpRect);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool GetClientRect(IntPtr hWnd, out Win32Rect lpRect);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool ClientToScreen(IntPtr hWnd, ref Win32Point lpPoint);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern int GetWindowTextLength(IntPtr hWnd);

    [DllImport("user32.dll", EntryPoint = "GetWindowLongW", SetLastError = true)]
    private static extern int GetWindowLong32(IntPtr hWnd, int nIndex);

    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW", SetLastError = true)]
    private static extern IntPtr GetWindowLongPtr64(IntPtr hWnd, int nIndex);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);

    [DllImport("user32.dll")]
    public static extern uint GetDpiForWindow(IntPtr hWnd);

    [DllImport("dwmapi.dll")]
    public static extern int DwmFlush();

    [DllImport("dwmapi.dll")]
    public static extern int DwmGetWindowAttribute(IntPtr hwnd, int dwAttribute, out int pvAttribute, int cbAttribute);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern IntPtr OpenProcess(int dwDesiredAccess, bool bInheritHandle, int dwProcessId);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr hObject);

    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern bool OpenProcessToken(IntPtr ProcessHandle, uint DesiredAccess, out IntPtr TokenHandle);

    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern bool GetTokenInformation(IntPtr TokenHandle, int TokenInformationClass, out int TokenInformation, int TokenInformationLength, out int ReturnLength);

    // GWL_EXSTYLE = -20. WS_EX_TOPMOST = 0x8, WS_EX_TOOLWINDOW = 0x80.
    public static int GetWindowExStyle(IntPtr hWnd)
    {
        if (IntPtr.Size == 8)
        {
            return (int)GetWindowLongPtr64(hWnd, -20).ToInt64();
        }
        return GetWindowLong32(hWnd, -20);
    }

    // DWMWA_CLOAKED = 14. A cloaked window is not a real visible desktop window.
    public static int GetCloaked(IntPtr hWnd)
    {
        int value;
        int hr = DwmGetWindowAttribute(hWnd, 14, out value, 4);
        return hr == 0 ? value : 0;
    }

    public static IntPtr[] GetWindowsForProcess(int processId)
    {
        var windows = new System.Collections.Generic.List<IntPtr>();
        EnumWindows(delegate(IntPtr hWnd, IntPtr lParam)
        {
            uint pid;
            GetWindowThreadProcessId(hWnd, out pid);
            if (pid == (uint)processId)
            {
                windows.Add(hWnd);
            }
            return true;
        }, IntPtr.Zero);
        return windows.ToArray();
    }

    // TokenElevation = 20, TOKEN_QUERY = 0x0008, PROCESS_QUERY_LIMITED_INFORMATION = 0x1000.
    // Returns bool when readable, null when the token cannot be queried.
    public static object IsProcessElevated(int processId)
    {
        IntPtr process = OpenProcess(0x1000, false, processId);
        if (process == IntPtr.Zero)
        {
            return null;
        }
        try
        {
            IntPtr token;
            if (!OpenProcessToken(process, 0x0008, out token))
            {
                return null;
            }
            try
            {
                int elevated;
                int returned;
                if (!GetTokenInformation(token, 20, out elevated, 4, out returned))
                {
                    return null;
                }
                return elevated != 0;
            }
            finally
            {
                CloseHandle(token);
            }
        }
        finally
        {
            CloseHandle(process);
        }
    }
}
'@

# Runs on a dedicated helper thread (via its own runspace) so the per-monitor DPI
# awareness context is scoped to the capture thread and restored on completion.
$helperSource = @'
param(
    [int]$TargetPid,
    [string]$PngPath,
    [long]$DeadlineUnixMs
)

$ErrorActionPreference = 'Stop'

$capture = [ordered]@{
    found              = $false
    hwnd               = $null
    hwndValue          = 0L
    title              = $null
    outer              = $null
    client             = $null
    topmostOriginally  = $null
    topmostApplied     = $false
    topmostRestored    = $null
    cloaked            = $null
    screenshotWritten  = $false
    screenshotWidth    = 0
    screenshotHeight   = 0
    dpiContext         = $null
    proceduralError    = $null
}

$previousDpi = [IntPtr]::Zero
$topmostChanged = $false
$hwnd = [IntPtr]::Zero
$bitmap = $null
$graphics = $null

function Get-BestVisibleWindow {
    param([int]$ProcessId)
    $best = [IntPtr]::Zero
    $bestArea = [int64]-1
    foreach ($candidate in [Win32Native]::GetWindowsForProcess($ProcessId)) {
        if (-not [Win32Native]::IsWindow($candidate)) { continue }
        if (-not [Win32Native]::IsWindowVisible($candidate)) { continue }
        if ([Win32Native]::IsIconic($candidate)) { continue }
        if ([Win32Native]::GetCloaked($candidate) -ne 0) { continue }
        $rect = [Win32Rect]::new()
        if (-not [Win32Native]::GetWindowRect($candidate, [ref]$rect)) { continue }
        $width = $rect.Right - $rect.Left
        $height = $rect.Bottom - $rect.Top
        if ($width -le 0 -or $height -le 0) { continue }
        $area = [int64]$width * [int64]$height
        if ($area -gt $bestArea) { $bestArea = $area; $best = $candidate }
    }
    return $best
}

try {
    $previousDpi = [Win32Native]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
    if ($previousDpi -eq [IntPtr]::Zero) {
        throw 'SetThreadDpiAwarenessContext could not select per-monitor DPI awareness for the capture helper thread.'
    }
    $capture.dpiContext = 'PerMonitorV2 (capture helper thread only)'

    while ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() -lt $DeadlineUnixMs) {
        $hwnd = Get-BestVisibleWindow -ProcessId $TargetPid
        if ($hwnd -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 250
    }

    if ($hwnd -eq [IntPtr]::Zero) {
        throw "No visible, non-minimized, non-cloaked top-level window owned by PID $TargetPid appeared before the observation deadline."
    }

    $capture.found = $true
    $capture.hwnd = ('0x{0:X}' -f $hwnd.ToInt64())
    $capture.hwndValue = $hwnd.ToInt64()
    $capture.cloaked = [Win32Native]::GetCloaked($hwnd)

    $titleLength = [Win32Native]::GetWindowTextLength($hwnd)
    if ($titleLength -gt 0) {
        $builder = [System.Text.StringBuilder]::new($titleLength + 1)
        [void][Win32Native]::GetWindowText($hwnd, $builder, $builder.Capacity)
        $capture.title = $builder.ToString()
    } else {
        $capture.title = ''
    }

    $outer = [Win32Rect]::new()
    if (-not [Win32Native]::GetWindowRect($hwnd, [ref]$outer)) {
        throw 'GetWindowRect failed for the owned window.'
    }
    $client = [Win32Rect]::new()
    if (-not [Win32Native]::GetClientRect($hwnd, [ref]$client)) {
        throw 'GetClientRect failed for the owned window.'
    }
    $origin = [Win32Point]::new()
    $origin.X = $client.Left
    $origin.Y = $client.Top
    if (-not [Win32Native]::ClientToScreen($hwnd, [ref]$origin)) {
        throw 'ClientToScreen failed for the owned window.'
    }

    $outerWidth = $outer.Right - $outer.Left
    $outerHeight = $outer.Bottom - $outer.Top
    $clientWidth = $client.Right - $client.Left
    $clientHeight = $client.Bottom - $client.Top
    if ($outerWidth -le 0 -or $outerHeight -le 0 -or $clientWidth -le 0 -or $clientHeight -le 0) {
        throw "Owned window geometry is empty (outer ${outerWidth}x${outerHeight}, client ${clientWidth}x${clientHeight})."
    }

    $capture.outer = [ordered]@{
        left = $outer.Left; top = $outer.Top; right = $outer.Right; bottom = $outer.Bottom
        width = $outerWidth; height = $outerHeight
    }
    $capture.client = [ordered]@{
        width = $clientWidth; height = $clientHeight; screenX = $origin.X; screenY = $origin.Y
        dpi = [Win32Native]::GetDpiForWindow($hwnd)
    }

    # Temporarily raise the existing owned visible window; restore topmost state afterwards.
    # SWP_NOSIZE(0x1) | SWP_NOMOVE(0x2) | SWP_NOACTIVATE(0x10). HWND_TOPMOST=-1, HWND_NOTOPMOST=-2.
    $topmostOriginally = ([Win32Native]::GetWindowExStyle($hwnd) -band 0x00000008) -ne 0
    $capture.topmostOriginally = $topmostOriginally
    $setWindowPosFlags = [uint32](0x0001 -bor 0x0002 -bor 0x0010)
    if (-not $topmostOriginally) {
        if ([Win32Native]::SetWindowPos($hwnd, [IntPtr]::new(-1), 0, 0, 0, 0, $setWindowPosFlags)) {
            $topmostChanged = $true
            $capture.topmostApplied = $true
        } else {
            throw 'SetWindowPos could not raise the existing owned visible window for capture.'
        }
    }

    $flushResult = [Win32Native]::DwmFlush()
    if ($flushResult -ne 0) { throw "DwmFlush failed with HRESULT $flushResult." }
    # Give the compositor one presentation interval after the flush before sampling.
    Start-Sleep -Milliseconds 150

    $bitmap = [System.Drawing.Bitmap]::new([int]$outerWidth, [int]$outerHeight)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($outer.Left, $outer.Top, 0, 0, [System.Drawing.Size]::new([int]$outerWidth, [int]$outerHeight))
    $pngStream = [System.IO.File]::Open($PngPath, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
    try {
        $bitmap.Save($pngStream, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $pngStream.Dispose()
    }
    $capture.screenshotWritten = Test-Path -LiteralPath $PngPath -PathType Leaf
    $capture.screenshotWidth = [int]$outerWidth
    $capture.screenshotHeight = [int]$outerHeight
} catch {
    $capture.proceduralError = $_.Exception.Message
} finally {
    if ($topmostChanged -and $hwnd -ne [IntPtr]::Zero) {
        try {
            $capture.topmostRestored = [Win32Native]::SetWindowPos($hwnd, [IntPtr]::new(-2), 0, 0, 0, 0, [uint32](0x0001 -bor 0x0002 -bor 0x0010))
        } catch {
            $capture.topmostRestored = $false
        }
    } elseif ($capture.topmostOriginally -eq $true) {
        $capture.topmostRestored = $true
    }

    if ($null -ne $graphics) {
        try { $graphics.Dispose() } catch { }
        $graphics = $null
    }
    if ($null -ne $bitmap) {
        try { $bitmap.Dispose() } catch { }
        $bitmap = $null
    }
    if ($topmostChanged -and $capture.topmostRestored -ne $true) {
        $capture.proceduralError = 'Failed to restore the owned window topmost state after capture.'
    }
    if ($previousDpi -ne [IntPtr]::Zero) {
        try {
            if ([Win32Native]::SetThreadDpiAwarenessContext($previousDpi) -eq [IntPtr]::Zero) { throw 'DPI restoration failed' }
        } catch { $capture.proceduralError = 'Failed to restore capture helper thread DPI awareness.' }
    }
}

[pscustomobject]$capture
'@

function Get-OwnedProcessTree {
    param([int]$RootProcessId)

    $processes = Get-CimInstance -ClassName Win32_Process -Property ProcessId, ParentProcessId, CreationDate, Name -ErrorAction Stop
    $children = @{}
    foreach ($process in $processes) {
        $parent = [int]$process.ParentProcessId
        if (-not $children.ContainsKey($parent)) {
            $children[$parent] = New-Object System.Collections.Generic.List[int]
        }
        $children[$parent].Add([int]$process.ProcessId)
    }

    $entries = New-Object System.Collections.Generic.List[object]
    $stack = New-Object System.Collections.Generic.Stack[int]
    $stack.Push($RootProcessId)
    $seen = @{}
    while ($stack.Count -gt 0) {
        $id = $stack.Pop()
        if ($seen.ContainsKey($id)) { continue }
        $seen[$id] = $true
        $match = $processes | Where-Object { [int]$_.ProcessId -eq $id } | Select-Object -First 1
        if ($match) {
            $created = $null
            if ($null -ne $match.CreationDate) {
                $created = ([datetime]$match.CreationDate).ToUniversalTime().ToString('o')
            }
            $entries.Add([ordered]@{
                pid                = [int]$match.ProcessId
                parentPid          = [int]$match.ParentProcessId
                creationTimeUTC    = $created
                executableBasename = [string]$match.Name
            })
        }
        if ($match -and $children.ContainsKey($id)) {
            foreach ($child in $children[$id]) {
                $candidate = $processes | Where-Object { [int]$_.ProcessId -eq $child } | Select-Object -First 1
                if ($candidate -and $candidate.CreationDate -ge $match.CreationDate) { $stack.Push([int]$child) }
            }
        }
    }
    return $entries.ToArray()
}

$resultPath = Join-Path $OutputDirectory 'native-window.json'
$pngPath = Join-Path $OutputDirectory 'native-window.png'

if (-not [System.IO.Path]::IsPathRooted($OutputDirectory)) {
    throw 'OutputDirectory must be an absolute existing owned evidence directory.'
}
if (-not (Test-Path -LiteralPath $OutputDirectory -PathType Container)) {
    throw "OutputDirectory '$OutputDirectory' is not a directory."
}
if ((Test-Path -LiteralPath $resultPath) -or (Test-Path -LiteralPath $pngPath)) {
    throw 'Refusing to overwrite existing native-window output files. Select a fresh evidence directory.'
}

$ok = $false
$errorMessage = $null
$capture = $null
$captureHwnd = $null
$captureTitle = $null
$outerGeometry = $null
$clientGeometry = $null
$processTree = @()
$processTreeError = $null
$screenshot = $null
$window = [ordered]@{ topmostOriginally = $null; topmostApplied = $false; topmostRestored = $null; cloaked = $null }

$environment = [ordered]@{
    osDescription          = $null
    osArchitecture         = $null
    processArchitecture    = $null
    psVersion              = $null
    psEdition              = $null
    sessionName            = $env:SESSIONNAME
    sessionId              = $null
    appSessionId           = $null
    userInteractive        = $null
    captureProcessElevated = $null
    appProcessElevated     = $null
    primaryScreenBounds    = $null
    virtualScreenBounds    = $null
    dpiContext             = $null
}

try {
    if ($AppProcessId -le 0) {
        throw "AppProcessId must be a positive PID, received $AppProcessId."
    }

    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    Add-Type -TypeDefinition $nativeSource -Language CSharp

    # Start the native observation deadline after cold assembly preparation.
    $observationDeadlineUnixMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() + ([int64]$ObservationSeconds * 1000)

    $environment.osDescription = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
    $environment.osArchitecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    $environment.processArchitecture = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
    $environment.psVersion = $PSVersionTable.PSVersion.ToString()
    $environment.psEdition = $PSVersionTable.PSEdition
    $environment.sessionId = [System.Diagnostics.Process]::GetCurrentProcess().SessionId
    $environment.userInteractive = [System.Windows.Forms.SystemInformation]::UserInteractive
    $environment.primaryScreenBounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds.ToString()
    $environment.virtualScreenBounds = [System.Windows.Forms.SystemInformation]::VirtualScreen.ToString()
    try {
        $identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
        $principal = New-Object System.Security.Principal.WindowsPrincipal($identity)
        $environment.captureProcessElevated = $principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)
    } catch {
        $environment.captureProcessElevated = $null
    }

    $appProcess = $null
    try {
        $appProcess = Get-Process -Id $AppProcessId -ErrorAction Stop
    } catch {
        throw "AppProcessId $AppProcessId is not a running process: $($_.Exception.Message)"
    }
    try {
        $environment.appSessionId = $appProcess.SessionId
    } catch {
        $environment.appSessionId = $null
    }
    $environment.appProcessElevated = [Win32Native]::IsProcessElevated($AppProcessId)

    try {
        $processTree = Get-OwnedProcessTree -RootProcessId $AppProcessId
    } catch {
        $processTree = @()
        $processTreeError = $_.Exception.Message
    }

    $runspace = $null
    $powerShell = $null
    try {
        $runspace = [System.Management.Automation.Runspaces.RunspaceFactory]::CreateRunspace()
        $runspace.Open()
        $powerShell = [powershell]::Create()
        $powerShell.Runspace = $runspace
        [void]$powerShell.AddScript($helperSource)
        [void]$powerShell.AddArgument($AppProcessId)
        [void]$powerShell.AddArgument($pngPath)
        [void]$powerShell.AddArgument([int64]$observationDeadlineUnixMs)

        $asyncResult = $powerShell.BeginInvoke()
        $waitBudget = $observationDeadlineUnixMs - [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() + 15000
        if ($waitBudget -lt 1000) { $waitBudget = 1000 }
        if (-not $asyncResult.AsyncWaitHandle.WaitOne([int]$waitBudget)) {
            $powerShell.Stop()
            throw "Native window capture helper exceeded its observation budget (${ObservationSeconds}s plus assembly preparation)."
        }

        $helperOutput = $powerShell.EndInvoke($asyncResult)
        $streamErrors = @($powerShell.Streams.Error | ForEach-Object { $_.ToString() })
        if ($helperOutput.Count -gt 0) {
            $capture = $helperOutput[$helperOutput.Count - 1]
        } elseif ($streamErrors.Count -gt 0) {
            throw "Native window capture helper reported errors: $($streamErrors -join '; ')"
        } else {
            throw 'Native window capture helper returned no result.'
        }
    } finally {
        if ($null -ne $powerShell) { $powerShell.Dispose() }
        if ($null -ne $runspace) { $runspace.Close(); $runspace.Dispose() }
    }

    if ($null -eq $capture) {
        throw 'Native window capture helper returned no result.'
    }

    $captureHwnd = $capture.hwnd
    $captureTitle = $capture.title
    $outerGeometry = $capture.outer
    $clientGeometry = $capture.client
    $window = [ordered]@{
        topmostOriginally = $capture.topmostOriginally
        topmostApplied    = [bool]$capture.topmostApplied
        topmostRestored   = $capture.topmostRestored
        cloaked           = $capture.cloaked
    }
    $environment.dpiContext = $capture.dpiContext

    if ($capture.proceduralError) {
        throw [string]$capture.proceduralError
    }
    if (-not $capture.screenshotWritten) {
        throw 'Native window capture did not produce a PNG file.'
    }

    $screenshotHash = (Get-FileHash -LiteralPath $pngPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $screenshot = [ordered]@{
        path   = $pngPath
        sha256 = $screenshotHash
        width  = [int]$capture.screenshotWidth
        height = [int]$capture.screenshotHeight
    }
    $ok = $true
} catch {
    $errorMessage = $_.Exception.Message
}

$result = [ordered]@{
    kind                      = $Kind
    ok                        = [bool]$ok
    appProcessId              = $AppProcessId
    capturedUtc               = (Get-Date).ToUniversalTime().ToString('o')
    hwnd                      = $captureHwnd
    title                     = $captureTitle
    outer                     = $outerGeometry
    client                    = $clientGeometry
    window                    = $window
    environment               = $environment
    processTree               = @($processTree)
    processTreeError          = $processTreeError
    screenshot                = $screenshot
    observationSeconds        = $ObservationSeconds
    error                     = $errorMessage
    fullParityVerified        = $false
    manualVisualReviewRequired = $true
    scope                     = 'Diagnostic capture of an existing visible owned HWND. Does not create, show, or activate application windows, and does not accept product parity.'
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
    Write-Error "Could not write native-window.json: $($_.Exception.Message)"
}

if ($ok) {
    Write-Output "Captured owned native window for PID $AppProcessId; evidence written to $resultPath and $pngPath"
    exit 0
}

Write-Output "Native window capture failed for PID $AppProcessId; see $resultPath"
exit 1
