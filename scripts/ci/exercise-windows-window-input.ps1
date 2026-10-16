<#
.SYNOPSIS
Exercise one existing owned application's native window using actual mouse input.
.DESCRIPTION
Checks PID creation identity and HWND ownership before any action. Snapshot only
observes. Drag and Resize use SendInput and bounded cursor motion; caption actions
click fresh native TITLEBARINFOEX rectangles. Never creates or shows a window,
sends a system command, changes a window rectangle programmatically, or kills a process.
Writes one fresh JSON result, restores the thread DPI context, and always releases
its own pressed mouse button. Use Get-Help -Full for argument details.
.PARAMETER AppProcessId
Existing owned root process ID.
.PARAMETER CreationUtc
Exact CIM CreationDate.ToUniversalTime().ToString('o') recorded by the launcher.
.PARAMETER WindowHandle
Existing owned visible top-level HWND, positive decimal value.
.PARAMETER Action
Snapshot, Drag, Resize, Minimize, Maximize, Restore, or Close.
.PARAMETER PointX
Screen pixel X for the Drag start point; Resize uses the visible right border midpoint.
.PARAMETER PointY
Screen pixel Y for the Drag start point.
.PARAMETER DeltaX
Bounded screen pixel horizontal movement for Drag or Resize (maximum absolute 120).
.PARAMETER DeltaY
Bounded screen pixel vertical movement for Drag; Resize requires zero.
.PARAMETER OutputFile
Fresh absolute JSON evidence path inside the launcher's owned output directory.
.EXAMPLE
pwsh -NoProfile -File exercise-windows-window-input.ps1 -AppProcessId 1234 -CreationUtc 2026-10-16T15:00:00.0000000Z -WindowHandle 12345 -Action Snapshot -OutputFile C:\owned\snapshot.json
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateRange(1,2147483647)][int]$AppProcessId,
    [Parameter(Mandatory=$true)][string]$CreationUtc,
    [Parameter(Mandatory=$true)][ValidateRange(1,9223372036854775807)][long]$WindowHandle,
    [Parameter(Mandatory=$true)][ValidateSet('Snapshot','Drag','Resize','Minimize','Maximize','Restore','Close')][string]$Action,
    [int]$PointX=0, [int]$PointY=0,
    [ValidateRange(-120,120)][int]$DeltaX=60,
    [ValidateRange(-120,120)][int]$DeltaY=40,
    [Parameter(Mandatory=$true)][string]$OutputFile
)
$ErrorActionPreference='Stop'

$script = @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;

