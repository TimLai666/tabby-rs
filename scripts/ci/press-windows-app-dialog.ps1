<#
.SYNOPSIS
Inspect, or explicitly invoke, a uniquely identified button on an owned native modal.
.DESCRIPTION
Inspect writes dialog-inspect.json. Confirm rechecks the dialog, invokes the button once,
and writes dialog-confirm.json. Failures are returned in JSON when a fresh output file
could be reserved; an existing output file is never replaced. Invalid arguments, missing
or ambiguous dialogs, UI Automation errors, and timeouts fail without claiming success.
.PARAMETER AppProcessId
Required positive process ID of the application that owns the dialog.
.PARAMETER ExpectedTextToken
Required unique nonce of at least 16 characters, visible in the dialog's accessible text.
.PARAMETER ButtonText
Required exact accessible button name, such as Paste. A normal mnemonic ampersand is ignored.
.PARAMETER Action
Inspect (default) reports the dialog without acting. Confirm invokes the matching button once.
.PARAMETER OutputDirectory
Required existing absolute directory for the fresh JSON result file.
.EXAMPLE
pwsh -NoProfile -File press-windows-app-dialog.ps1 -AppProcessId 1234 -ExpectedTextToken unique-test-token-123 -ButtonText Paste -Action Inspect -OutputDirectory C:\owned\app
.EXAMPLE
pwsh -NoProfile -File press-windows-app-dialog.ps1 -AppProcessId 1234 -ExpectedTextToken unique-test-token-123 -ButtonText Paste -Action Confirm -OutputDirectory C:\owned\app
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][int]$AppProcessId,
    [Parameter(Mandatory=$true)][string]$ExpectedTextToken,
    [Parameter(Mandatory=$true)][string]$ButtonText,
    [ValidateSet('Inspect','Confirm')][string]$Action='Inspect',
    [Parameter(Mandatory=$true)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
$started = [DateTime]::UtcNow
$result = [ordered]@{ ok=$false; action=$Action; actualAction=$false; disappeared=$false; error=$null; timestampUtc=$started.ToString('o') }
$outputPath = $null
$outputStream = $null
$script:uiaDeadline = $null

function Get-DialogMatch {
    param([string]$ExpectedStart)
    $dialogMatches = @()
    foreach ($handle in [OwnedDialogPresence]::GetWindowsForProcess($AppProcessId)) {
        $window = [System.Windows.Automation.AutomationElement]::FromHandle($handle)
        if ($window.Current.ProcessId -ne $AppProcessId) { throw 'Native window ownership changed during discovery' }
        if ($window.Current.ClassName -cne '#32770') { continue }
        if ([DateTime]::UtcNow -gt $script:uiaDeadline) { throw 'UIA discovery exceeded 15 seconds' }
        if ($window.Current.IsOffscreen -or -not $window.Current.IsEnabled) { continue }
        $windowPattern = $null
        if (-not $window.TryGetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern, [ref]$windowPattern)) { continue }
        if (-not $windowPattern.Current.IsModal) { continue }
        $all = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
        if ($all.Count -gt 200) { throw 'Dialog has more than 200 accessible descendants' }
        $texts = @(); $buttons = @()
        foreach ($item in $all) {
            if ([DateTime]::UtcNow -gt $script:uiaDeadline) { throw 'UIA inspection exceeded 15 seconds' }
            if ($item.Current.IsOffscreen) { continue }
            $name = [string]$item.Current.Name
            if ($name) { $texts += $name }
            $valuePattern = $null
            if ($item.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$valuePattern) -and $valuePattern.Current.Value) { $texts += [string]$valuePattern.Current.Value }
            if ($item.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button -and $item.Current.IsEnabled -and $name -and (($name -replace '&','') -ceq ($ButtonText -replace '&',''))) {
                $invokePattern = $null
                if ($item.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$invokePattern)) { $buttons += ,@{ element=$item; pattern=$invokePattern; name=$name; automationId=$item.Current.AutomationId } }
            }
        }
        $visibleText = ($texts -join "`n")
        $nonceCount = [regex]::Matches($visibleText, [regex]::Escape($ExpectedTextToken)).Count
        if ($nonceCount -eq 0) { continue }
        if ($nonceCount -ne 1) { throw 'Expected nonce occurs more than once in a matching dialog' }
        if ($buttons.Count -ne 1) { throw "Expected exactly one visible enabled Invoke button named '$ButtonText'; found $($buttons.Count)" }
        $ownerPid = [int]$window.Current.ProcessId
        $proc = Get-Process -Id $ownerPid -ErrorAction Stop
        $creation = $proc.StartTime.ToUniversalTime().ToString('o')
        if ($ExpectedStart -and $creation -cne $ExpectedStart) { throw 'Owning process creation identity changed' }
        $rect = $window.Current.BoundingRectangle
        $nativeHandle = [Int64]$window.Current.NativeWindowHandle
        if ($nativeHandle -le 0) { throw 'Matching native dialog has no valid HWND' }
        $dialogMatches += ,@{ element=$window; button=$buttons[0]; pid=$ownerPid; creation=$creation; hwnd=$nativeHandle; title=[string]$window.Current.Name; bounds=@{x=$rect.X;y=$rect.Y;width=$rect.Width;height=$rect.Height}; text=$visibleText }
    }
    if ($dialogMatches.Count -gt 1) { throw "Expected one visible owned modal #32770 containing the nonce; found $($dialogMatches.Count)" }
    if ($dialogMatches.Count -eq 0) { return $null }
    return $dialogMatches[0]
}

