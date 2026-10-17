<#
.SYNOPSIS
Tests the real physical pointer coordinates and right-edge resize behavior of the supplied Windows input helper.
.DESCRIPTION
Creates one independent, owned WinForms worker with an Application.Run message pump, records its exact CIM process identity and HWND, and loads the unchanged ExistingWindowInput C# class from HelperPath. The fixture runs the real Snapshot action, reflects only SendMove, ReadCursor, and SetThreadDpiAwarenessContext to sample 24 visible client points without LEFTDOWN, then runs the real Resize action with DeltaX -60 and DeltaY 0. Child PowerShell consoles start hidden while the owned WinForms window remains visible. Pointer samples and resize evidence are recorded independently in result.json. A failed sample does not skip the resize control group. If an owned mouse journal remains, recovery uses the helper's RecoverMouse action only after the exact helper child has exited. Finally the fixture closes and reaps only its own WinForms worker after checking PID and creation identity.
.PARAMETER HelperPath
Absolute path to the unchanged exercise-windows-window-input.ps1 helper that contains the ExistingWindowInput C# source.
.PARAMETER OutputDirectory
Fresh absolute directory for result.json and the fixture's private worker/action evidence files. The directory must not already exist.
.EXAMPLE
pwsh -NoProfile -File scripts/ci/test-windows-pointer-coordinates.ps1 -HelperPath scripts/ci/exercise-windows-window-input.ps1 -OutputDirectory C:\qa\pointer-control-001
.EXAMPLE
Get-Help .\scripts\ci\test-windows-pointer-coordinates.ps1 -Full
.NOTES
Windows PowerShell 7+ on an interactive Windows desktop is required. The fixture never launches either application, changes an unrelated window, changes display settings, or sends LEFTDOWN during pointer sampling. Exit code 0 means every required sample, resize, and cleanup check passed; exit code 1 means at least one check failed.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$HelperPath,
    [Parameter(Mandatory = $true)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'

function Add-FixtureError {
    param([Collections.Generic.List[object]]$Errors, [string]$Stage, [string]$Message)
    $Errors.Add([ordered]@{ stage = $Stage; message = $Message })
}

function Get-ExactProcessIdentity {
    param([Parameter(Mandatory = $true)][int]$ProcessId)
    $row = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = $ProcessId" -ErrorAction Stop
    if ($null -eq $row) { return $null }
    return [ordered]@{
        pid = [int]$row.ProcessId
        parentPid = [int]$row.ParentProcessId
        creationTimeUTC = $row.CreationDate.ToUniversalTime().ToString('o')
    }
}

function Test-SameProcessIdentity {
    param($Expected, $Actual)
    return $null -ne $Expected -and $null -ne $Actual -and
        [int]$Expected.pid -eq [int]$Actual.pid -and
        [string]$Expected.creationTimeUTC -ceq [string]$Actual.creationTimeUTC
}

function Wait-ExactProcessIdentity {
    param([Parameter(Mandatory = $true)][int]$ProcessId, [Parameter(Mandatory = $true)][datetime]$DeadlineUtc)
    do {
        $identity = Get-ExactProcessIdentity -ProcessId $ProcessId
        if ($null -ne $identity) { return $identity }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $DeadlineUtc)
    return $null
}

function Wait-ExactProcessGone {
    param($Expected, [Parameter(Mandatory = $true)][datetime]$DeadlineUtc)
    if ($null -eq $Expected) { return $false }
    do {
        $actual = Get-ExactProcessIdentity -ProcessId ([int]$Expected.pid)
        if (-not (Test-SameProcessIdentity -Expected $Expected -Actual $actual)) { return $true }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $DeadlineUtc)
    return $false
}

function Quote-ProcessArgument {
    param([Parameter(Mandatory = $true)][string]$Value)
    if ($Value.Contains('"')) { throw 'Windows paths and fixture arguments cannot contain a double quote' }
    return '"' + $Value + '"'
}

function Join-ProcessArguments {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    return [string]::Join(' ', $Arguments)
}

function Wait-ChildExit {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][int]$TimeoutMilliseconds
    )
    try { return $Process.WaitForExit($TimeoutMilliseconds) } catch { return $false }
}

function Stop-ExactOwnedChild {
    param(
        [Diagnostics.Process]$Process,
        $Identity,
        [Parameter(Mandatory = $true)][string]$Label,
        [Collections.Generic.List[object]]$Errors
    )
    if ($null -eq $Process -or $null -eq $Identity) { return $false }
    try {
        $actual = Get-ExactProcessIdentity -ProcessId ([int]$Identity.pid)
        if (Test-SameProcessIdentity -Expected $Identity -Actual $actual) {
            $Process.Refresh()
            if (-not $Process.HasExited) {
                $Process.Kill()
                if (-not $Process.WaitForExit(10000)) {
                    Add-FixtureError $Errors "$Label-cleanup" 'Exact owned child did not exit after its bounded cleanup termination.'
                    return $false
                }
            }
        }
        return Wait-ExactProcessGone -Expected $Identity -DeadlineUtc ([DateTime]::UtcNow.AddSeconds(8))
    } catch {
        Add-FixtureError $Errors "$Label-cleanup" $_.Exception.Message
        return $false
    }
}

