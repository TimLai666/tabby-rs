<#
.SYNOPSIS
Exercise the owned-dialog observer with real Win32 modal windows, including a retained disabled dialog.
.DESCRIPTION
Diagnostic fixture only. This does not verify Tabby or full desktop parity.
#>
param([string]$HelperPath, [string]$OutputDirectory, [switch]$Worker, [string]$Token, [string]$Behavior, [string]$StatePath)
$ErrorActionPreference = 'Stop'
if ($Worker) {
    Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Text;
using System.Threading;
using System.Runtime.InteropServices;
public static class ModalFixture {
    delegate IntPtr DialogProc(IntPtr w, uint m, UIntPtr wp, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr CreateWindowExW(uint ex, string cls, string title, uint style, int x, int y, int cx, int cy, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr param);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr DialogBoxIndirectParamW(IntPtr instance, IntPtr template, IntPtr parent, DialogProc callback, IntPtr param);
    [DllImport("user32.dll")] static extern bool EndDialog(IntPtr w, IntPtr value);
    [DllImport("user32.dll")] static extern bool EnableWindow(IntPtr w, bool enabled);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern bool SetWindowTextW(IntPtr w, string value);
    public static void Run(string token, string behavior, string statePath) {
        IntPtr label = IntPtr.Zero;
        DialogProc callback = (w,m,wp,lp) => {
            if (m == 0x110) {
                label = CreateWindowExW(0,"STATIC",token,0x50000000,12,20,400,40,w,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero);
                CreateWindowExW(0,"BUTTON","Paste",0x50010001,12,80,100,30,w,new IntPtr(100),IntPtr.Zero,IntPtr.Zero);
                File.WriteAllText(statePath + ".hwnd",w.ToInt64().ToString());
                File.WriteAllText(statePath,"ready");
                return new IntPtr(1);
            }
            if (m == 0x111 && (wp.ToUInt64() & 0xffff) == 100) {
                File.WriteAllText(statePath,"clicked");
                if (behavior == "close") EndDialog(w,new IntPtr(100));
                else { SetWindowTextW(label,"Processing, nonce removed"); EnableWindow(w,false); }
                return new IntPtr(1);
            }
            return IntPtr.Zero;
        };
        var stream = new MemoryStream();
        var b = new BinaryWriter(stream,Encoding.Unicode);
        b.Write((uint)0x90c80080); b.Write((uint)0); b.Write((ushort)0);
        b.Write((short)10); b.Write((short)10); b.Write((short)300); b.Write((short)120);
        b.Write((ushort)0); b.Write((ushort)0); b.Write(Encoding.Unicode.GetBytes("Owned diagnostic modal\0"));
        var bytes = stream.ToArray();
        var template = Marshal.AllocHGlobal(bytes.Length);
        try {
            Marshal.Copy(bytes,0,template,bytes.Length);
            var owner = CreateWindowExW(0,"STATIC","Owned fixture parent",0x10cf0000,20,20,500,300,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero);
            if (owner == IntPtr.Zero) throw new Exception("fixture owner creation failed");
            var value = DialogBoxIndirectParamW(IntPtr.Zero,template,owner,callback,IntPtr.Zero);
            if (value.ToInt64() == -1) throw new Exception("fixture modal creation failed");
            GC.KeepAlive(callback);
            Thread.Sleep(60000);
        } finally { Marshal.FreeHGlobal(template); }
    }
}
'@
    [ModalFixture]::Run($Token,$Behavior,$StatePath)
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
    @{name='before-disabled'; helper=$oldHelper; behavior='disabled'; expectOk=$true; expectGone=$true},
    @{name='fixed-disabled'; helper=$HelperPath; behavior='disabled'; expectOk=$false; expectGone=$false},
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
        Add-Type -AssemblyName UIAutomationClient
        Add-Type -AssemblyName UIAutomationTypes
        $hwnd = [IntPtr]::new([long](Get-Content -LiteralPath ($state + '.hwnd') -Raw))
        $actual = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
        $wp = $null
        $hasPattern = $actual.TryGetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern,[ref]$wp)
        $condition = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ProcessIdProperty,$workerProcess.Id)
        $top = [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children,$condition)
        $children = $actual.FindAll([System.Windows.Automation.TreeScope]::Children,[System.Windows.Automation.Condition]::TrueCondition)
        $summary = @{hwnd=$hwnd.ToInt64(); name=$actual.Current.Name; class=$actual.Current.ClassName; isOffscreen=$actual.Current.IsOffscreen; isEnabled=$actual.Current.IsEnabled; windowPattern=$hasPattern; modal=$(if($hasPattern){$wp.Current.IsModal}else{$null}); top=@($top | ForEach-Object { @{hwnd=$_.Current.NativeWindowHandle;name=$_.Current.Name;class=$_.Current.ClassName} }); children=@($children | ForEach-Object { @{name=$_.Current.Name;class=$_.Current.ClassName;control=$_.Current.ControlType.ProgrammaticName;offscreen=$_.Current.IsOffscreen;enabled=$_.Current.IsEnabled} })}
        $summary | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $dir 'actual-uia.json')
        $inspect = Invoke-Helper $case.helper $workerProcess.Id $nonce 'Inspect' $dir
        if ($inspect.exitCode -ne 0 -or -not $inspect.result.ok -or $inspect.result.actualAction -or (Get-Content -LiteralPath $state -Raw) -cne 'ready') { throw 'Inspect failed or acted on the dialog' }
        $confirm = Invoke-Helper $case.helper $workerProcess.Id $nonce 'Confirm' $dir
        if ($confirm.result.ok -ne $case.expectOk -or $confirm.result.disappeared -ne $case.expectGone -or -not $confirm.result.actualAction -or (Get-Content -LiteralPath $state -Raw) -cne 'clicked') { throw "Unexpected actual dialog result: $($case.name)" }
        if (($confirm.exitCode -eq 0) -ne $case.expectOk) { throw 'Helper exit result does not match verified result' }
        $reports += @{case=$case.name; inspect=$inspect; confirm=$confirm; beforeFixFalsePositive=($case.name -eq 'before-disabled')}
    } finally {
        if (-not $workerProcess.HasExited) { $workerProcess.Kill(); $workerProcess.WaitForExit(5000) | Out-Null }
        $workerProcess.Dispose()
    }
}
@{kind='native-modal-observer-regression'; ok=$true; productionAppVerified=$false; cases=$reports} | ConvertTo-Json -Depth 12 | Set-Content (Join-Path $OutputDirectory 'result.json')