try {
    if ($AppProcessId -le 0) { throw 'AppProcessId must be positive' }
    if ([string]::IsNullOrWhiteSpace($ExpectedTextToken) -or $ExpectedTextToken.Length -lt 16) { throw 'ExpectedTextToken must contain at least 16 characters' }
    if ([string]::IsNullOrWhiteSpace($ButtonText)) { throw 'ButtonText is required' }
    if (-not [IO.Path]::IsPathFullyQualified($OutputDirectory) -or -not (Test-Path -LiteralPath $OutputDirectory -PathType Container)) { throw 'OutputDirectory must be an existing absolute directory' }
    $OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
    $outputPath = Join-Path $OutputDirectory $(if ($Action -eq 'Inspect') { 'dialog-inspect.json' } else { 'dialog-confirm.json' })
    $outputStream = [IO.File]::Open($outputPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class OwnedDialogPresence {
    private delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);
    public static IntPtr[] GetWindowsForProcess(int expectedProcessId) {
        var windows = new System.Collections.Generic.List<IntPtr>();
        bool success = EnumWindows((window, parameter) => {
            uint processId;
            if (GetWindowThreadProcessId(window, out processId) != 0 && processId == expectedProcessId) windows.Add(window);
            return true;
        }, IntPtr.Zero);
        if (!success) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        return windows.ToArray();
    }
    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
}
'@
    $script:uiaDeadline = [DateTime]::UtcNow.AddSeconds(15)
    $proc = Get-Process -Id $AppProcessId -ErrorAction Stop
    $startIdentity = $proc.StartTime.ToUniversalTime().ToString('o')
    $dialog = $null
    do {
        $dialog = Get-DialogMatch -ExpectedStart $startIdentity
        if ($dialog) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $script:uiaDeadline)
    if (-not $dialog) { throw 'No matching owned modal appeared within 15 seconds' }
    $result.hwnd = $dialog.hwnd; $result.title = $dialog.title; $result.bounds = $dialog.bounds
    $result.processId = $dialog.pid; $result.processCreationUtc = $dialog.creation
    $result.nonce = $ExpectedTextToken; $result.button = @{name=$dialog.button.name;automationId=$dialog.button.automationId}
    $result.visibleAccessibleTextTruncated = $dialog.text.Length -gt 20000
    $result.visibleAccessibleText = $dialog.text.Substring(0, [Math]::Min(20000, $dialog.text.Length))
    if ($Action -eq 'Confirm') {
        $again = Get-DialogMatch -ExpectedStart $startIdentity
        if (-not $again) { throw 'Matching dialog disappeared before invocation' }
        if ($again.hwnd -ne $dialog.hwnd -or $again.button.name -cne $dialog.button.name) { throw 'Dialog identity changed before invocation' }
        $again.button.pattern.Invoke()
        $result.actualAction = $true
        $until = [DateTime]::UtcNow.AddSeconds(3)
        $present = $true
        do {
            Start-Sleep -Milliseconds 100
            $owner = Get-Process -Id $AppProcessId -ErrorAction Stop
            if ($owner.StartTime.ToUniversalTime().ToString('o') -cne $startIdentity) { throw 'Owning process creation identity changed after invocation' }
            $handle = [IntPtr]::new($dialog.hwnd)
            $present = [OwnedDialogPresence]::IsWindow($handle)
            if ($present) {
                [uint32]$ownerPid = 0
                $thread = [OwnedDialogPresence]::GetWindowThreadProcessId($handle, [ref]$ownerPid)
                if ($thread -eq 0 -or $ownerPid -ne $AppProcessId) { throw 'Original HWND ownership could not be confirmed after invocation' }
            }
        } while ($present -and [DateTime]::UtcNow -lt $until)
        $result.disappeared = -not $present
        if (-not $result.disappeared) { throw 'Original dialog HWND still exists after 3 seconds' }
    }
    $result.ok = $true
} catch { $result.error = $_.Exception.Message }

if (-not $outputPath) { Write-Error $result.error; exit 1 }
try {
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($result | ConvertTo-Json -Depth 8))
    $outputStream.Write($bytes,0,$bytes.Length)
} catch { Write-Error "Could not write result JSON: $($_.Exception.Message)"; exit 2 }
finally { if ($outputStream) { $outputStream.Dispose() } }
$result | ConvertTo-Json -Depth 8 -Compress
if (-not $result.ok) { exit 1 }