public static class ExistingWindowInput {
    const uint GA_ROOT=2, WM_GETTITLEBARINFOEX=0x033F, WM_NCHITTEST=0x0084;
    const uint SMTO_BLOCK=1, SMTO_ABORTIFHUNG=2, INPUT_MOUSE=0;
    const uint LEFTDOWN=2, LEFTUP=4, STATE_UNAVAILABLE=1, STATE_INVISIBLE=0x8000, STATE_OFFSCREEN=0x10000;
    const int HTRIGHT=11;
    static IntPtr hwnd; static int pid; static long creationFileTime; static bool buttonDown;
    [StructLayout(LayoutKind.Sequential)] struct POINT { public int x,y; }
    [StructLayout(LayoutKind.Sequential)] struct RECT { public int left,top,right,bottom; }
    [StructLayout(LayoutKind.Sequential)] struct TITLEBARINFOEX { public uint cbSize; public RECT rcTitleBar; [MarshalAs(UnmanagedType.ByValArray,SizeConst=6)] public uint[] rgstate; [MarshalAs(UnmanagedType.ByValArray,SizeConst=6)] public RECT[] rgrect; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx,dy; public uint mouseData,dwFlags,time; public UIntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit,Size=32)] struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mi; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public INPUTUNION U; }
    [StructLayout(LayoutKind.Sequential)] struct FILETIME { public uint low,high; }
    [DllImport("user32.dll")] static extern int IsWindow(IntPtr h);
    [DllImport("user32.dll")] static extern int IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern int IsIconic(IntPtr h);
    [DllImport("user32.dll")] static extern int IsZoomed(IntPtr h);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint processId);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h,uint flags);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT p);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern int SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern int GetWindowRect(IntPtr h,out RECT r);
    [DllImport("user32.dll")] static extern int GetClientRect(IntPtr h,out RECT r);
    [DllImport("user32.dll")] static extern int ClientToScreen(IntPtr h,ref POINT p);
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll",SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint msg,UIntPtr wp,IntPtr lp,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll",SetLastError=true)] static extern int SetCursorPos(int x,int y);
    [DllImport("user32.dll",SetLastError=true)] static extern uint SendInput(uint n,INPUT[] a,int size);
    [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr OpenProcess(uint access,int inherit,int pid);
    [DllImport("kernel32.dll",SetLastError=true)] static extern int GetProcessTimes(IntPtr h,out FILETIME created,out FILETIME exit,out FILETIME kernel,out FILETIME user);
    [DllImport("kernel32.dll")] static extern int CloseHandle(IntPtr h);
    [DllImport("user32.dll",SetLastError=true)] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);

    // CIM_DATETIME preserves microseconds, while FILETIME has 100ns ticks.
    static long FileTime(FILETIME f) { return ((long)f.high<<32)|f.low; }
    static void Check(bool ok,string message) { if(!ok) throw new InvalidOperationException(message); }
    static Dictionary<string,object> Bounds(RECT r) { return new Dictionary<string,object>{{"left",r.left},{"top",r.top},{"right",r.right},{"bottom",r.bottom}}; }
    static Dictionary<string,object> ReadSnapshot() {
        Validate(); RECT outer,client; Check(GetWindowRect(hwnd,out outer)!=0,"GetWindowRect failed"); Check(GetClientRect(hwnd,out client)!=0,"GetClientRect failed");
        POINT origin=new POINT{x=client.left,y=client.top}; Check(ClientToScreen(hwnd,ref origin)!=0,"ClientToScreen failed");
        return new Dictionary<string,object>{{"outer",Bounds(outer)},{"client",new Dictionary<string,object>{{"screenX",origin.x},{"screenY",origin.y},{"width",client.right-client.left},{"height",client.bottom-client.top},{"dpi",GetDpiForWindow(hwnd)}}},{"minimized",IsIconic(hwnd)!=0},{"maximized",IsZoomed(hwnd)!=0}};
    }
    static void Validate() {
        Check(IsWindow(hwnd)!=0,"original HWND no longer exists"); uint actualPid; GetWindowThreadProcessId(hwnd,out actualPid);
        Check(actualPid==(uint)pid,"HWND process ownership changed"); Check(GetAncestor(hwnd,GA_ROOT)==hwnd,"HWND is not its own GA_ROOT");
        Check(IsWindowVisible(hwnd)!=0,"owned top-level HWND is not visible");
        IntPtr ph=OpenProcess(0x1000,0,pid); Check(ph!=IntPtr.Zero,"OpenProcess for identity check failed");
        try { FILETIME c,e,k,u; Check(GetProcessTimes(ph,out c,out e,out k,out u)!=0,"GetProcessTimes failed"); Check(FileTime(c)/10==creationFileTime/10,"process creation identity changed"); }
        finally { CloseHandle(ph); }
    }
    static void Foreground() {
        Validate(); if(GetForegroundWindow()!=hwnd) SetForegroundWindow(hwnd);
        var sw=Stopwatch.StartNew(); while(sw.ElapsedMilliseconds<1500) { Validate(); if(GetForegroundWindow()==hwnd)return; Thread.Sleep(25); }
        throw new InvalidOperationException("owned HWND did not become foreground");
    }
    static uint Send(uint flags) {
        INPUT i=new INPUT(); i.type=INPUT_MOUSE; i.U.mi=new MOUSEINPUT{dwFlags=flags};
        uint sent=SendInput(1,new[]{i},Marshal.SizeOf(typeof(INPUT)));
        Check(sent==1,"SendInput failed to deliver mouse event; Win32="+Marshal.GetLastWin32Error()); return sent;
    }
    static void PositionAndVerify(POINT p,bool exactRoot) {
        Check(SetCursorPos(p.x,p.y)!=0,"SetCursorPos to validated input point failed; Win32="+Marshal.GetLastWin32Error());
        Validate(); Check(GetForegroundWindow()==hwnd,"foreground target changed before input");
        IntPtr under=WindowFromPoint(p); Check(under!=IntPtr.Zero,"input point no longer resolves to a window");
        Check((exactRoot ? under==hwnd : GetAncestor(under,GA_ROOT)==hwnd),"input point moved over a foreign window");
    }
    static void ClickDown() { Send(LEFTDOWN); buttonDown=true; }
    static void ClickUp() { if(buttonDown) { Send(LEFTUP); buttonDown=false; } }
    static void MoveGradually(int x0,int y0,int x1,int y1) {
        const int steps=12;
        for(int i=1;i<=steps;i++) { Validate(); Check(GetForegroundWindow()==hwnd,"foreground changed during mouse input"); int x=x0+(int)Math.Round((x1-x0)*(i/(double)steps)); int y=y0+(int)Math.Round((y1-y0)*(i/(double)steps)); Check(SetCursorPos(x,y)!=0,"SetCursorPos failed; Win32="+Marshal.GetLastWin32Error()); Thread.Sleep(20); }
    }
    static void WaitGeometry(RECT before,string action,int dx,int dy) {
        var sw=Stopwatch.StartNew(); RECT r;
        while(sw.ElapsedMilliseconds<3000) { Validate(); Check(GetWindowRect(hwnd,out r)!=0,"GetWindowRect failed after input");
            if(action=="Drag" && Math.Abs((r.left-before.left)-dx)<=3 && Math.Abs((r.top-before.top)-dy)<=3 && r.right-r.left==before.right-before.left && r.bottom-r.top==before.bottom-before.top) return;
            if(action=="Resize" && Math.Abs((r.right-before.right)-dx)<=3 && r.left==before.left && r.top==before.top && r.bottom==before.bottom) return;
            Thread.Sleep(40);
        }
        throw new InvalidOperationException(action+" geometry change was not observed within 3 pixels");
    }
    static int HitTest(int x,int y) { long p=((long)(ushort)x)|((long)(ushort)y<<16); UIntPtr result; Check(SendMessageTimeoutW(hwnd,WM_NCHITTEST,UIntPtr.Zero,new IntPtr(p),SMTO_BLOCK|SMTO_ABORTIFHUNG,1000,out result)!=IntPtr.Zero,"WM_NCHITTEST timed out"); return unchecked((int)result.ToUInt64()); }
    static POINT CaptionCenter(int index) {
        TITLEBARINFOEX info=new TITLEBARINFOEX{cbSize=(uint)Marshal.SizeOf(typeof(TITLEBARINFOEX)),rgstate=new uint[6],rgrect=new RECT[6]};
        IntPtr p=Marshal.AllocHGlobal(Marshal.SizeOf(typeof(TITLEBARINFOEX)));
        try { Marshal.StructureToPtr(info,p,false); UIntPtr ignored; Check(SendMessageTimeoutW(hwnd,WM_GETTITLEBARINFOEX,UIntPtr.Zero,p,SMTO_BLOCK|SMTO_ABORTIFHUNG,1000,out ignored)!=IntPtr.Zero,"WM_GETTITLEBARINFOEX failed or timed out"); info=Marshal.PtrToStructure<TITLEBARINFOEX>(p); }
        finally { Marshal.FreeHGlobal(p); }
        RECT r=info.rgrect[index]; uint s=info.rgstate[index]; Check(r.right>r.left&&r.bottom>r.top,"caption button rectangle is empty"); Check((s&(STATE_UNAVAILABLE|STATE_INVISIBLE|STATE_OFFSCREEN))==0,"caption button is unavailable, invisible, or offscreen");
        POINT c=new POINT{x=r.left+(r.right-r.left)/2,y=r.top+(r.bottom-r.top)/2}; Check(WindowFromPoint(c)==hwnd&&GetAncestor(WindowFromPoint(c),GA_ROOT)==hwnd,"caption center is not owned root HWND"); return c;
    }
    static bool WaitState(string action,bool icon,bool zoom) { var sw=Stopwatch.StartNew(); while(sw.ElapsedMilliseconds<3000) { if(action=="Close") { if(IsWindow(hwnd)==0)return true; } else { Validate(); if((IsIconic(hwnd)!=0)==icon&&(IsZoomed(hwnd)!=0)==zoom)return true; } Thread.Sleep(40); } return false; }
    public static Dictionary<string,object> Run(int processId,string creationUtc,long windowHandle,string action,int px,int py,int dx,int dy) {
        Check(Environment.Is64BitProcess,"this input helper requires 64-bit PowerShell"); pid=processId; hwnd=new IntPtr(windowHandle); DateTimeOffset parsed; Check(DateTimeOffset.TryParse(creationUtc,out parsed),"CreationUtc is invalid"); creationFileTime=parsed.UtcDateTime.ToFileTimeUtc();
        IntPtr oldDpi=IntPtr.Zero; bool changed=false; Dictionary<string,object> result=new Dictionary<string,object>{{"ok",false},{"before",null},{"after",null},{"inputSent",false},{"error",null}};
        try {
            oldDpi=SetThreadDpiAwarenessContext(new IntPtr(-4)); changed=oldDpi!=IntPtr.Zero; Check(changed,"could not set per-monitor-v2 thread DPI context");
            Validate(); result["before"]=ReadSnapshot();
            if(action=="Snapshot") { result["after"]=result["before"]; result["ok"]=true; return result; }
            bool wasIcon=IsIconic(hwnd)!=0, wasZoom=IsZoomed(hwnd)!=0;
            if(action=="Maximize") Check(!wasIcon&&!wasZoom,"Maximize requires an ordinary window");
            if(action=="Restore") Check(wasZoom&&!wasIcon,"Restore requires a maximized window");
            Foreground(); RECT before; Check(GetWindowRect(hwnd,out before)!=0,"GetWindowRect failed before input");
            POINT start,end;
            if(action=="Drag") {
                Check(px>=before.left&&px<before.right&&py>=before.top&&py<before.bottom,"Drag point is outside current outer bounds");
                start=new POINT{x=px,y=py}; IntPtr under=WindowFromPoint(start); Check(under!=IntPtr.Zero&&GetAncestor(under,GA_ROOT)==hwnd,"Drag point is not on an owned descendant");
                end=new POINT{x=px+dx,y=py+dy}; Check(dx!=0||dy!=0,"Drag delta must move the window");
                PositionAndVerify(start,false); ClickDown(); result["inputSent"]=true; MoveGradually(start.x,start.y,end.x,end.y); ClickUp(); WaitGeometry(before,action,dx,dy);
            } else if(action=="Resize") {
                Check(dy==0&&dx!=0,"Resize requires nonzero DeltaX and DeltaY=0");
                start=new POINT{x=before.right-1,y=before.top+(before.bottom-before.top)/2}; IntPtr under=WindowFromPoint(start);
                Check(under!=IntPtr.Zero&&GetAncestor(under,GA_ROOT)==hwnd,"right border midpoint is not on an owned descendant"); Check(HitTest(start.x,start.y)==HTRIGHT,"right border midpoint did not hit-test as HTRIGHT");
                end=new POINT{x=start.x+dx,y=start.y}; PositionAndVerify(start,false); Check(HitTest(start.x,start.y)==HTRIGHT,"right edge lost HTRIGHT hit-test before input"); ClickDown(); result["inputSent"]=true; MoveGradually(start.x,start.y,end.x,end.y); ClickUp(); WaitGeometry(before,action,dx,0);
            } else {
                int index=action=="Minimize"?2:action=="Close"?5:3; POINT c=CaptionCenter(index);
                PositionAndVerify(c,true); ClickDown(); result["inputSent"]=true; ClickUp();
                bool expectedIcon=action=="Minimize"?true:false, expectedZoom=action=="Maximize"?true:action=="Restore"?false:wasZoom;
                Check(WaitState(action,expectedIcon,expectedZoom),action+" state was not observed after real mouse input");
            }
            if(action=="Close") { Check(IsWindow(hwnd)==0,"Close input did not destroy original HWND"); result["after"]=null; }
            else result["after"]=ReadSnapshot();
            result["ok"]=true;
        } catch(Exception ex) { result["error"]=ex.GetType().Name+": "+ex.Message; try { if(IsWindow(hwnd)!=0)result["after"]=ReadSnapshot(); } catch {} }
        finally { if(buttonDown) { try { Send(LEFTUP); buttonDown=false; } catch(Exception ex) { result["ok"]=false; if(result["error"]==null)result["error"]="mouse release failed: "+ex.Message; } } if(changed) { try { if(SetThreadDpiAwarenessContext(oldDpi)==IntPtr.Zero) { result["ok"]=false; if(result["error"]==null)result["error"]="failed to restore thread DPI context"; } } catch(Exception ex) { result["ok"]=false; if(result["error"]==null)result["error"]="DPI restore failed: "+ex.Message; } } }
        return result;
    }
}
'@