function Complete-OwnedChild {
    param(
        [Diagnostics.Process]$Process,
        $Identity,
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][int]$TimeoutMilliseconds,
        [Collections.Generic.List[object]]$Errors
    )
    if ($null -eq $Process) { return [ordered]@{ identity = $Identity; reaped = $false; exactIdentityGone = $false } }
    if ($null -eq $Identity) {
        $Identity = Wait-ExactProcessIdentity -ProcessId $Process.Id -DeadlineUtc ([DateTime]::UtcNow.AddSeconds(5))
    }
    if ($null -eq $Identity) {
        $exited = Wait-ChildExit -Process $Process -TimeoutMilliseconds $TimeoutMilliseconds
        Add-FixtureError $Errors "$Label-cleanup" 'Could not capture exact PID/creation identity; waited on the held process handle and did not terminate an unverified process.'
        return [ordered]@{ identity = $null; reaped = $exited; exactIdentityGone = $false }
    }
    if ([int]$Identity.parentPid -ne $PID) {
        $exited = Wait-ChildExit -Process $Process -TimeoutMilliseconds $TimeoutMilliseconds
        Add-FixtureError $Errors "$Label-cleanup" 'Child is not a direct fixture child; waited on its held process handle and did not terminate it.'
        return [ordered]@{ identity = $Identity; reaped = $exited; exactIdentityGone = $false }
    }
    $exited = Wait-ChildExit -Process $Process -TimeoutMilliseconds $TimeoutMilliseconds
    if (-not $exited) {
        Add-FixtureError $Errors "$Label-timeout" 'Owned child exceeded its bounded completion deadline.'
        $null = Stop-ExactOwnedChild -Process $Process -Identity $Identity -Label $Label -Errors $Errors
    }
    try { $Process.Refresh() } catch { Add-FixtureError $Errors "$Label-cleanup" $_.Exception.Message }
    $reaped = $false
    try { $reaped = $Process.HasExited } catch { Add-FixtureError $Errors "$Label-cleanup" $_.Exception.Message }
    $gone = Wait-ExactProcessGone -Expected $Identity -DeadlineUtc ([DateTime]::UtcNow.AddSeconds(8))
    return [ordered]@{ identity = $Identity; reaped = $reaped; exactIdentityGone = $gone }
}

function Get-OwnedMouseJournalRecord {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][int]$OwnerProcessId,
        [Parameter(Mandatory = $true)][long]$WindowHandle,
        [Parameter(Mandatory = $true)][int]$HelperProcessId
    )
    $lines = [IO.File]::ReadAllLines($Path, [Text.UTF8Encoding]::new($false, $true))
    $owner = 0; $hwnd = 0L; $helper = 0
    $valid = $lines.Length -eq 3 -and
        [int]::TryParse($lines[0], [ref]$owner) -and
        [long]::TryParse($lines[1], [ref]$hwnd) -and
        [int]::TryParse($lines[2], [ref]$helper) -and
        $owner -eq $OwnerProcessId -and $hwnd -eq $WindowHandle -and $helper -eq $HelperProcessId
    return [ordered]@{ path = $Path; present = $true; validOwnedJournal = $valid; ownerPid = $owner; hwnd = $hwnd; helperPid = $helper }
}

function Write-FreshJson {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)]$Value)
    $json = ConvertTo-Json -InputObject $Value -Depth 16
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
        $writer.WriteLine($json)
        $writer.Flush()
        $stream.Flush($true)
        $writer.Dispose()
    } finally {
        $stream.Dispose()
    }
}

$result = [ordered]@{
    ok = $false
    identity = $null
    snapshot = $null
    samples = $null
    resize = $null
    cleanup = [ordered]@{
        workerStopRequested = $false
        workerReaped = $false
        workerExactIdentityGone = $false
        actionReaped = $false
        actionExactIdentityGone = $false
        mouseJournal = $null
        mouseRecovery = $null
        recoveryReaped = $null
        recoveryExactIdentityGone = $null
        passed = $false
    }
    errors = [Collections.Generic.List[object]]::new()
}

$outputCreated = $false
$worker = $null
$workerIdentity = $null
$actionProcess = $null
$actionIdentity = $null
$recoveryProcess = $null
$recoveryIdentity = $null
$workerReadyPath = $null
$workerStopPath = $null
$actionResultPath = $null
$mouseJournalPath = $null
$workerScriptPath = $null
$actionScriptPath = $null
$recoveryResultPath = $null
$pwshPath = $null
$workerStopToken = [Guid]::NewGuid().ToString('N')

