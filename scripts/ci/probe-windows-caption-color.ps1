<#
.SYNOPSIS
Capture the actual desktop maximize/restore button and verify its configured foreground.
.DESCRIPTION
Read-only own-window probe. Requires the exact root HWND, PID and CIM UTC creation
identity. Writes caption-color.json and caption-color.png using CreateNew. Exit 0
requires a valid owned crop with at least 20 exact configured RGB pixels; exit 1
reports prerequisite, color, capture or evidence failures separately. Does not
activate, resize, invoke controls, send input, or terminate any process.
.PARAMETER AppProcessId
Positive process ID recorded by the owned application launcher.
.PARAMETER CreationUtc
Exact CIM CreationDate.ToUniversalTime().ToString('o') from that launcher.
.PARAMETER WindowHandle
Positive decimal root HWND owned by the same process.
.PARAMETER ExpectedForeground
Required foreground in #RRGGBB form.
.PARAMETER OutputDirectory
Existing fresh evidence directory. Existing evidence files are never overwritten.
.EXAMPLE
powershell.exe -NoProfile -File probe-windows-caption-color.ps1 -AppProcessId 1234 -CreationUtc 2026-10-17T00:00:00.0000000Z -WindowHandle 12345 -ExpectedForeground '#b050d0' -OutputDirectory C:\owned\caption-color
#>
param(
    [Parameter(Mandatory=$true)][int]$AppProcessId,
    [Parameter(Mandatory=$true)][string]$CreationUtc,
    [Parameter(Mandatory=$true)][long]$WindowHandle,
    [Parameter(Mandatory=$true)][string]$ExpectedForeground,
    [Parameter(Mandatory=$true)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
$expectedRed = 0
$expectedGreen = 0
$expectedBlue = 0
$pngPath = Join-Path $OutputDirectory 'caption-color.png'
$jsonPath = Join-Path $OutputDirectory 'caption-color.json'
$evidence = [ordered]@{
    ok = $false
    status = 'prerequisite_failed'
    error = $null
    prerequisiteStatus = 'not_started'
    expectedForeground = $ExpectedForeground
    requiredExactPixels = 20
    exactColorPixelCount = $null
    root = [ordered]@{
        processId = $AppProcessId
        handle = ('0x{0:X}' -f $WindowHandle)
        creationUtc = $CreationUtc
        threadId = $null
        maximized = $null
    }
    button = $null
    crop = $null
    points = [Collections.Generic.List[object]]::new()
}
$bitmap = $null
$graphics = $null
$previousDpi = [IntPtr]::Zero

try {
    if ($AppProcessId -le 0 -or $WindowHandle -le 0) { throw 'Positive owned PID and root HWND are required' }
    if ($ExpectedForeground -cnotmatch '^#[0-9A-Fa-f]{6}$') { throw 'ExpectedForeground must be #RRGGBB' }
    $expectedRed = [Convert]::ToInt32($ExpectedForeground.Substring(1, 2), 16)
    $expectedGreen = [Convert]::ToInt32($ExpectedForeground.Substring(3, 2), 16)
    $expectedBlue = [Convert]::ToInt32($ExpectedForeground.Substring(5, 2), 16)
    if (-not [IO.Directory]::Exists($OutputDirectory)) { throw 'OutputDirectory must already exist' }

    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    Add-Type -AssemblyName WindowsBase
    Add-Type -AssemblyName System.Drawing

    $providerAssembly = [Reflection.Assembly]::Load('UIAutomationClientSideProviders')
    Add-Type -ReferencedAssemblies @(
        [System.Windows.Automation.AutomationElement].Assembly.Location,
        [System.Windows.Automation.ControlType].Assembly.Location,
        $providerAssembly.Location
    ) -TypeDefinition @'
public static class CaptionColorAutomationProviders {
    public static void Register() {
        System.Windows.Automation.ClientSettings.RegisterClientSideProviders(
            UIAutomationClientsideProviders.UIAutomationClientSideProviders.ClientSideProviderDescriptionTable);
    }
}
'@
    [CaptionColorAutomationProviders]::Register()

    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class CaptionColorNative {
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int x; public int y; }
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool IsWindow(IntPtr hWnd);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool IsIconic(IntPtr hWnd);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool IsZoomed(IntPtr hWnd);
    [DllImport("user32.dll", SetLastError=true)] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr GetAncestor(IntPtr hWnd, uint flags);
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr WindowFromPoint(POINT point);
    public static IntPtr WindowAt(int x, int y) { return WindowFromPoint(new POINT { x=x, y=y }); }
    [DllImport("user32.dll", SetLastError=true, EntryPoint="SendMessageTimeoutW")]
    private static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint message, IntPtr wParam, IntPtr lParam, uint flags, uint timeout, out IntPtr result);
    public static int HitTest(IntPtr hWnd, int x, int y) {
        int packed = unchecked((int)(((uint)x & 0xffffU) | (((uint)y & 0xffffU) << 16)));
        IntPtr result;
        IntPtr sent = SendMessageTimeout(hWnd, 0x0084U, IntPtr.Zero, new IntPtr(packed), 0x0001U | 0x0002U, 1000U, out result);
        if (sent == IntPtr.Zero) throw new InvalidOperationException("WM_NCHITTEST failed or timed out");
        return result.ToInt32();
    }
}
'@

    $previousDpi = [CaptionColorNative]::SetThreadDpiAwarenessContext([IntPtr](-4))
    if ($previousDpi -eq [IntPtr]::Zero) { throw 'Could not set per-monitor-v2 DPI awareness' }

    function Assert-RootIdentity {
        $rootHandle = [IntPtr]$WindowHandle
        if (-not [CaptionColorNative]::IsWindow($rootHandle)) { throw 'WindowHandle is not a live HWND' }
        if (-not [CaptionColorNative]::IsWindowVisible($rootHandle)) { throw 'Root HWND is not visible' }
        if ([CaptionColorNative]::IsIconic($rootHandle)) { throw 'Root HWND is minimized' }
        if ([CaptionColorNative]::GetAncestor($rootHandle, 2) -ne $rootHandle) { throw 'WindowHandle is not the exact root HWND' }
        [uint32]$actualProcessId = 0
        $threadId = [CaptionColorNative]::GetWindowThreadProcessId($rootHandle, [ref]$actualProcessId)
        if ($actualProcessId -ne $AppProcessId -or $threadId -eq 0) { throw 'Root HWND process/thread ownership mismatch' }
        $process = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = $AppProcessId" -ErrorAction Stop
        if ($null -eq $process) { throw 'Owned process does not exist in CIM' }
        $actualCreationUtc = $process.CreationDate.ToUniversalTime().ToString('o')
        if ($actualCreationUtc -cne $CreationUtc) { throw 'CreationUtc does not match the exact process identity' }
        $evidence.root.threadId = [uint32]$threadId
        $evidence.root.maximized = [bool][CaptionColorNative]::IsZoomed($rootHandle)
    }

    function Get-ElementFacts {
        param([System.Windows.Automation.AutomationElement]$element)
        if ($null -eq $element) { return $null }
        $current = $element.Current
        return [ordered]@{
            role = [string]$current.ControlType.ProgrammaticName
            name = [string]$current.Name
            processId = [int]$current.ProcessId
            nativeWindowHandle = ('0x{0:X}' -f [long]$current.NativeWindowHandle)
            isControlElement = [bool]$current.IsControlElement
            isContentElement = [bool]$current.IsContentElement
        }
    }

    function Assert-DescendantPointOwnership {
        param([int]$x, [int]$y)
        $hitWindow = [CaptionColorNative]::WindowAt($x, $y)
        if ($hitWindow -eq [IntPtr]::Zero) { throw 'WindowFromPoint did not return a live HWND' }
        [uint32]$hitProcessId = 0
        [void][CaptionColorNative]::GetWindowThreadProcessId($hitWindow, [ref]$hitProcessId)
        $hitRoot = [CaptionColorNative]::GetAncestor($hitWindow, 2)
        $chain = [Collections.Generic.List[object]]::new()
        $evidence.pointOwnership = @{ hwnd=('0x{0:X}' -f $hitWindow.ToInt64()); processId=$hitProcessId; rootHwnd=('0x{0:X}' -f $hitRoot.ToInt64()); processChain=$chain }
        if ($hitRoot -ne [IntPtr]$WindowHandle) { throw 'WindowFromPoint HWND is outside the exact owned root' }
        # WebView2 native children run in a separate browser process. Require
        # both the exact HWND hierarchy and a live creation-ordered process chain.
        $nextId = [int]$hitProcessId
        $seen = @{}
        $childCreation = [DateTime]::MaxValue
        while ($nextId -ne $AppProcessId) {
            if ($nextId -le 0 -or $seen.ContainsKey($nextId) -or $chain.Count -ge 16) { throw 'Point process ancestry is invalid' }
            $seen[$nextId] = $true
            $entry = Get-CimInstance Win32_Process -Filter "ProcessId = $nextId" -ErrorAction Stop
            if (-not $entry -or $entry.CreationDate -gt $childCreation -or $entry.CreationDate.ToUniversalTime() -lt ([DateTime]$CreationUtc).ToUniversalTime()) { throw 'Point process is missing, reused, or older than the owned root' }
            $chain.Add(@{ pid=[int]$entry.ProcessId; parentPid=[int]$entry.ParentProcessId; creationUtc=$entry.CreationDate.ToUniversalTime().ToString('o') })
            $childCreation = $entry.CreationDate
            $nextId = [int]$entry.ParentProcessId
        }
        Assert-RootIdentity
        [uint32]$freshHitProcessId = 0
        [void][CaptionColorNative]::GetWindowThreadProcessId($hitWindow, [ref]$freshHitProcessId)
        if ($freshHitProcessId -ne $hitProcessId -or [CaptionColorNative]::GetAncestor($hitWindow, 2) -ne [IntPtr]$WindowHandle) { throw 'Point HWND identity changed during ancestry validation' }
        return $hitWindow
    }

    function Get-BoundedControlDescendants {
        param([System.Windows.Automation.AutomationElement]$rootElement)
        $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
        $descendants = [Collections.Generic.List[System.Windows.Automation.AutomationElement]]::new()
        $pending = [Collections.Generic.Stack[System.Windows.Automation.AutomationElement]]::new()
        $child = $walker.GetFirstChild($rootElement)
        $children = [Collections.Generic.List[System.Windows.Automation.AutomationElement]]::new()
        while ($null -ne $child) {
            if (($descendants.Count + $pending.Count + $children.Count + 1) -gt 200) { throw 'UIA root has more than 200 descendants' }
            $children.Add($child)
            $child = $walker.GetNextSibling($child)
        }
        for ($childIndex = $children.Count - 1; $childIndex -ge 0; $childIndex--) { $pending.Push($children[$childIndex]) }
        while ($pending.Count -gt 0) {
            $node = $pending.Pop()
            $descendants.Add($node)
            $children = [Collections.Generic.List[System.Windows.Automation.AutomationElement]]::new()
            $child = $walker.GetFirstChild($node)
            while ($null -ne $child) {
                if (($descendants.Count + $pending.Count + $children.Count + 1) -gt 200) { throw 'UIA root has more than 200 descendants' }
                $children.Add($child)
                $child = $walker.GetNextSibling($child)
            }
            for ($childIndex = $children.Count - 1; $childIndex -ge 0; $childIndex--) { $pending.Push($children[$childIndex]) }
        }
        return $descendants.ToArray()
    }

    Assert-RootIdentity
    $root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$WindowHandle)
    if ($null -eq $root) { throw 'UIA could not resolve the exact root HWND' }
    $rootFacts = Get-ElementFacts $root
    if ($rootFacts.processId -ne $AppProcessId -or $rootFacts.nativeWindowHandle -cne ('0x{0:X}' -f $WindowHandle)) {
        throw 'UIA root identity does not match the exact owned HWND'
    }
    $desiredName = if ($evidence.root.maximized) { 'Restore' } else { 'Maximize' }
    $allDescendants = @(Get-BoundedControlDescendants -rootElement $root)

    $matches = [Collections.Generic.List[object]]::new()
    for ($nodeIndex = 0; $nodeIndex -lt $allDescendants.Count; $nodeIndex++) {
        $element = $allDescendants[$nodeIndex]
        $current = $element.Current
        if ([int]$current.ProcessId -ne $AppProcessId) { continue }
        if ([string]$current.ControlType.ProgrammaticName -cne 'ControlType.Button' -or
            [string]$current.Name -cne $desiredName -or -not [bool]$current.IsEnabled -or [bool]$current.IsOffscreen) { continue }
        $bounds = $current.BoundingRectangle
        if ([double]::IsNaN($bounds.X) -or [double]::IsNaN($bounds.Y) -or [double]::IsNaN($bounds.Width) -or [double]::IsNaN($bounds.Height) -or
            [double]::IsInfinity($bounds.X) -or [double]::IsInfinity($bounds.Y) -or [double]::IsInfinity($bounds.Width) -or [double]::IsInfinity($bounds.Height) -or
            $bounds.Width -le 0 -or $bounds.Height -le 0) { continue }
        $centerX = [int][Math]::Floor($bounds.X + ($bounds.Width / 2.0))
        $centerY = [int][Math]::Floor($bounds.Y + ($bounds.Height / 2.0))
        if ([CaptionColorNative]::HitTest([IntPtr]$WindowHandle, $centerX, $centerY) -ne 9) { continue }
        $matches.Add(@{ element=$element; bounds=$bounds; centerX=$centerX; centerY=$centerY })
    }
    if ($matches.Count -ne 1) { throw "Expected exactly one visible enabled $desiredName Button with HTMAXBUTTON bounds; found $($matches.Count)" }

    $selected = $matches[0]
    $selectedElement = [System.Windows.Automation.AutomationElement]$selected.element
    $selectedBounds = $selected.bounds
    $selectedFacts = Get-ElementFacts $selectedElement
    $evidence.button = [ordered]@{
        role = $selectedFacts.role
        name = $selectedFacts.name
        processId = $selectedFacts.processId
        nativeWindowHandle = $selectedFacts.nativeWindowHandle
        isControlElement = $selectedFacts.isControlElement
        isContentElement = $selectedFacts.isContentElement
        enabled = [bool]$selectedElement.Current.IsEnabled
        offscreen = [bool]$selectedElement.Current.IsOffscreen
        bounds = [ordered]@{ x=[double]$selectedBounds.X; y=[double]$selectedBounds.Y; width=[double]$selectedBounds.Width; height=[double]$selectedBounds.Height }
    }

    $pointSpecs = @(
        @{ name='center'; x=($selectedBounds.X + ($selectedBounds.Width / 2.0)); y=($selectedBounds.Y + ($selectedBounds.Height / 2.0)) },
        @{ name='glyph-outline'; x=($selectedBounds.X + ($selectedBounds.Width / 2.0) - 5.0); y=($selectedBounds.Y + ($selectedBounds.Height / 2.0) - 5.0) },
        @{ name='top-left-hover'; x=($selectedBounds.X + 3.0); y=($selectedBounds.Y + 3.0) }
    )
    foreach ($pointSpec in $pointSpecs) {
        $rawElement = [System.Windows.Automation.AutomationElement]::FromPoint([System.Windows.Point]::new([double]$pointSpec.x, [double]$pointSpec.y))
        $controlElement = if ($null -eq $rawElement) { $null } else { [System.Windows.Automation.TreeWalker]::ControlViewWalker.Normalize($rawElement) }
        $contentElement = if ($null -eq $rawElement) { $null } else { [System.Windows.Automation.TreeWalker]::ContentViewWalker.Normalize($rawElement) }
        $evidence.points.Add([ordered]@{
            name = $pointSpec.name
            x = [double]$pointSpec.x
            y = [double]$pointSpec.y
            raw = (Get-ElementFacts $rawElement)
            controlNormalized = (Get-ElementFacts $controlElement)
            contentNormalized = (Get-ElementFacts $contentElement)
        })
    }

    $cropX = [int][Math]::Floor($selectedBounds.X)
    $cropY = [int][Math]::Floor($selectedBounds.Y)
    $cropRight = [int][Math]::Ceiling($selectedBounds.X + $selectedBounds.Width)
    $cropBottom = [int][Math]::Ceiling($selectedBounds.Y + $selectedBounds.Height)
    $cropWidth = $cropRight - $cropX
    $cropHeight = $cropBottom - $cropY
    if ($cropWidth -lt 1 -or $cropHeight -lt 1 -or $cropWidth -gt 256 -or $cropHeight -gt 256) { throw 'Caption button crop dimensions must be between 1 and 256 pixels' }

    # Recheck process/HWND identity, UIA identity and geometry, hit-test, and desktop ownership immediately before capture.
    Assert-RootIdentity
    $freshButton = $selectedElement.Current
    if ([int]$freshButton.ProcessId -ne $AppProcessId -or [string]$freshButton.ControlType.ProgrammaticName -cne 'ControlType.Button' -or
        [string]$freshButton.Name -cne $desiredName -or -not [bool]$freshButton.IsEnabled -or [bool]$freshButton.IsOffscreen) {
        throw 'Selected UIA Button identity or state changed before crop'
    }
    if (('0x{0:X}' -f [long]$freshButton.NativeWindowHandle) -cne $evidence.button.nativeWindowHandle) { throw 'Selected UIA native handle identity changed before crop' }
    $freshBounds = $freshButton.BoundingRectangle
    if ($freshBounds.X -ne $selectedBounds.X -or $freshBounds.Y -ne $selectedBounds.Y -or
        $freshBounds.Width -ne $selectedBounds.Width -or $freshBounds.Height -ne $selectedBounds.Height) { throw 'Caption button geometry changed before crop' }
    if ([CaptionColorNative]::HitTest([IntPtr]$WindowHandle, $selected.centerX, $selected.centerY) -ne 9) { throw 'Root no longer reports HTMAXBUTTON at the selected center' }
    $pointWindow = Assert-DescendantPointOwnership -x $selected.centerX -y $selected.centerY
    $evidence.crop = [ordered]@{ x=$cropX; y=$cropY; width=$cropWidth; height=$cropHeight; windowFromPoint=('0x{0:X}' -f $pointWindow.ToInt64()) }
    $evidence.prerequisiteStatus = 'passed'

    $bitmap = [System.Drawing.Bitmap]::new($cropWidth, $cropHeight, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($cropX, $cropY, 0, 0, [System.Drawing.Size]::new($cropWidth, $cropHeight), [System.Drawing.CopyPixelOperation]::SourceCopy)
    $pngStream = [IO.File]::Open($pngPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $bitmap.Save($pngStream, [System.Drawing.Imaging.ImageFormat]::Png) } finally { $pngStream.Dispose() }

    $exactPixels = 0
    for ($pixelY = 0; $pixelY -lt $cropHeight; $pixelY++) {
        for ($pixelX = 0; $pixelX -lt $cropWidth; $pixelX++) {
            $pixel = $bitmap.GetPixel($pixelX, $pixelY)
            if ($pixel.R -eq $expectedRed -and $pixel.G -eq $expectedGreen -and $pixel.B -eq $expectedBlue) { $exactPixels++ }
        }
    }
    $evidence.exactColorPixelCount = $exactPixels
    if ($exactPixels -ge 20) {
        $evidence.ok = $true
        $evidence.status = 'ok'
    } else {
        $evidence.status = 'color_mismatch'
        $evidence.error = "Valid desktop crop contained $exactPixels exact #$('{0:X2}{1:X2}{2:X2}' -f $expectedRed,$expectedGreen,$expectedBlue) pixels; expected at least 20"
    }
} catch {
    if ($evidence.prerequisiteStatus -eq 'passed') { $evidence.status = 'capture_or_evidence_failure' }
    else { $evidence.status = 'prerequisite_failed' }
    $evidence.error = $_.Exception.GetType().Name + ': ' + $_.Exception.Message
} finally {
    if ($previousDpi -ne [IntPtr]::Zero -and [CaptionColorNative]::SetThreadDpiAwarenessContext($previousDpi) -eq [IntPtr]::Zero) {
        $evidence.ok = $false
        $evidence.status = 'capture_or_evidence_failure'
        $evidence.error = 'Could not restore the DPI awareness context'
    }
    if ($null -ne $graphics) { $graphics.Dispose() }
    if ($null -ne $bitmap) { $bitmap.Dispose() }
    try {
        $json = ConvertTo-Json -InputObject $evidence -Depth 10
        $jsonStream = [IO.File]::Open($jsonPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try {
            $writer = [IO.StreamWriter]::new($jsonStream, [Text.UTF8Encoding]::new($false))
            try { $writer.WriteLine($json) } finally { $writer.Dispose() }
        } finally { $jsonStream.Dispose() }
    } catch {
        [Console]::Error.WriteLine('Unable to create caption-color.json: ' + $_.Exception.Message)
        exit 1
    }
}

if ($evidence.ok) { exit 0 }
[Console]::Error.WriteLine($evidence.error)
exit 1
