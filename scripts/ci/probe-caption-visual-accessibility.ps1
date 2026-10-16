param(
    [Parameter(Mandatory=$true)][int]$AppProcessId,
    [Parameter(Mandatory=$true)][long]$ParentHandle,
    [Parameter(Mandatory=$true)][long]$ButtonHandle,
    [Parameter(Mandatory=$true)][string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$evidence = [ordered]@{
    success = $false
    error = $null
    identity = [ordered]@{
        appProcessId = $AppProcessId
        parentHandle = ('0x{0:X}' -f $ParentHandle)
        buttonHandle = ('0x{0:X}' -f $ButtonHandle)
        parentProcessId = $null
        buttonProcessId = $null
        parentThreadId = $null
        buttonThreadId = $null
        buttonParentHandle = $null
        controlType = $null
        name = $null
        uiAutomationProcessId = $null
        nativeWindowHandle = $null
        enabled = $null
        offscreen = $null
    }
    bounds = $null
    points = [Collections.Generic.List[object]]::new()
    descendants = [ordered]@{ raw = @(); control = @(); content = @() }
    invokeCount = 0
}

try {
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    Add-Type -AssemblyName WindowsBase

    $providerAssembly = [Reflection.Assembly]::Load('UIAutomationClientSideProviders')
    Add-Type -ReferencedAssemblies @(
        [System.Windows.Automation.AutomationElement].Assembly.Location,
        [System.Windows.Automation.ControlType].Assembly.Location,
        $providerAssembly.Location
    ) -TypeDefinition @'
public static class CaptionButtonAutomationProviders {
    public static void Register() {
        System.Windows.Automation.ClientSettings.RegisterClientSideProviders(
            UIAutomationClientsideProviders.UIAutomationClientSideProviders.ClientSideProviderDescriptionTable);
    }
}
'@
    [CaptionButtonAutomationProviders]::Register()

    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class CaptionButtonNativeChecks {
    [DllImport("user32.dll", SetLastError=true)] public static extern bool IsWindow(IntPtr hWnd);
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr GetParent(IntPtr hWnd);
    [DllImport("user32.dll", SetLastError=true)] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
}
'@

    function Assert-ExactNativeOwnership {
        if ($ParentHandle -eq 0 -or $ButtonHandle -eq 0 -or $ParentHandle -eq $ButtonHandle) {
            throw 'Input HWNDs must be distinct nonzero handles'
        }
        $parent = [IntPtr]$ParentHandle
        $button = [IntPtr]$ButtonHandle
        if (-not [CaptionButtonNativeChecks]::IsWindow($parent)) { throw 'ParentHandle is not a live window' }
        if (-not [CaptionButtonNativeChecks]::IsWindow($button)) { throw 'ButtonHandle is not a live window' }
        [uint32]$parentPid = 0
        [uint32]$buttonPid = 0
        $parentTid = [CaptionButtonNativeChecks]::GetWindowThreadProcessId($parent, [ref]$parentPid)
        $buttonTid = [CaptionButtonNativeChecks]::GetWindowThreadProcessId($button, [ref]$buttonPid)
        $actualParent = [CaptionButtonNativeChecks]::GetParent($button)
        $evidence.identity.parentProcessId = [int]$parentPid
        $evidence.identity.buttonProcessId = [int]$buttonPid
        $evidence.identity.parentThreadId = [uint32]$parentTid
        $evidence.identity.buttonThreadId = [uint32]$buttonTid
        $evidence.identity.buttonParentHandle = ('0x{0:X}' -f $actualParent.ToInt64())
        if ($parentPid -ne $AppProcessId -or $buttonPid -ne $AppProcessId) { throw 'Native HWND process ownership mismatch' }
        if ($parentTid -eq 0 -or $parentTid -ne $buttonTid) { throw 'Native HWND UI thread mismatch' }
        if ($actualParent -ne $parent) { throw 'ButtonHandle is not a direct child of ParentHandle' }
    }

    function Get-ValidatedButtonElement {
        Assert-ExactNativeOwnership
        $element = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$ButtonHandle)
        if ($null -eq $element) { throw 'UIA returned no element for ButtonHandle' }
        $current = $element.Current
        $type = [string]$current.ControlType.ProgrammaticName
        $name = [string]$current.Name
        $elementProcessId = [int]$current.ProcessId
        $native = [long]$current.NativeWindowHandle
        $enabled = [bool]$current.IsEnabled
        $offscreen = [bool]$current.IsOffscreen
        $rect = $current.BoundingRectangle
        $evidence.identity.controlType = $type
        $evidence.identity.name = $name
        $evidence.identity.uiAutomationProcessId = $elementProcessId
        $evidence.identity.nativeWindowHandle = ('0x{0:X}' -f $native)
        $evidence.identity.enabled = $enabled
        $evidence.identity.offscreen = $offscreen
        $evidence.bounds = [ordered]@{ x=[double]$rect.X; y=[double]$rect.Y; width=[double]$rect.Width; height=[double]$rect.Height }
        if ($type -cne 'ControlType.Button') { throw 'UIA control type is not Button' }
        if ($name -cne 'Maximize') { throw 'UIA name is not Maximize' }
        if ($elementProcessId -ne $AppProcessId) { throw 'UIA process identity mismatch' }
        if ($native -ne $ButtonHandle) { throw 'UIA native window handle mismatch' }
        if (-not $enabled) { throw 'UIA button is disabled' }
        if ($offscreen) { throw 'UIA button is offscreen' }
        if ([double]::IsNaN($rect.X) -or [double]::IsNaN($rect.Y) -or
            [double]::IsNaN($rect.Width) -or [double]::IsNaN($rect.Height) -or
            [double]::IsInfinity($rect.X) -or [double]::IsInfinity($rect.Y) -or
            [double]::IsInfinity($rect.Width) -or [double]::IsInfinity($rect.Height) -or
            $rect.Width -le 0 -or $rect.Height -le 0) { throw 'UIA button bounds are empty or invalid' }
        $pattern = $null
        if (-not $element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern) -or $null -eq $pattern) {
            throw 'UIA InvokePattern is unavailable'
        }
        return @{ element=$element; pattern=$pattern; bounds=$rect }
    }

    function Assert-ElementAtExactButtonPoint {
        param([string]$phase, [string]$pointName, [double]$x, [double]$y)
        $pointElement = [System.Windows.Automation.AutomationElement]::FromPoint([System.Windows.Point]::new($x, $y))
        if ($null -eq $pointElement) { throw "UIA ElementFromPoint returned no element at $pointName ($phase)" }
        $pointData = $pointElement.Current
        $pointType = [string]$pointData.ControlType.ProgrammaticName
        $pointNameValue = [string]$pointData.Name
        $pointProcessId = [int]$pointData.ProcessId
        $pointNativeHandle = [long]$pointData.NativeWindowHandle
        $record = [ordered]@{
            phase = $phase
            point = $pointName
            x = $x
            y = $y
            processId = $pointProcessId
            nativeWindowHandle = ('0x{0:X}' -f $pointNativeHandle)
            controlType = $pointType
            name = $pointNameValue
            isControlElement = [bool]$pointData.IsControlElement
            isContentElement = [bool]$pointData.IsContentElement
            controlNormalized = $null
            contentNormalized = $null
        }
        $evidence.points.Add($record)
        foreach ($normalView in @('control', 'content')) {
            $walker = if ($normalView -eq 'control') { [System.Windows.Automation.TreeWalker]::ControlViewWalker } else { [System.Windows.Automation.TreeWalker]::ContentViewWalker }
            $normalized = $walker.Normalize($pointElement).Current
            $record[$normalView + 'Normalized'] = @{
                hwnd = ('0x{0:X}' -f [long]$normalized.NativeWindowHandle)
                processId = [int]$normalized.ProcessId
                type = [string]$normalized.ControlType.ProgrammaticName
                name = [string]$normalized.Name
                isControlElement = [bool]$normalized.IsControlElement
                isContentElement = [bool]$normalized.IsContentElement
            }
        }
        if ($pointNativeHandle -ne $ButtonHandle -or $pointProcessId -ne $AppProcessId -or
            $pointType -cne 'ControlType.Button' -or $pointNameValue -cne 'Maximize') {
            # Preserve raw/control/content evidence before the same strict failure.
            $exactButton = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$ButtonHandle)
            Assert-ButtonDescendantViews -rootElement $exactButton -phase 'point-failure-diagnostic'
            throw "UIA ElementFromPoint did not return the exact Maximize ButtonHandle at $pointName ($phase)"
        }
    }

    function Add-BoundedChildren {
        param(
            [System.Windows.Automation.TreeWalker]$walker,
            [System.Windows.Automation.AutomationElement]$parentElement,
            [int]$parentDepth,
            [Collections.Generic.Stack[object]]$pending,
            [Collections.Generic.List[object]]$records,
            [string]$view
        )
        $child = $walker.GetFirstChild($parentElement)
        if ($parentDepth -ge 4) {
            if ($null -ne $child) { throw "$view descendant depth exceeds 4" }
            return
        }
        $children = [Collections.Generic.List[object]]::new()
        while ($null -ne $child) {
            if (($records.Count + $pending.Count + $children.Count + 1) -gt 16) {
                throw "$view descendant limit exceeded (16 nodes)"
            }
            $children.Add($child)
            $child = $walker.GetNextSibling($child)
        }
        for ($childIndex = $children.Count - 1; $childIndex -ge 0; $childIndex--) {
            $pending.Push(@{ element=$children[$childIndex]; depth=$parentDepth + 1 })
        }
    }

    function Get-BoundedDescendants {
        param([System.Windows.Automation.TreeWalker]$walker, [System.Windows.Automation.AutomationElement]$rootElement, [string]$view)
        $records = [Collections.Generic.List[object]]::new()
        $pending = [Collections.Generic.Stack[object]]::new()
        Add-BoundedChildren -walker $walker -parentElement $rootElement -parentDepth 0 -pending $pending -records $records -view $view
        while ($pending.Count -gt 0) {
            if ($records.Count -ge 16) { throw "$view descendant limit exceeded (16 nodes)" }
            $entry = $pending.Pop()
            $node = [System.Windows.Automation.AutomationElement]$entry.element
            $depth = [int]$entry.depth
            $nodeData = $node.Current
            $nodeProcessId = [int]$nodeData.ProcessId
            $record = [ordered]@{
                name = [string]$nodeData.Name
                class = [string]$nodeData.ClassName
                type = [string]$nodeData.ControlType.ProgrammaticName
                hwnd = ('0x{0:X}' -f [long]$nodeData.NativeWindowHandle)
                processId = $nodeProcessId
                offscreen = [bool]$nodeData.IsOffscreen
                isControlElement = [bool]$nodeData.IsControlElement
                isContentElement = [bool]$nodeData.IsContentElement
                depth = $depth
            }
            $records.Add($record)
            if ($nodeProcessId -ne $AppProcessId) { throw "$view descendant belongs to another process" }
            Add-BoundedChildren -walker $walker -parentElement $node -parentDepth $depth -pending $pending -records $records -view $view
        }
        return $records.ToArray()
    }

    function Assert-ButtonPointHits {
        param([string]$phase, [System.Windows.Rect]$rect)
        $pointSpecs = @(
            @{ name='center'; x=($rect.X + ($rect.Width / 2.0)); y=($rect.Y + ($rect.Height / 2.0)) },
            @{ name='glyph-outline'; x=($rect.X + ($rect.Width / 2.0) - 5.0); y=($rect.Y + ($rect.Height / 2.0) - 5.0) },
            @{ name='hover-background'; x=($rect.X + 3.0); y=($rect.Y + 3.0) }
        )
        foreach ($pointSpec in $pointSpecs) {
            Assert-ElementAtExactButtonPoint -phase $phase -pointName $pointSpec.name -x $pointSpec.x -y $pointSpec.y
        }
    }

    function Assert-ButtonDescendantViews {
        param([System.Windows.Automation.AutomationElement]$rootElement, [string]$phase)
        $evidence.descendants.raw = @(Get-BoundedDescendants -walker ([System.Windows.Automation.TreeWalker]::RawViewWalker) -rootElement $rootElement -view 'raw')
        $evidence.descendants.control = @(Get-BoundedDescendants -walker ([System.Windows.Automation.TreeWalker]::ControlViewWalker) -rootElement $rootElement -view 'control')
        $evidence.descendants.content = @(Get-BoundedDescendants -walker ([System.Windows.Automation.TreeWalker]::ContentViewWalker) -rootElement $rootElement -view 'content')
        if ($evidence.descendants.control.Count -ne 0 -or $evidence.descendants.content.Count -ne 0) {
            throw "$phase UIA ControlView/ContentView exposes extra descendants"
        }
    }

    $validated = Get-ValidatedButtonElement
    Assert-ButtonPointHits -phase 'initial' -rect $validated.bounds
    Assert-ButtonDescendantViews -rootElement $validated.element -phase 'initial'
    # Re-fetch and validate the exact input HWND immediately before the sole Invoke.
    $validated = Get-ValidatedButtonElement
    Assert-ButtonPointHits -phase 'preInvoke' -rect $validated.bounds
    Assert-ButtonDescendantViews -rootElement $validated.element -phase 'preInvoke'
    $validated = Get-ValidatedButtonElement
    $evidence.invokeCount = 1
    $validated.pattern.Invoke()
    $evidence.success = $true
} catch {
    $evidence.error = $_.Exception.GetType().Name + ': ' + $_.Exception.Message
} finally {
    try {
        $json = ConvertTo-Json -InputObject $evidence -Depth 8
        $stream = [IO.File]::Open($OutputPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try {
            $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
            try { $writer.WriteLine($json) } finally { $writer.Dispose() }
        } finally {
            if ($null -ne $stream) { $stream.Dispose() }
        }
    } catch {
        [Console]::Error.WriteLine('Unable to create evidence file: ' + $_.Exception.Message)
        exit 1
    }
}

if ($evidence.success) { exit 0 }
[Console]::Error.WriteLine($evidence.error)
exit 1