try {
    if (-not $IsWindows) { throw 'This fixture requires Windows PowerShell 7+ on an interactive Windows desktop.' }
    if (-not [IO.Path]::IsPathRooted($HelperPath)) { throw 'HelperPath must be absolute.' }
    if (-not [IO.Path]::IsPathRooted($OutputDirectory)) { throw 'OutputDirectory must be absolute.' }
    $HelperPath = [IO.Path]::GetFullPath($HelperPath)
    $OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
    if (-not (Test-Path -LiteralPath $HelperPath -PathType Leaf)) { throw "HelperPath does not exist: $HelperPath" }
    if (Test-Path -LiteralPath $OutputDirectory) { throw 'OutputDirectory must be fresh and must not already exist.' }
    [IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null
    $outputCreated = $true

    $pwshPath = (Get-Process -Id $PID -ErrorAction Stop).Path
    if ([string]::IsNullOrWhiteSpace($pwshPath) -or -not (Test-Path -LiteralPath $pwshPath -PathType Leaf)) {
        throw 'Could not resolve the current PowerShell executable for owned fixture children.'
    }

    $workerReadyPath = Join-Path $OutputDirectory 'worker-ready.json'
    $workerStopPath = Join-Path $OutputDirectory 'worker-stop.txt'
    $actionResultPath = Join-Path $OutputDirectory 'action-result.json'
    $mouseJournalPath = Join-Path $OutputDirectory 'resize-action.json.mouse-down'
    $workerScriptPath = Join-Path $OutputDirectory 'owned-winforms-worker.ps1'
    $actionScriptPath = Join-Path $OutputDirectory 'pointer-and-resize-actions.ps1'
    $recoveryResultPath = Join-Path $OutputDirectory 'mouse-recovery.json'

    $workerScript = @'
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$ReadyPath, [Parameter(Mandatory=$true)][string]$StopPath)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$foregroundSource = @"
using System;
using System.Runtime.InteropServices;
public static class OwnedPointerFixtureForeground {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
}
"@
Add-Type -TypeDefinition $foregroundSource -Language CSharp

$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$clientWidth = [Math]::Min(640, $screen.Width - 80)
$clientHeight = [Math]::Min(420, $screen.Height - 140)
if ($clientWidth -lt 360 -or $clientHeight -lt 240) { throw 'Primary screen is too small for the bounded visible-client fixture.' }
$form = [System.Windows.Forms.Form]::new()
$form.Text = 'Owned pointer coordinate fixture'
$form.StartPosition = [System.Windows.Forms.FormStartPosition]::Manual
$form.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::Sizable
$form.ClientSize = [System.Drawing.Size]::new($clientWidth, $clientHeight)
$form.ShowInTaskbar = $true
$form.Location = [System.Drawing.Point]::new($screen.Left + [Math]::Max(0, ($screen.Width - $form.Width) / 2), $screen.Top + [Math]::Max(0, ($screen.Height - $form.Height) / 2))
$label = [System.Windows.Forms.Label]::new()
$label.Text = 'Owned WinForms pointer fixture'
$label.Dock = [System.Windows.Forms.DockStyle]::Fill
$label.TextAlign = [System.Drawing.ContentAlignment]::MiddleCenter
$form.Controls.Add($label)

$form.Add_Shown({
    $form.Activate()
    $form.BringToFront()
    [OwnedPointerFixtureForeground]::SetForegroundWindow($form.Handle) | Out-Null
    $ready = [ordered]@{ pid = $PID; hwnd = $form.Handle.ToInt64() } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText($ReadyPath, $ready, [Text.UTF8Encoding]::new($false))
})
$timer = [System.Windows.Forms.Timer]::new()
$timer.Interval = 75
$timer.Add_Tick({
    if (Test-Path -LiteralPath $StopPath -PathType Leaf) {
        $token = [IO.File]::ReadAllText($StopPath).Trim()
        if ($token.Length -gt 0) { $timer.Stop(); $form.Close() }
    }
})
$timer.Start()
try { [System.Windows.Forms.Application]::Run($form) }
finally { $timer.Dispose(); $form.Dispose() }
'@

    $actionScript = @'
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$HelperPath,
    [Parameter(Mandatory=$true)][int]$OwnerProcessId,
    [Parameter(Mandatory=$true)][string]$OwnerCreationUtc,
    [Parameter(Mandatory=$true)][long]$WindowHandle,
    [Parameter(Mandatory=$true)][string]$JournalPath,
    [Parameter(Mandatory=$true)][string]$ResultPath
)
$ErrorActionPreference = 'Stop'
$errors = [Collections.Generic.List[object]]::new()
$sampleRows = [Collections.Generic.List[object]]::new()
$actionResult = [ordered]@{
    ok = $false
    identity = [ordered]@{ ownerPid = $OwnerProcessId; ownerCreationTimeUTC = $OwnerCreationUtc; hwnd = $WindowHandle; actionPid = $PID }
    snapshot = $null
    samples = [ordered]@{ requiredCount = 24; points = @(); virtualDesktop = $null; beyondMidpointRequired = $false; beyondMidpointCount = 0; passed = $false }
    resize = $null
    mouseJournalPath = $JournalPath
    errors = @()
}
function Add-ActionError([string]$Stage, [string]$Message) { $errors.Add([ordered]@{ stage = $Stage; message = $Message }) }
function Get-InvocationError([Exception]$ErrorObject) {
    if ($null -ne $ErrorObject.InnerException) { return $ErrorObject.InnerException.Message }
    return $ErrorObject.Message
}
try {
    Add-Type -AssemblyName System.Windows.Forms
    $helperSourceText = [IO.File]::ReadAllText($HelperPath)
    $sourceMatches = [regex]::Matches($helperSourceText, '(?ms)^\$script = @''\r?\n(?<source>.*?)^''@\s*$')
    if ($sourceMatches.Count -ne 1) { throw "Expected one original ExistingWindowInput C# here-string; found $($sourceMatches.Count)." }
    $helperCSharp = $sourceMatches[0].Groups['source'].Value
    Add-Type -TypeDefinition $helperCSharp -Language CSharp -ErrorAction Stop

    $snapshot = [ExistingWindowInput]::Run($OwnerProcessId, $OwnerCreationUtc, $WindowHandle, 'Snapshot', 0, 0, 60, 0, $JournalPath)
    $actionResult.snapshot = $snapshot
    if (-not $snapshot['ok']) { throw "Actual Run(Snapshot) failed: $($snapshot['error'])" }
    $client = $snapshot['after']['client']
    if ($null -eq $client -or [int]$client['width'] -lt 6 -or [int]$client['height'] -lt 4) {
        throw 'Actual Run(Snapshot) did not provide a usable physical client rectangle.'
    }

    $bindingFlags = [Reflection.BindingFlags]::NonPublic -bor [Reflection.BindingFlags]::Static
    $sendMove = [ExistingWindowInput].GetMethod('SendMove', $bindingFlags)
    $readCursor = [ExistingWindowInput].GetMethod('ReadCursor', $bindingFlags)
    $setDpiContext = [ExistingWindowInput].GetMethod('SetThreadDpiAwarenessContext', $bindingFlags)
    if ($null -eq $sendMove -or $null -eq $readCursor -or $null -eq $setDpiContext) {
        throw 'The original helper is missing a required private input method.'
    }

    $previousDpiContext = [IntPtr]::Zero
    try {
        $previousDpiContext = [IntPtr]$setDpiContext.Invoke($null, @([IntPtr](-4)))
        if ($previousDpiContext -eq [IntPtr]::Zero) { throw 'Could not set per-monitor-v2 thread DPI awareness for coordinate sampling.' }
        $virtual = [System.Windows.Forms.SystemInformation]::VirtualScreen
        $virtualWidth = [int]$virtual.Width
        $virtualHeight = [int]$virtual.Height
        $midpointX = [int]$virtual.Left + [int][Math]::Floor($virtualWidth / 2.0)
        $actionResult.samples.virtualDesktop = [ordered]@{ left = $virtual.Left; top = $virtual.Top; width = $virtualWidth; height = $virtualHeight; midpointX = $midpointX }
        $actionResult.samples.beyondMidpointRequired = $virtualWidth -eq 1024 -and $virtualHeight -eq 768

        # This is only a point-selection grid inside the real client. The fixture never duplicates SendMove's input-coordinate conversion.
        for ($row = 0; $row -lt 4; $row++) {
            for ($column = 0; $column -lt 6; $column++) {
                $x = [int]$client['screenX'] + [int][Math]::Floor(($column + 0.5) * [int]$client['width'] / 6.0)
                $y = [int]$client['screenY'] + [int][Math]::Floor(($row + 0.5) * [int]$client['height'] / 4.0)
                $sample = [ordered]@{ index = $sampleRows.Count; requestedPhysical = [ordered]@{ x = $x; y = $y }; observedPhysical = $null; passed = $false; error = $null }
                try {
                    $null = $sendMove.Invoke($null, @([int]$x, [int]$y))
                    $observed = $readCursor.Invoke($null, @())
                    $sample.observedPhysical = [ordered]@{ x = [int]$observed.x; y = [int]$observed.y }
                    $sample.passed = $sample.observedPhysical.x -eq $x -and $sample.observedPhysical.y -eq $y
                    if (-not $sample.passed) { $sample.error = 'actual GetCursorPos differs from requested physical client point' }
                } catch {
                    $sample.error = Get-InvocationError $_.Exception
                    try {
                        $observed = $readCursor.Invoke($null, @())
                        $sample.observedPhysical = [ordered]@{ x = [int]$observed.x; y = [int]$observed.y }
                    } catch { $sample.error += '; ReadCursor failed: ' + (Get-InvocationError $_.Exception) }
                }
                $sampleRows.Add($sample)
                if ($sample.requestedPhysical.x -gt $midpointX) { $actionResult.samples.beyondMidpointCount++ }
                if (-not $sample.passed) { Add-ActionError "sample-$($sample.index)" ([string]$sample.error) }
            }
        }
    } catch {
        Add-ActionError 'pointer-sampling' (Get-InvocationError $_.Exception)
    } finally {
        if ($previousDpiContext -ne [IntPtr]::Zero) {
            try {
                $restored = [IntPtr]$setDpiContext.Invoke($null, @($previousDpiContext))
                if ($restored -eq [IntPtr]::Zero) { Add-ActionError 'dpi-restore' 'Could not restore the pre-sample thread DPI context.' }
            } catch { Add-ActionError 'dpi-restore' (Get-InvocationError $_.Exception) }
        }
    }

    $actionResult.samples.points = $sampleRows.ToArray()
    $actionResult.samples.passed = $sampleRows.Count -eq 24 -and @($sampleRows | Where-Object { -not $_.passed }).Count -eq 0 -and
        (-not $actionResult.samples.beyondMidpointRequired -or $actionResult.samples.beyondMidpointCount -gt 0)

    if (Test-Path -LiteralPath $JournalPath) {
        $actionResult.resize = [ordered]@{ attempted = $false; requestedDelta = [ordered]@{ x = -60; y = 0 }; passed = $false; error = 'owned mouse journal exists before Resize; stopped before further mouse input' }
        Add-ActionError 'resize' $actionResult.resize.error
    } else {
        $nativeResize = $null
        try {
            $nativeResize = [ExistingWindowInput]::Run($OwnerProcessId, $OwnerCreationUtc, $WindowHandle, 'Resize', 0, 0, -60, 0, $JournalPath)
            $beforeOuter = if ($null -ne $nativeResize['before']) { $nativeResize['before']['outer'] } else { $null }
            $afterOuter = if ($null -ne $nativeResize['after']) { $nativeResize['after']['outer'] } else { $null }
            $geometryPassed = $false
            if ($null -ne $beforeOuter -and $null -ne $afterOuter) {
                $geometryPassed = [int]$afterOuter['left'] -eq [int]$beforeOuter['left'] -and
                    [int]$afterOuter['top'] -eq [int]$beforeOuter['top'] -and
                    [int]$afterOuter['bottom'] -eq [int]$beforeOuter['bottom'] -and
                    [Math]::Abs(([int]$afterOuter['right'] - [int]$beforeOuter['right']) - (-60)) -le 3
            }
            $resizePassed = $nativeResize['ok'] -eq $true -and $geometryPassed
            $actionResult.resize = [ordered]@{
                attempted = $true
                requestedDelta = [ordered]@{ x = -60; y = 0 }
                run = $nativeResize
                beforeOuter = $beforeOuter
                afterOuter = $afterOuter
                geometryPassed = $geometryPassed
                passed = $resizePassed
                error = $nativeResize['error']
            }
            if (-not $resizePassed) { Add-ActionError 'resize' ([string]$nativeResize['error']) }
        } catch {
            $actionResult.resize = [ordered]@{ attempted = $true; requestedDelta = [ordered]@{ x = -60; y = 0 }; run = $nativeResize; passed = $false; error = Get-InvocationError $_.Exception }
            Add-ActionError 'resize' ([string]$actionResult.resize.error)
        }
    }
} catch {
    Add-ActionError 'action-fixture' (Get-InvocationError $_.Exception)
} finally {
    $actionResult.samples.points = $sampleRows.ToArray()
    $actionResult.errors = $errors.ToArray()
    $resizePassed = $null -ne $actionResult.resize -and $actionResult.resize.passed -eq $true
    $actionResult.ok = $null -ne $actionResult.snapshot -and $actionResult.snapshot['ok'] -eq $true -and
        $actionResult.samples.passed -eq $true -and $resizePassed -and $errors.Count -eq 0
    $actionResult.mouseJournalPresent = Test-Path -LiteralPath $JournalPath
    try {
        $json = ConvertTo-Json -InputObject $actionResult -Depth 16
        $stream = [IO.File]::Open($ResultPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try {
            $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
            $writer.WriteLine($json); $writer.Flush(); $stream.Flush($true); $writer.Dispose()
        } finally { $stream.Dispose() }
    } catch {
        [Console]::Error.WriteLine('Could not write action evidence: ' + $_.Exception.Message)
        exit 1
    }
}
'@

    [IO.File]::WriteAllText($workerScriptPath, $workerScript, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($actionScriptPath, $actionScript, [Text.UTF8Encoding]::new($false))

    $workerArgumentLine = Join-ProcessArguments @(
        '-NoProfile', '-NonInteractive', '-Sta', '-File', (Quote-ProcessArgument $workerScriptPath),
        '-ReadyPath', (Quote-ProcessArgument $workerReadyPath), '-StopPath', (Quote-ProcessArgument $workerStopPath)
    )
    $worker = Start-Process -FilePath $pwshPath -ArgumentList $workerArgumentLine -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $OutputDirectory 'worker.stdout.txt') -RedirectStandardError (Join-Path $OutputDirectory 'worker.stderr.txt')
    $workerIdentity = Wait-ExactProcessIdentity -ProcessId $worker.Id -DeadlineUtc ([DateTime]::UtcNow.AddSeconds(10))
    if ($null -eq $workerIdentity) { throw 'Could not capture exact CIM identity for the launched WinForms worker.' }
    if ($workerIdentity.parentPid -ne $PID) { throw 'WinForms worker is not a direct child of this fixture process.' }

    $workerReadyDeadline = [DateTime]::UtcNow.AddSeconds(15)
    $workerInfo = $null
    while ([DateTime]::UtcNow -lt $workerReadyDeadline) {
        if (Test-Path -LiteralPath $workerReadyPath -PathType Leaf) {
            try { $workerInfo = Get-Content -LiteralPath $workerReadyPath -Raw | ConvertFrom-Json } catch { $workerInfo = $null }
            if ($null -ne $workerInfo) { break }
        }
        $worker.Refresh()
        if ($worker.HasExited) { throw "WinForms worker exited before ready: $($worker.ExitCode)" }
        Start-Sleep -Milliseconds 50
    }
    if ($null -eq $workerInfo) { throw 'WinForms Application.Run worker did not publish its HWND before the readiness deadline.' }
    if ([int]$workerInfo.pid -ne $worker.Id -or [long]$workerInfo.hwnd -le 0) { throw 'WinForms worker ready record does not match its process or contain a valid HWND.' }
    $workerCurrentIdentity = Get-ExactProcessIdentity -ProcessId $worker.Id
    if (-not (Test-SameProcessIdentity -Expected $workerIdentity -Actual $workerCurrentIdentity)) { throw 'WinForms worker exact PID/creation identity changed before snapshot.' }
    $result.identity = [ordered]@{
        pid = $workerIdentity.pid
        parentPid = $workerIdentity.parentPid
        creationTimeUTC = $workerIdentity.creationTimeUTC
        hwnd = [long]$workerInfo.hwnd
    }

    $actionArgumentLine = Join-ProcessArguments @(
        '-NoProfile', '-NonInteractive', '-Sta', '-File', (Quote-ProcessArgument $actionScriptPath),
        '-HelperPath', (Quote-ProcessArgument $HelperPath),
        '-OwnerProcessId', [string]$workerIdentity.pid,
        '-OwnerCreationUtc', (Quote-ProcessArgument $workerIdentity.creationTimeUTC),
        '-WindowHandle', [string][long]$workerInfo.hwnd,
        '-JournalPath', (Quote-ProcessArgument $mouseJournalPath),
        '-ResultPath', (Quote-ProcessArgument $actionResultPath)
    )
    $actionProcess = Start-Process -FilePath $pwshPath -ArgumentList $actionArgumentLine -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $OutputDirectory 'actions.stdout.txt') -RedirectStandardError (Join-Path $OutputDirectory 'actions.stderr.txt')
    $actionIdentity = Wait-ExactProcessIdentity -ProcessId $actionProcess.Id -DeadlineUtc ([DateTime]::UtcNow.AddSeconds(10))
    if ($null -eq $actionIdentity) { throw 'Could not capture exact CIM identity for the action helper child.' }
    if ($actionIdentity.parentPid -ne $PID) { throw 'Action helper is not a direct child of this fixture process.' }

    $actionCompletion = Complete-OwnedChild -Process $actionProcess -Identity $actionIdentity -Label 'action-helper' -TimeoutMilliseconds 90000 -Errors $result.errors
    $actionIdentity = $actionCompletion.identity
    $result.cleanup.actionReaped = $actionCompletion.reaped
    $result.cleanup.actionExactIdentityGone = $actionCompletion.exactIdentityGone
    if (-not $result.cleanup.actionReaped -or -not $result.cleanup.actionExactIdentityGone) { Add-FixtureError $result.errors 'action-cleanup' 'Action helper did not exit and disappear by its exact PID/creation identity.' }

    if (Test-Path -LiteralPath $actionResultPath -PathType Leaf) {
        $actionResult = Get-Content -LiteralPath $actionResultPath -Raw | ConvertFrom-Json
        $result.snapshot = $actionResult.snapshot
        $result.samples = $actionResult.samples
        $result.resize = $actionResult.resize
        foreach ($actionError in @($actionResult.errors)) {
            if ($null -ne $actionError) { Add-FixtureError $result.errors ([string]$actionError.stage) ([string]$actionError.message) }
        }
    } else {
        Add-FixtureError $result.errors 'action-evidence' 'Action helper did not create action-result.json.'
    }

    if (Test-Path -LiteralPath $mouseJournalPath -PathType Leaf) {
        $result.cleanup.mouseJournal = Get-OwnedMouseJournalRecord -Path $mouseJournalPath -OwnerProcessId $workerIdentity.pid -WindowHandle ([long]$workerInfo.hwnd) -HelperProcessId $actionProcess.Id
        if (-not $result.cleanup.mouseJournal.validOwnedJournal) { Add-FixtureError $result.errors 'mouse-journal' 'Mouse journal does not match the fixture worker, HWND, and action process.' }
    } else {
        $result.cleanup.mouseJournal = [ordered]@{ path = $mouseJournalPath; present = $false; safe = $true }
    }
} catch {
    Add-FixtureError $result.errors 'fixture' $_.Exception.Message
} finally {
    if ($null -ne $actionProcess -and (-not $result.cleanup.actionReaped -or -not $result.cleanup.actionExactIdentityGone)) {
        try {
            $actionCompletion = Complete-OwnedChild -Process $actionProcess -Identity $actionIdentity -Label 'action-helper-finally' -TimeoutMilliseconds 90000 -Errors $result.errors
            $actionIdentity = $actionCompletion.identity
            $result.cleanup.actionReaped = $actionCompletion.reaped
            $result.cleanup.actionExactIdentityGone = $actionCompletion.exactIdentityGone
        } catch { Add-FixtureError $result.errors 'action-finally-cleanup' $_.Exception.Message }
    }
    if ($null -ne $mouseJournalPath -and (Test-Path -LiteralPath $mouseJournalPath -PathType Leaf) -and
        $null -eq $recoveryProcess -and $null -ne $actionProcess -and $result.cleanup.actionReaped -and $result.cleanup.actionExactIdentityGone -and
        $null -ne $workerIdentity -and $null -ne $workerInfo) {
        try {
            if ($null -eq $result.cleanup.mouseJournal -or $result.cleanup.mouseJournal.present -ne $true) {
                $result.cleanup.mouseJournal = Get-OwnedMouseJournalRecord -Path $mouseJournalPath -OwnerProcessId $workerIdentity.pid -WindowHandle ([long]$workerInfo.hwnd) -HelperProcessId $actionProcess.Id
            }
            if (-not $result.cleanup.mouseJournal.validOwnedJournal) { throw 'Mouse journal does not match the fixture worker, HWND, and action process.' }

            $recoveryArgumentLine = Join-ProcessArguments @(
                '-NoProfile', '-NonInteractive', '-Sta', '-File', (Quote-ProcessArgument $HelperPath),
                '-AppProcessId', [string]$workerIdentity.pid, '-WindowHandle', [string][long]$workerInfo.hwnd,
                '-Action', 'RecoverMouse', '-InputJournal', (Quote-ProcessArgument $mouseJournalPath),
                '-InputHelperPid', [string]$actionProcess.Id, '-OutputFile', (Quote-ProcessArgument $recoveryResultPath)
            )
            $recoveryProcess = Start-Process -FilePath $pwshPath -ArgumentList $recoveryArgumentLine -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $OutputDirectory 'recovery.stdout.txt') -RedirectStandardError (Join-Path $OutputDirectory 'recovery.stderr.txt')
            $recoveryIdentity = Wait-ExactProcessIdentity -ProcessId $recoveryProcess.Id -DeadlineUtc ([DateTime]::UtcNow.AddSeconds(10))
            if ($null -eq $recoveryIdentity -or $recoveryIdentity.parentPid -ne $PID) { throw 'Could not verify the direct child identity of the fallback mouse recovery helper.' }
            $recoveryCompletion = Complete-OwnedChild -Process $recoveryProcess -Identity $recoveryIdentity -Label 'mouse-recovery-finally' -TimeoutMilliseconds 20000 -Errors $result.errors
            $recoveryIdentity = $recoveryCompletion.identity
            $result.cleanup.recoveryReaped = $recoveryCompletion.reaped
            $result.cleanup.recoveryExactIdentityGone = $recoveryCompletion.exactIdentityGone
            if (Test-Path -LiteralPath $recoveryResultPath -PathType Leaf) {
                $result.cleanup.mouseRecovery = Get-Content -LiteralPath $recoveryResultPath -Raw | ConvertFrom-Json
            }
            if ($null -eq $result.cleanup.mouseRecovery -or $result.cleanup.mouseRecovery.ok -ne $true -or
                $result.cleanup.mouseRecovery.recovery.journalRemoved -ne $true -or $result.cleanup.mouseRecovery.recovery.leftButtonDownAfter -ne $false) {
                Add-FixtureError $result.errors 'mouse-recovery-finally' 'Fallback RecoverMouse did not prove journal removal and left-button-up state.'
            }
        } catch { Add-FixtureError $result.errors 'mouse-recovery-finally' $_.Exception.Message }
    }
    if ($null -ne $recoveryProcess -and (-not $result.cleanup.recoveryReaped -or -not $result.cleanup.recoveryExactIdentityGone)) {
        try {
            $recoveryCompletion = Complete-OwnedChild -Process $recoveryProcess -Identity $recoveryIdentity -Label 'mouse-recovery-finally' -TimeoutMilliseconds 20000 -Errors $result.errors
            $recoveryIdentity = $recoveryCompletion.identity
            $result.cleanup.recoveryReaped = $recoveryCompletion.reaped
            $result.cleanup.recoveryExactIdentityGone = $recoveryCompletion.exactIdentityGone
        } catch { Add-FixtureError $result.errors 'recovery-finally-cleanup' $_.Exception.Message }
    }
    if ($null -eq $recoveryProcess -and ($null -eq $mouseJournalPath -or -not (Test-Path -LiteralPath $mouseJournalPath -PathType Leaf))) {
        if ($null -eq $result.cleanup.mouseJournal -and $null -ne $mouseJournalPath) {
            $result.cleanup.mouseJournal = [ordered]@{ path = $mouseJournalPath; present = $false; safe = $true }
        }
        $result.cleanup.mouseRecovery = [ordered]@{ required = $false; safe = $true }
        $result.cleanup.recoveryReaped = $true
        $result.cleanup.recoveryExactIdentityGone = $true
    } elseif ($null -eq $recoveryProcess -and -not $result.cleanup.actionExactIdentityGone) {
        Add-FixtureError $result.errors 'mouse-recovery' 'Did not recover the owned mouse journal because exact action-process exit was not verified.'
    }

    $journalStillPresent = $null -ne $mouseJournalPath -and (Test-Path -LiteralPath $mouseJournalPath -PathType Leaf)
    $mouseRecoverySettled = -not $journalStillPresent -or
        ($result.cleanup.mouseRecovery.ok -eq $true -and $result.cleanup.mouseRecovery.recovery.journalRemoved -eq $true -and $result.cleanup.mouseRecovery.recovery.leftButtonDownAfter -eq $false)
    $childrenSettled = ($null -eq $actionProcess -or ($result.cleanup.actionReaped -and $result.cleanup.actionExactIdentityGone)) -and
        ($null -eq $recoveryProcess -or ($result.cleanup.recoveryReaped -and $result.cleanup.recoveryExactIdentityGone)) -and $mouseRecoverySettled
    if ($null -ne $worker -and $null -ne $workerStopPath -and $childrenSettled) {
        try {
            [IO.File]::WriteAllText($workerStopPath, $workerStopToken, [Text.UTF8Encoding]::new($false))
            $result.cleanup.workerStopRequested = $true
        } catch { Add-FixtureError $result.errors 'worker-stop' $_.Exception.Message }
        try {
            $workerCompletion = Complete-OwnedChild -Process $worker -Identity $workerIdentity -Label 'winforms-worker' -TimeoutMilliseconds 15000 -Errors $result.errors
            $workerIdentity = $workerCompletion.identity
            $result.cleanup.workerReaped = $workerCompletion.reaped
            $result.cleanup.workerExactIdentityGone = $workerCompletion.exactIdentityGone
        } catch { Add-FixtureError $result.errors 'worker-cleanup' $_.Exception.Message }
        if (-not $result.cleanup.workerReaped -or -not $result.cleanup.workerExactIdentityGone) {
            Add-FixtureError $result.errors 'worker-cleanup' 'Owned WinForms worker did not exit and disappear by exact PID/creation identity.'
        }
    } elseif ($null -ne $worker -and -not $childrenSettled) {
        Add-FixtureError $result.errors 'worker-cleanup' 'Left the owned WinForms worker open because action or recovery child exit could not be verified.'
    }
    $result.cleanup.passed = $result.cleanup.workerReaped -and $result.cleanup.workerExactIdentityGone -and
        $result.cleanup.actionReaped -and $result.cleanup.actionExactIdentityGone -and
        $result.cleanup.recoveryReaped -and $result.cleanup.recoveryExactIdentityGone -and
        ($null -ne $result.cleanup.mouseJournal -and ($result.cleanup.mouseJournal.present -eq $false -or
            ($result.cleanup.mouseRecovery.ok -eq $true -and $result.cleanup.mouseRecovery.recovery.journalRemoved -eq $true -and $result.cleanup.mouseRecovery.recovery.leftButtonDownAfter -eq $false))
        )
    $result.ok = $result.cleanup.passed -and $result.errors.Count -eq 0 -and
        $null -ne $result.identity -and $null -ne $result.snapshot -and $result.snapshot.ok -eq $true -and
        $null -ne $result.samples -and $result.samples.passed -eq $true -and
        $null -ne $result.resize -and $result.resize.passed -eq $true
    if ($outputCreated) {
        try { Write-FreshJson -Path (Join-Path $OutputDirectory 'result.json') -Value $result }
        catch { [Console]::Error.WriteLine('Could not write result.json: ' + $_.Exception.Message); $result.ok = $false }
    }
}

if ($outputCreated) { Write-Output (Get-Content -LiteralPath (Join-Path $OutputDirectory 'result.json') -Raw) }
if ($result.ok) { exit 0 } else { exit 1 }