$result = [ordered]@{ ok=$false; action=$Action; appProcessId=$AppProcessId; hwnd=$WindowHandle; before=$null; after=$null; inputSent=$false; error=$null }
$canWrite = $false
try {
    if (-not [IO.Path]::IsPathRooted($OutputFile)) { throw 'OutputFile must be an absolute path' }
    $fullOutput = [IO.Path]::GetFullPath($OutputFile)
    $parent = [IO.Path]::GetDirectoryName($fullOutput)
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) { throw 'OutputFile parent directory does not exist' }
    if (Test-Path -LiteralPath $fullOutput) { throw 'OutputFile already exists' }
    $canWrite = $true
    if (-not $IsWindows) { throw 'This helper requires Windows PowerShell 7+' }
    $proc = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = $AppProcessId" -ErrorAction Stop
    if ($null -eq $proc) { throw 'Owned process does not exist' }
    $actualCreation = $proc.CreationDate.ToUniversalTime().ToString('o')
    if ($actualCreation -cne $CreationUtc) { throw 'CreationUtc does not match the owned process identity' }
    Add-Type -TypeDefinition $script -Language CSharp -ErrorAction Stop
    $native = [ExistingWindowInput]::Run($AppProcessId,$CreationUtc,$WindowHandle,$Action,$PointX,$PointY,$DeltaX,$DeltaY)
    foreach ($key in @('ok','before','after','inputSent','error')) { $result[$key]=$native[$key] }
} catch {
    $result.error = $_.Exception.GetType().Name + ': ' + $_.Exception.Message
}
if ($canWrite) {
    $json = $result | ConvertTo-Json -Depth 8
    $stream = $null
    try {
        $stream = [IO.File]::Open($fullOutput,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
        $writer = [IO.StreamWriter]::new($stream,[Text.UTF8Encoding]::new($false))
        $writer.WriteLine($json); $writer.Flush(); $writer.Dispose(); $stream=$null
    } catch {
        if ($null -ne $stream) { $stream.Dispose() }
        [Console]::Error.WriteLine('Could not create fresh evidence file: ' + $_.Exception.Message)
        exit 1
    }
}
[Console]::WriteLine(($result | ConvertTo-Json -Depth 8 -Compress))
if ($result.ok) { exit 0 } else { exit 1 }
