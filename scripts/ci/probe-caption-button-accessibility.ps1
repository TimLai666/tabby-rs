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
    invokeCount = 0
}

try {
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes

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
        return @{ element=$element; pattern=$pattern }
    }

    $validated = Get-ValidatedButtonElement
    # Re-fetch and validate the exact input HWND immediately before the sole Invoke.
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
