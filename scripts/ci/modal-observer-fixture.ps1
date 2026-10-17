<#
.SYNOPSIS
Exercise real native modal windows, including a retained dialog with changed text and a disabled button.
.DESCRIPTION
Diagnostic fixture only. This does not verify Tabby or full desktop parity.
#>
param([string]$HelperPath, [string]$OutputDirectory, [switch]$Worker, [string]$Token, [string]$Behavior, [string]$StatePath)
$ErrorActionPreference = 'Stop'
if ($Worker) {
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.Application]::EnableVisualStyles()
    $owner = [System.Windows.Forms.Form]::new()
    $owner.Text = 'Owned fixture parent'
    $owner.Show()
    $page = [System.Windows.Forms.TaskDialogPage]::new()
    $page.Caption = 'Owned diagnostic task dialog'
    $page.Text = $Token
    $button = [System.Windows.Forms.TaskDialogButton]::new('Paste')
    $button.AllowCloseDialog = $Behavior -eq 'close'
    $page.Buttons.Add($button)
    $page.add_Created({ [IO.File]::WriteAllText($StatePath,'ready') })
    $button.add_Click({
        [IO.File]::WriteAllText($StatePath,'clicked')
        if ($Behavior -ne 'close') { $page.Text = 'Processing, nonce removed'; $button.Enabled = $false }
    })
    [System.Windows.Forms.TaskDialog]::ShowDialog($owner,$page,[System.Windows.Forms.TaskDialogStartupLocation]::CenterOwner) | Out-Null
    Start-Sleep -Seconds 60
    exit 0
}
if (-not [IO.Path]::IsPathFullyQualified($HelperPath) -or -not (Test-Path -LiteralPath $HelperPath -PathType Leaf)) { throw 'Existing absolute helper path required' }
if (-not [IO.Path]::IsPathFullyQualified($OutputDirectory) -or (Test-Path -LiteralPath $OutputDirectory)) { throw 'Fresh absolute output directory required' }
[IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null
function Start-OwnedProcess([string[]]$Arguments) {
    $si = [Diagnostics.ProcessStartInfo]::new((Get-Process -Id $PID).Path)
    $si.UseShellExecute = $false
    foreach ($argument in $Arguments) { $si.ArgumentList.Add($argument) }
    return [Diagnostics.Process]::Start($si)
}
function Invoke-Helper([string]$File, [int]$OwnerPid, [string]$Nonce, [string]$Action, [string]$Directory) {
    $p = Start-OwnedProcess @('-NoProfile','-File',$File,'-AppProcessId',"$OwnerPid",'-ExpectedTextToken',$Nonce,'-ButtonText','Paste','-Action',$Action,'-OutputDirectory',$Directory)
    try {
        if (-not $p.WaitForExit(30000)) { $p.Kill(); throw 'Dialog helper external deadline exceeded' }
        $resultPath = Join-Path $Directory "dialog-$($Action.ToLowerInvariant()).json"
        return @{exitCode=$p.ExitCode; result=(Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json)}
    } finally { $p.Dispose() }
}
$source = Get-Content -LiteralPath $HelperPath -Raw
$oldStart = $source.IndexOf('        $present = $true')
$oldEnd = $source.IndexOf("`n    }`n    `$result.ok", $oldStart)
if ($oldStart -lt 0 -or $oldEnd -le $oldStart) { throw 'Fixed presence loop boundary missing' }
$oldLoop = @'
        $present = $dialog
        do {
            Start-Sleep -Milliseconds 100
            $present = Get-DialogMatch -ExpectedStart $startIdentity
            if ($present -and $present.hwnd -ne $dialog.hwnd) { throw 'A different matching dialog prevents disappearance confirmation' }
        } while ($present -and $present.hwnd -eq $dialog.hwnd -and [DateTime]::UtcNow -lt $until)
        $result.disappeared = ($null -eq $present)
        if (-not $result.disappeared) { throw 'Matching dialog did not disappear within 3 seconds' }
'@
$oldHelper = Join-Path $OutputDirectory 'before-fix.ps1'
[IO.File]::WriteAllText($oldHelper, $source.Substring(0,$oldStart) + $oldLoop + $source.Substring($oldEnd), [Text.UTF8Encoding]::new($false))
$cases = @(
    @{name='before-retained'; helper=$oldHelper; behavior='disabled'; expectOk=$true; expectGone=$true},
    @{name='fixed-retained'; helper=$HelperPath; behavior='disabled'; expectOk=$false; expectGone=$false},
    @{name='fixed-close'; helper=$HelperPath; behavior='close'; expectOk=$true; expectGone=$true}
)
$reports = @()
foreach ($case in $cases) {
    $dir = Join-Path $OutputDirectory $case.name
    [IO.Directory]::CreateDirectory($dir) | Out-Null
    $nonce = 'owned-modal-' + [Guid]::NewGuid().ToString('N')
    $state = Join-Path $dir 'fixture-state.txt'
    $args = @('-NoProfile','-STA','-File',$PSCommandPath,'-Worker','-Token',$nonce,'-Behavior',$case.behavior,'-StatePath',$state)
    $workerProcess = Start-OwnedProcess $args
    try {
        $readyDeadline = [DateTime]::UtcNow.AddSeconds(15)
        while (-not (Test-Path -LiteralPath $state) -and [DateTime]::UtcNow -lt $readyDeadline -and -not $workerProcess.HasExited) { Start-Sleep -Milliseconds 100 }
        if (-not (Test-Path -LiteralPath $state)) { throw 'Actual native fixture did not become ready' }
        $inspect = Invoke-Helper $case.helper $workerProcess.Id $nonce 'Inspect' $dir
        if ($inspect.exitCode -ne 0 -or -not $inspect.result.ok -or $inspect.result.actualAction -or (Get-Content -LiteralPath $state -Raw) -cne 'ready') { throw 'Inspect failed or acted on the dialog' }
        $confirm = Invoke-Helper $case.helper $workerProcess.Id $nonce 'Confirm' $dir
        if ($confirm.result.ok -ne $case.expectOk -or $confirm.result.disappeared -ne $case.expectGone -or -not $confirm.result.actualAction -or (Get-Content -LiteralPath $state -Raw) -cne 'clicked') { throw "Unexpected actual dialog result: $($case.name)" }
        if (($confirm.exitCode -eq 0) -ne $case.expectOk) { throw 'Helper exit result does not match verified result' }
        $reports += @{case=$case.name; inspect=$inspect; confirm=$confirm; beforeFixFalsePositive=($case.name -eq 'before-retained')}
    } finally {
        if (-not $workerProcess.HasExited) { $workerProcess.Kill(); $workerProcess.WaitForExit(5000) | Out-Null }
        $workerProcess.Dispose()
    }
}
@{kind='native-modal-observer-regression'; ok=$true; productionAppVerified=$false; cases=$reports} | ConvertTo-Json -Depth 12 | Set-Content (Join-Path $OutputDirectory 'result.json')
