<#
.SYNOPSIS
Exercise or recover mouse input for one existing owned application's native window.
.DESCRIPTION
Checks PID creation identity and HWND ownership before an action. Snapshot only
observes. Drag and Resize use real mouse input and bounded cursor motion; caption
actions click fresh native TITLEBARINFOEX rectangles, or uniquely matched accessible
caption controls with the same owned root and native hit-test when original self-painted
controls have no system rectangle. RecoverMouse validates and
releases only the left button recorded in a fresh owned mouse journal. The journal
contains three UTF-8 decimal lines: owner PID, root HWND, and helper PID. Recovery
records the button state and journal outcome and does not require a live target HWND.
Activate raises the existing owned visible window within normal Z order and clicks only the supplied owned client point to request foreground.
Never creates or shows a window, sends a system command, changes a window rectangle programmatically,
or kills a process. Run normal actions with a 25-second process timeout and reserve at
least 45 seconds for the action and recovery. Run RecoverMouse with a 15-second timeout.
After a timed-out or failed action, recover a remaining verified journal, retain the
recovery JSON as evidence, and do not launch another action unless recovery succeeds.
Use Get-Help -Full for argument details.
.PARAMETER AppProcessId
Existing owner process ID recorded in the mouse journal.
.PARAMETER CreationUtc
Exact CIM CreationDate.ToUniversalTime().ToString('o') recorded by the launcher; required except for RecoverMouse.
.PARAMETER WindowHandle
Existing owned root HWND, positive decimal value; recovery does not require it to remain live.
.PARAMETER Action
Snapshot, Activate, Drag, Resize, Minimize, Maximize, Restore, Close, or RecoverMouse.
.PARAMETER PointX
Screen pixel X for Drag or an Activate point inside the caller-verified terminal client; Resize uses the visible right border midpoint.
.PARAMETER PointY
Screen pixel Y for Drag or Activate.
.PARAMETER DeltaX
Bounded screen pixel horizontal movement for Drag or Resize (maximum absolute 120).
.PARAMETER DeltaY
Bounded screen pixel vertical movement for Drag; Resize requires zero.
.PARAMETER InputJournal
For RecoverMouse, absolute path to the journal in the same directory as OutputFile.
.PARAMETER InputHelperPid
For RecoverMouse, decimal PID of the terminated helper recorded in InputJournal.
.PARAMETER OutputFile
Fresh absolute JSON evidence path inside the launcher's owned output directory.
.EXAMPLE
pwsh -NoProfile -File exercise-windows-window-input.ps1 -AppProcessId 1234 -CreationUtc 2026-10-16T15:00:00.0000000Z -WindowHandle 12345 -Action Snapshot -OutputFile C:\owned\snapshot.json
.EXAMPLE
pwsh -NoProfile -File exercise-windows-window-input.ps1 -AppProcessId 1234 -WindowHandle 12345 -Action RecoverMouse -InputJournal C:\owned\drag.json.mouse-down -InputHelperPid 5678 -OutputFile C:\owned\recover.json
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][int]$AppProcessId,
    [string]$CreationUtc,
    [Parameter(Mandatory=$true)][long]$WindowHandle,
    [Parameter(Mandatory=$true)][string]$Action,
    [int]$PointX=0, [int]$PointY=0,
    [int]$DeltaX=60,
    [int]$DeltaY=40,
    [string]$InputJournal,
    [int]$InputHelperPid=0,
    [Parameter(Mandatory=$true)][string]$OutputFile
)
$ErrorActionPreference='Stop'

$script = @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class ExistingWindowInput {
    const uint GA_ROOT=2, WM_GETTITLEBARINFOEX=0x033F, WM_NCHITTEST=0x0084;
    const uint SMTO_BLOCK=1, SMTO_ABORTIFHUNG=2, INPUT_MOUSE=0;
    const uint LEFTDOWN=2, LEFTUP=4, STATE_UNAVAILABLE=1, STATE_INVISIBLE=0x8000, STATE_OFFSCREEN=0x10000;
    const int HTRIGHT=11, VK_LBUTTON=1, DRAG_STEPS=12;
    static IntPtr hwnd; static int pid; static long creationFileTime; static bool buttonDown; static string journalPath;
    static readonly List<Dictionary<string,object>> inputTrace=new List<Dictionary<string,object>>();
    [StructLayout(LayoutKind.Sequential)] struct POINT { public int x,y; }
    [StructLayout(LayoutKind.Sequential)] struct RECT { public int left,top,right,bottom; }
    [StructLayout(LayoutKind.Sequential)] struct TITLEBARINFOEX { public uint cbSize; public RECT rcTitleBar; [MarshalAs(UnmanagedType.ByValArray,SizeConst=6)] public uint[] rgstate; [MarshalAs(UnmanagedType.ByValArray,SizeConst=6)] public RECT[] rgrect; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx,dy; public uint mouseData,dwFlags,time; public UIntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit,Size=32)] struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mi; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public INPUTUNION U; }
    [StructLayout(LayoutKind.Sequential)] struct FILETIME { public uint low,high; }
    [StructLayout(LayoutKind.Sequential)] struct GUITHREADINFO { public uint cbSize,flags; public IntPtr active,focus,capture,menuOwner,moveSize,caret; public RECT caretRect; }
    [DllImport("user32.dll")] static extern int IsWindow(IntPtr h);
    [DllImport("user32.dll")] static extern int IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern int IsIconic(IntPtr h);
    [DllImport("user32.dll")] static extern int IsZoomed(IntPtr h);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint processId);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h,uint flags);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT p);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern int SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll",SetLastError=true)] static extern int SetWindowPos(IntPtr h,IntPtr after,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] static extern int GetWindowRect(IntPtr h,out RECT r);
    [DllImport("user32.dll")] static extern IntPtr GetWindowLongPtrW(IntPtr h,int index);
    [DllImport("user32.dll")] static extern int GetClientRect(IntPtr h,out RECT r);
    [DllImport("user32.dll")] static extern int ClientToScreen(IntPtr h,ref POINT p);
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h,StringBuilder text,int capacity);
    [DllImport("user32.dll")] static extern int GetSystemMetricsForDpi(int index,uint dpi);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h,uint attribute,out RECT bounds,uint size);
    [DllImport("user32.dll",SetLastError=true)] static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll",SetLastError=true)] static extern int GetCursorPos(out POINT p);
    [DllImport("user32.dll",SetLastError=true)] static extern int GetClipCursor(out RECT bounds);
    [DllImport("user32.dll",SetLastError=true)] static extern int GetGUIThreadInfo(uint threadId,ref GUITHREADINFO info);
    [DllImport("user32.dll",SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint msg,UIntPtr wp,IntPtr lp,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll",SetLastError=true)] static extern int SetCursorPos(int x,int y);
    [DllImport("user32.dll",SetLastError=true)] static extern uint SendInput(uint n,INPUT[] a,int size);
    [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr OpenProcess(uint access,int inherit,int pid);
    [DllImport("kernel32.dll",SetLastError=true)] static extern int GetProcessTimes(IntPtr h,out FILETIME created,out FILETIME exit,out FILETIME kernel,out FILETIME user);
    [DllImport("kernel32.dll")] static extern int CloseHandle(IntPtr h);
    [DllImport("user32.dll",SetLastError=true)] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);

    static long FileTime(FILETIME f) { return ((long)f.high<<32)|f.low; }
    static bool LeftButtonDown() { return (((ushort)GetAsyncKeyState(VK_LBUTTON))&0x8000)!=0; }
    static void Check(bool ok,string message) { if(!ok) throw new InvalidOperationException(message); }
    static Dictionary<string,object> Bounds(RECT r) { return new Dictionary<string,object>{{"left",r.left},{"top",r.top},{"right",r.right},{"bottom",r.bottom}}; }
    static Dictionary<string,object> ReadSnapshot() {
        Validate(); RECT outer,client; Check(GetWindowRect(hwnd,out outer)!=0,"GetWindowRect failed"); Check(GetClientRect(hwnd,out client)!=0,"GetClientRect failed");
        POINT origin=new POINT{x=client.left,y=client.top}; Check(ClientToScreen(hwnd,ref origin)!=0,"ClientToScreen failed");
        RECT caption; int captionHr=DwmGetWindowAttribute(hwnd,5,out caption,(uint)Marshal.SizeOf(typeof(RECT)));
        RECT extended; int extendedHr=DwmGetWindowAttribute(hwnd,9,out extended,(uint)Marshal.SizeOf(typeof(RECT)));
        return new Dictionary<string,object>{{"outer",Bounds(outer)},{"style",unchecked((uint)GetWindowLongPtrW(hwnd,-16).ToInt64())},{"extendedStyle",unchecked((uint)GetWindowLongPtrW(hwnd,-20).ToInt64())},{"extendedFrame",new Dictionary<string,object>{{"hresult",extendedHr},{"bounds",Bounds(extended)}}},{"client",new Dictionary<string,object>{{"screenX",origin.x},{"screenY",origin.y},{"width",client.right-client.left},{"height",client.bottom-client.top},{"dpi",GetDpiForWindow(hwnd)}}},{"captionButtons",new Dictionary<string,object>{{"hresult",captionHr},{"windowRelativeBounds",Bounds(caption)},{"captionMetric",GetSystemMetricsForDpi(4,GetDpiForWindow(hwnd))}}},{"minimized",IsIconic(hwnd)!=0},{"maximized",IsZoomed(hwnd)!=0}};
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
    static POINT ReadCursor() { POINT p; Check(GetCursorPos(out p)!=0,"GetCursorPos failed; Win32="+Marshal.GetLastWin32Error()); return p; }
    static Dictionary<string,object> ReadGuiState(IntPtr target) {
        uint process; uint thread=GetWindowThreadProcessId(target,out process);
        GUITHREADINFO info=new GUITHREADINFO{cbSize=(uint)Marshal.SizeOf(typeof(GUITHREADINFO))};
        bool available=thread!=0&&GetGUIThreadInfo(thread,ref info)!=0;
        return new Dictionary<string,object>{{"threadId",thread},{"processId",process},{"available",available},{"flags",info.flags},{"active",info.active.ToInt64()},{"focus",info.focus.ToInt64()},{"capture",info.capture.ToInt64()},{"moveSize",info.moveSize.ToInt64()}};
    }
    static void TraceInput(string stage,int requestedX,int requestedY) {
        if(inputTrace.Count>=64)return;
        var row=new Dictionary<string,object>{{"stage",stage},{"requestedX",requestedX},{"requestedY",requestedY}};
        try {
            POINT point=ReadCursor(); IntPtr under=WindowFromPoint(point);
            var name=new StringBuilder(256); GetClassNameW(under,name,name.Capacity);
            row["cursorX"]=point.x; row["cursorY"]=point.y; row["underHwnd"]=under.ToInt64(); row["underClass"]=name.ToString(); row["underRootHwnd"]=GetAncestor(under,GA_ROOT).ToInt64();
            row["foregroundHwnd"]=GetForegroundWindow().ToInt64(); row["leftButtonDown"]=LeftButtonDown(); row["rootGui"]=ReadGuiState(hwnd); row["underGui"]=ReadGuiState(under);
            RECT clip; if(GetClipCursor(out clip)!=0)row["clip"]=Bounds(clip);
            RECT outer; if(GetWindowRect(hwnd,out outer)!=0)row["outer"]=Bounds(outer);
            UIntPtr hit; if(under!=IntPtr.Zero&&SendMessageTimeoutW(under,WM_NCHITTEST,UIntPtr.Zero,new IntPtr(((long)(ushort)point.x)|((long)(ushort)point.y<<16)),SMTO_BLOCK|SMTO_ABORTIFHUNG,100,out hit)!=IntPtr.Zero)row["underHitTest"]=unchecked((int)hit.ToUInt64());
            if(SendMessageTimeoutW(hwnd,WM_NCHITTEST,UIntPtr.Zero,new IntPtr(((long)(ushort)point.x)|((long)(ushort)point.y<<16)),SMTO_BLOCK|SMTO_ABORTIFHUNG,100,out hit)!=IntPtr.Zero)row["rootHitTest"]=unchecked((int)hit.ToUInt64());
        } catch(Exception ex) { row["observationError"]=ex.GetType().Name+": "+ex.Message; }
        inputTrace.Add(row);
    }
    static void CheckCursor(POINT expected) { POINT actual=ReadCursor(); Check(actual.x==expected.x&&actual.y==expected.y,"GetCursorPos did not match requested screen point (cursor clipping)"); }
    static void WaitLeftButtonUp() {
        var sw=Stopwatch.StartNew(); while(sw.ElapsedMilliseconds<500) { if(!LeftButtonDown())return; Thread.Sleep(10); }
        throw new InvalidOperationException("left button remained down after LEFTUP");
    }
    static uint Send(uint flags) {
        INPUT i=new INPUT(); i.type=INPUT_MOUSE; i.U.mi=new MOUSEINPUT{dwFlags=flags};
        uint sent=SendInput(1,new[]{i},Marshal.SizeOf(typeof(INPUT)));
        Check(sent==1,"SendInput failed to deliver mouse event; Win32="+Marshal.GetLastWin32Error()); return sent;
    }
    static void WriteOwnJournal() {
        string[] lines={pid.ToString(CultureInfo.InvariantCulture),hwnd.ToInt64().ToString(CultureInfo.InvariantCulture),Environment.ProcessId.ToString(CultureInfo.InvariantCulture)};
        using(FileStream fs=new FileStream(journalPath,FileMode.CreateNew,FileAccess.Write,FileShare.None))
        using(StreamWriter w=new StreamWriter(fs,new UTF8Encoding(false,true))) { foreach(string line in lines) w.WriteLine(line); w.Flush(); fs.Flush(true); }
    }
    static void CheckJournal(int expectedOwner,long expectedHwnd,int expectedHelper,string path) {
        Check(File.Exists(path),"mouse journal does not exist");
        string[] lines=File.ReadAllLines(path,new UTF8Encoding(false,true)); Check(lines.Length==3,"mouse journal must contain exactly three identity lines");
        int owner,helper; long target;
        Check(Int32.TryParse(lines[0],NumberStyles.None,CultureInfo.InvariantCulture,out owner)&&owner>0,"mouse journal owner PID is invalid");
        Check(Int64.TryParse(lines[1],NumberStyles.None,CultureInfo.InvariantCulture,out target)&&target>0,"mouse journal HWND is invalid");
        Check(Int32.TryParse(lines[2],NumberStyles.None,CultureInfo.InvariantCulture,out helper)&&helper>0,"mouse journal helper PID is invalid");
        Check(owner==expectedOwner&&target==expectedHwnd&&helper==expectedHelper,"mouse journal identity does not match supplied owner, HWND and helper PID");
    }
    static void RemoveVerifiedJournal(int owner,long target,int helper,string path) {
        CheckJournal(owner,target,helper,path); File.Delete(path); Check(!File.Exists(path),"verified mouse journal could not be removed");
    }
    static void PressAt(POINT expected,bool exactRoot,bool requireForeground=true,bool sameProcessDescendant=false) {
        Check(!LeftButtonDown(),"physical left button is already held; refusing to begin input");
        WriteOwnJournal();
        CheckCursor(expected); Validate(); Check(!requireForeground||GetForegroundWindow()==hwnd,"foreground target changed before LEFTDOWN");
        POINT actual=ReadCursor(); IntPtr under=WindowFromPoint(actual); Check(under!=IntPtr.Zero,"cursor point no longer resolves to a window");
        Check(exactRoot ? under==hwnd : GetAncestor(under,GA_ROOT)==hwnd,"cursor ownership changed before LEFTDOWN");
        if(sameProcessDescendant) { uint underPid; GetWindowThreadProcessId(under,out underPid); Check(underPid==(uint)pid,"caption recipient process changed before LEFTDOWN"); }
        Check(!LeftButtonDown(),"left button became held before LEFTDOWN; journal retained for recovery");
        Send(LEFTDOWN); buttonDown=true;
    }
    static void ClickUp() {
        if(!buttonDown)return;
        Send(LEFTUP); WaitLeftButtonUp(); buttonDown=false;
        RemoveVerifiedJournal(pid,hwnd.ToInt64(),Environment.ProcessId,journalPath);
    }
    static POINT PositionAndVerify(POINT requested,bool exactRoot,bool requireForeground=true) {
        Check(SetCursorPos(requested.x,requested.y)!=0,"SetCursorPos to validated input point failed; Win32="+Marshal.GetLastWin32Error());
        POINT actual=ReadCursor(); Check(actual.x==requested.x&&actual.y==requested.y,"cursor was clipped before input");
        Validate(); Check(!requireForeground||GetForegroundWindow()==hwnd,"foreground target changed before input");
        IntPtr under=WindowFromPoint(actual); Check(under!=IntPtr.Zero,"input point no longer resolves to a window");
        IntPtr underRoot=GetAncestor(under,GA_ROOT);
        if (!(exactRoot ? under==hwnd : underRoot==hwnd)) {
            uint underPid; GetWindowThreadProcessId(underRoot,out underPid);
            var className=new StringBuilder(256); GetClassNameW(underRoot,className,className.Capacity);
            throw new InvalidOperationException("input point moved over a foreign window; x="+actual.x+"; y="+actual.y+"; target="+hwnd+"; under="+under+"; root="+underRoot+"; rootPid="+underPid+"; rootClass="+className);
        }
        return actual;
    }
    static void SendMove(int x,int y) {
        int left=GetSystemMetrics(76),top=GetSystemMetrics(77),width=GetSystemMetrics(78),height=GetSystemMetrics(79);
        Check(width>1&&height>1&&x>=left&&x<left+width&&y>=top&&y<top+height,"requested input point is outside the virtual desktop");
        INPUT input=new INPUT(); input.type=INPUT_MOUSE;
        input.U.mi=new MOUSEINPUT{dx=(int)Math.Round((x-left)*65535.0/(width-1)),dy=(int)Math.Round((y-top)*65535.0/(height-1)),dwFlags=0x0001|0x4000|0x8000};
        Check(SendInput(1,new[]{input},Marshal.SizeOf(typeof(INPUT)))==1,"SendInput MOVE failed; Win32="+Marshal.GetLastWin32Error());
        var sw=Stopwatch.StartNew();
        while(sw.ElapsedMilliseconds<500) { Validate(); Check(GetForegroundWindow()==hwnd,"foreground changed while awaiting cursor input"); POINT observed=ReadCursor(); if(observed.x==x&&observed.y==y)return; Thread.Sleep(2); }
        TraceInput("move-timeout",x,y);
        throw new InvalidOperationException("injected cursor motion did not reach the exact requested point");
    }
    static void MoveGradually(int x0,int y0,int x1,int y1) {
        for(int i=1;i<=DRAG_STEPS;i++) {
            Validate(); Check(GetForegroundWindow()==hwnd,"foreground changed during mouse input");
            int x=x0+(int)Math.Round((x1-x0)*(i/(double)DRAG_STEPS)), y=y0+(int)Math.Round((y1-y0)*(i/(double)DRAG_STEPS));
            TraceInput("move-"+i+"-before",x,y); SendMove(x,y); TraceInput("move-"+i+"-after",x,y); POINT actual=ReadCursor();
            Check(actual.x==x&&actual.y==y,"cursor was clipped during gradual movement");
            IntPtr under=WindowFromPoint(actual); Check(under!=IntPtr.Zero&&GetAncestor(under,GA_ROOT)==hwnd,"gradual mouse movement reached a foreign window"); Thread.Sleep(20);
        }
    }
    static bool DragAxisMatches(int requested,int observed) {
        if(requested==0)return observed==0;
        if(Math.Sign(observed)!=Math.Sign(requested))return false;
        // Chromium starts its native move loop on the first movement sample.
        // Permit at most that sample to be consumed, plus normal pixel rounding.
        int missing=Math.Abs(requested)-Math.Abs(observed);
        return missing>=-3 && missing<=(int)Math.Ceiling(Math.Abs(requested)/(double)DRAG_STEPS)+3;
    }
    static void WaitGeometry(RECT before,string action,int dx,int dy) {
        var sw=Stopwatch.StartNew(); RECT r;
        while(sw.ElapsedMilliseconds<3000) { Validate(); Check(GetWindowRect(hwnd,out r)!=0,"GetWindowRect failed after input");
            if(action=="Drag" && DragAxisMatches(dx,r.left-before.left) && DragAxisMatches(dy,r.top-before.top) && r.right-r.left==before.right-before.left && r.bottom-r.top==before.bottom-before.top) return;
            if(action=="Resize" && Math.Abs((r.right-before.right)-dx)<=3 && r.left==before.left && r.top==before.top && r.bottom==before.bottom) return;
            Thread.Sleep(40);
        }
        throw new InvalidOperationException(action+" geometry did not match the requested motion (drag allows one initial sample; resize allows 3 pixels)");
    }
    static int HitTest(int x,int y) { long p=((long)(ushort)x)|((long)(ushort)y<<16); UIntPtr result; Check(SendMessageTimeoutW(hwnd,WM_NCHITTEST,UIntPtr.Zero,new IntPtr(p),SMTO_BLOCK|SMTO_ABORTIFHUNG,1000,out result)!=IntPtr.Zero,"WM_NCHITTEST timed out"); return unchecked((int)result.ToUInt64()); }
    static bool CaptionPointMatches(int index,int x,int y) {
        Validate(); RECT bounds; Check(GetWindowRect(hwnd,out bounds)!=0,"caption point bounds could not be read");
        if(x<bounds.left||x>=bounds.right||y<bounds.top||y>=bounds.bottom)return false;
        POINT point=new POINT{x=x,y=y};
        IntPtr under=WindowFromPoint(point); uint underPid; GetWindowThreadProcessId(under,out underPid);
        if(under==IntPtr.Zero||GetAncestor(under,GA_ROOT)!=hwnd||underPid!=(uint)pid)return false;
        int expected=index==2?8:index==3?9:index==5?20:0;
        Check(expected!=0,"caption index is unsupported");
        return HitTest(x,y)==expected;
    }
    public static Dictionary<string,object> InspectCaptionPoint(int index,int x,int y) {
        IntPtr previous=SetThreadDpiAwarenessContext(new IntPtr(-4));
        Check(previous!=IntPtr.Zero,"could not set caption inspection DPI awareness");
        try {
            Validate(); POINT point=new POINT{x=x,y=y}; IntPtr under=WindowFromPoint(point),root=GetAncestor(under,GA_ROOT);
            uint underPid; GetWindowThreadProcessId(under,out underPid);
            var name=new StringBuilder(256); GetClassNameW(under,name,name.Capacity);
            return new Dictionary<string,object>{{"matched",CaptionPointMatches(index,x,y)},{"underHwnd",under.ToInt64()},{"underRootHwnd",root.ToInt64()},{"expectedRootHwnd",hwnd.ToInt64()},{"underPid",underPid},{"underClass",name.ToString()},{"hitTest",HitTest(x,y)}};
        } finally { Check(SetThreadDpiAwarenessContext(previous)!=IntPtr.Zero,"could not restore caption inspection DPI awareness"); }
    }
    public static bool MatchesCaptionPoint(int index,int x,int y) {
        IntPtr previous=SetThreadDpiAwarenessContext(new IntPtr(-4));
        Check(previous!=IntPtr.Zero,"could not set caption discovery DPI awareness");
        try { return CaptionPointMatches(index,x,y); }
        finally { Check(SetThreadDpiAwarenessContext(previous)!=IntPtr.Zero,"could not restore caption discovery DPI awareness"); }
    }
    static POINT CaptionCenter(int index) {
        TITLEBARINFOEX info=new TITLEBARINFOEX{cbSize=(uint)Marshal.SizeOf(typeof(TITLEBARINFOEX)),rgstate=new uint[6],rgrect=new RECT[6]};
        IntPtr p=Marshal.AllocHGlobal(Marshal.SizeOf(typeof(TITLEBARINFOEX)));
        try { Marshal.StructureToPtr(info,p,false); UIntPtr ignored; Check(SendMessageTimeoutW(hwnd,WM_GETTITLEBARINFOEX,UIntPtr.Zero,p,SMTO_BLOCK|SMTO_ABORTIFHUNG,1000,out ignored)!=IntPtr.Zero,"WM_GETTITLEBARINFOEX failed or timed out"); info=Marshal.PtrToStructure<TITLEBARINFOEX>(p); }
        finally { Marshal.FreeHGlobal(p); }
        RECT r=info.rgrect[index]; uint s=info.rgstate[index]; Check(r.right>r.left&&r.bottom>r.top,"caption button rectangle is empty"); Check((s&(STATE_UNAVAILABLE|STATE_INVISIBLE|STATE_OFFSCREEN))==0,"caption button is unavailable, invisible, or offscreen");
        POINT c=new POINT{x=r.left+(r.right-r.left)/2,y=r.top+(r.bottom-r.top)/2}; Check(WindowFromPoint(c)==hwnd&&GetAncestor(WindowFromPoint(c),GA_ROOT)==hwnd,"caption center is not owned root HWND"); return c;
    }
    static bool WaitState(string action,bool icon,bool zoom) { var sw=Stopwatch.StartNew(); while(sw.ElapsedMilliseconds<3000) { if(action=="Close") { if(IsWindow(hwnd)==0)return true; } else { Validate(); if((IsIconic(hwnd)!=0)==icon&&(IsZoomed(hwnd)!=0)==zoom)return true; } Thread.Sleep(40); } return false; }
    static Dictionary<string,object> Envelope(bool ok,string error,object before,object after,bool inputSent) { return new Dictionary<string,object>{{"ok",ok},{"before",before},{"after",after},{"inputSent",inputSent},{"error",error},{"inputTrace",inputTrace.ToArray()}}; }
    public static Dictionary<string,object> Run(int processId,string creationUtc,long windowHandle,string action,int px,int py,int dx,int dy,string inputJournalPath) {
        return RunInternal(processId,creationUtc,windowHandle,action,px,py,dx,dy,inputJournalPath,false,0,0);
    }
    public static Dictionary<string,object> RunWithCaptionPoint(int processId,string creationUtc,long windowHandle,string action,int px,int py,int dx,int dy,string inputJournalPath,int captionX,int captionY) {
        Check(action=="Minimize"||action=="Maximize"||action=="Restore"||action=="Close","a caption point is only valid for caption actions");
        return RunInternal(processId,creationUtc,windowHandle,action,px,py,dx,dy,inputJournalPath,true,captionX,captionY);
    }
    static Dictionary<string,object> RunInternal(int processId,string creationUtc,long windowHandle,string action,int px,int py,int dx,int dy,string inputJournalPath,bool useCaptionPoint,int captionX,int captionY) {
        inputTrace.Clear();
        Check(Environment.Is64BitProcess,"this input helper requires 64-bit PowerShell"); pid=processId; hwnd=new IntPtr(windowHandle); journalPath=inputJournalPath;
        DateTimeOffset parsed; Check(DateTimeOffset.TryParse(creationUtc,out parsed),"CreationUtc is invalid"); creationFileTime=parsed.UtcDateTime.ToFileTimeUtc();
        Check(!File.Exists(journalPath),"a preexisting mouse journal blocks normal actions");
        IntPtr oldDpi=IntPtr.Zero; bool changed=false; object beforeSnapshot=null,afterSnapshot=null; bool inputSent=false; string error=null; bool ok=false;
        try {
            oldDpi=SetThreadDpiAwarenessContext(new IntPtr(-4)); changed=oldDpi!=IntPtr.Zero; Check(changed,"could not set per-monitor-v2 thread DPI context");
            Validate(); beforeSnapshot=ReadSnapshot();
            if(action=="Snapshot") { afterSnapshot=beforeSnapshot; ok=true; }
            else if(action=="Activate") {
                Check(IsIconic(hwnd)==0,"Activate requires an already visible non-minimized window");
                RECT original,client; Check(GetWindowRect(hwnd,out original)!=0,"Activate outer geometry could not be read"); Check(GetClientRect(hwnd,out client)!=0,"Activate client geometry could not be read");
                POINT origin=new POINT{x=0,y=0}; Check(ClientToScreen(hwnd,ref origin)!=0,"Activate client origin failed");
                Check(px>=origin.x&&px<origin.x+client.right&&py>=origin.y&&py<origin.y+client.bottom,"Activate point must be in the owned client");
                Check(SetWindowPos(hwnd,IntPtr.Zero,0,0,0,0,0x0001|0x0002|0x0010)!=0,"could not raise the existing owned window without activation or geometry changes");
                POINT point=PositionAndVerify(new POINT{x=px,y=py},false,false); PressAt(point,false,false); inputSent=true; ClickUp(); Foreground();
                RECT current; Check(GetWindowRect(hwnd,out current)!=0&&current.left==original.left&&current.top==original.top&&current.right==original.right&&current.bottom==original.bottom,"Activate changed window geometry");
                afterSnapshot=ReadSnapshot(); ok=true;
            }
            else {
                bool wasIcon=IsIconic(hwnd)!=0, wasZoom=IsZoomed(hwnd)!=0;
                if(action=="Maximize") Check(!wasIcon&&!wasZoom,"Maximize requires an ordinary window");
                if(action=="Restore") Check(wasZoom&&!wasIcon,"Restore requires a maximized window");
                Foreground(); RECT before; Check(GetWindowRect(hwnd,out before)!=0,"GetWindowRect failed before input"); POINT start,end;
                if(action=="Drag") {
                    Check(px>=before.left&&px<before.right&&py>=before.top&&py<before.bottom,"Drag point is outside current outer bounds");
                    POINT requested=new POINT{x=px,y=py}; POINT verified=PositionAndVerify(requested,false); IntPtr under=WindowFromPoint(verified); Check(under!=IntPtr.Zero&&GetAncestor(under,GA_ROOT)==hwnd,"Drag point is not on an owned descendant");
                    end=new POINT{x=px+dx,y=py+dy}; Check(dx!=0||dy!=0,"Drag delta must move the window");
                    PressAt(verified,false); inputSent=true; MoveGradually(verified.x,verified.y,end.x,end.y); ClickUp(); WaitGeometry(before,action,dx,dy);
                } else if(action=="Resize") {
                    Check(dy==0&&dx!=0,"Resize requires nonzero DeltaX and DeltaY=0");
                    start=new POINT{x=before.right-1,y=before.top+(before.bottom-before.top)/2}; IntPtr under=WindowFromPoint(start);
                    Check(under!=IntPtr.Zero&&GetAncestor(under,GA_ROOT)==hwnd,"right border midpoint is not on an owned descendant"); Check(HitTest(start.x,start.y)==HTRIGHT,"right border midpoint did not hit-test as HTRIGHT");
                    end=new POINT{x=start.x+dx,y=start.y}; start=PositionAndVerify(start,false); Check(HitTest(start.x,start.y)==HTRIGHT,"right edge lost HTRIGHT hit-test before input"); TraceInput("resize-positioned",start.x,start.y); PressAt(start,false); inputSent=true; TraceInput("resize-pressed",start.x,start.y); MoveGradually(start.x,start.y,end.x,end.y); ClickUp(); TraceInput("resize-released",end.x,end.y); WaitGeometry(before,action,dx,0);
                } else {
                    int index=action=="Minimize"?2:action=="Close"?5:3; POINT c;
                    if(useCaptionPoint) { Check(CaptionPointMatches(index,captionX,captionY),"accessible caption point no longer matches the owned native control"); c=new POINT{x=captionX,y=captionY}; }
                    else c=CaptionCenter(index);
                    bool exactCaptionRoot=!useCaptionPoint;
                    c=PositionAndVerify(c,exactCaptionRoot); Check(CaptionPointMatches(index,c.x,c.y),"caption point lost its native hit-test before input"); PressAt(c,exactCaptionRoot,true,useCaptionPoint); inputSent=true; ClickUp();
                    bool expectedIcon=action=="Minimize"?true:false, expectedZoom=action=="Maximize"?true:action=="Restore"?false:wasZoom;
                    Check(WaitState(action,expectedIcon,expectedZoom),action+" state was not observed after real mouse input");
                }
                if(action=="Close") { Check(IsWindow(hwnd)==0,"Close input did not destroy original HWND"); afterSnapshot=null; }
                else afterSnapshot=ReadSnapshot(); ok=true;
            }
        } catch(Exception ex) { error=ex.GetType().Name+": "+ex.Message; try { if(IsWindow(hwnd)!=0)afterSnapshot=ReadSnapshot(); } catch {} }
        finally {
            if(buttonDown) {
                try { Send(LEFTUP); WaitLeftButtonUp(); buttonDown=false; RemoveVerifiedJournal(pid,hwnd.ToInt64(),Environment.ProcessId,journalPath); }
                catch(Exception ex) { ok=false; if(error==null)error="own mouse release or journal removal failed: "+ex.Message; }
            }
            if(action=="Resize"||action=="Drag")TraceInput("action-complete",0,0);
            if(changed) { try { if(SetThreadDpiAwarenessContext(oldDpi)==IntPtr.Zero) { ok=false; if(error==null)error="failed to restore thread DPI context"; } } catch(Exception ex) { ok=false; if(error==null)error="DPI restore failed: "+ex.Message; } }
        }
        return Envelope(ok,error,beforeSnapshot,afterSnapshot,inputSent);
    }
    static bool HelperTerminated(int helperPid) {
        try { using(Process p=Process.GetProcessById(helperPid)) return p.HasExited; }
        catch(ArgumentException) { return true; }
    }
    public static Dictionary<string,object> Recover(int expectedOwnerPid,long expectedHwnd,int expectedHelperPid,string journalPath) {
        Dictionary<string,object> observation=new Dictionary<string,object>{{"journalPath",journalPath},{"expectedOwnerPid",expectedOwnerPid},{"expectedHwnd",expectedHwnd},{"expectedHelperPid",expectedHelperPid},{"currentHelperPid",Environment.ProcessId},{"journalVerified",false},{"helperTerminated",false},{"leftButtonWasDown",null},{"recoveryLeftUpSent",false},{"recoveryButtonWasDown",false},{"recoveryButtonStateObserved",false},{"recoveryReleaseSent",false},{"leftButtonDownAfter",null},{"journalRemoved",false}};
        bool inputSent=false; string error=null; bool ok=false;
        try {
            Check(Environment.Is64BitProcess,"this input helper requires 64-bit PowerShell"); Check(expectedOwnerPid>0&&expectedHwnd>0&&expectedHelperPid>0,"recovery identities must be positive");
            Check(expectedHelperPid!=Environment.ProcessId,"recovery helper PID must differ from current helper PID");
            CheckJournal(expectedOwnerPid,expectedHwnd,expectedHelperPid,journalPath); observation["journalVerified"]=true;
            bool terminated=HelperTerminated(expectedHelperPid); observation["helperTerminated"]=terminated; Check(terminated,"recorded input helper is still running or its termination is uncertain");
            bool down=LeftButtonDown(); observation["leftButtonWasDown"]=down; observation["recoveryButtonWasDown"]=down; observation["recoveryButtonStateObserved"]=true;
            if(down) { Send(LEFTUP); inputSent=true; observation["recoveryLeftUpSent"]=true; observation["recoveryReleaseSent"]=true; WaitLeftButtonUp(); }
            bool downAfter=LeftButtonDown(); observation["leftButtonDownAfter"]=downAfter; Check(!downAfter,"left button remains down after recovery release");
            RemoveVerifiedJournal(expectedOwnerPid,expectedHwnd,expectedHelperPid,journalPath); observation["journalRemoved"]=true; ok=true;
        } catch(Exception ex) { error=ex.GetType().Name+": "+ex.Message; try { observation["leftButtonDownAfter"]=LeftButtonDown(); } catch {} }
        Dictionary<string,object> result=Envelope(ok,error,null,null,inputSent); result["recoveryButtonWasDown"]=observation["recoveryButtonWasDown"]; result["recoveryButtonStateObserved"]=observation["recoveryButtonStateObserved"]; result["recoveryReleaseSent"]=observation["recoveryReleaseSent"]; result["recovery"]=observation; return result;
    }
}
'@


function Find-AccessibleCaptionPoint {
    param([int]$Index, [Collections.IDictionary]$Evidence)
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    $providers = [Reflection.Assembly]::Load('UIAutomationClientSideProviders')
    Add-Type -ReferencedAssemblies @([System.Windows.Automation.AutomationElement].Assembly.Location, [System.Windows.Automation.ControlType].Assembly.Location, $providers.Location) -TypeDefinition @'
public static class CaptionAutomationProviders {
    public static void Register() {
        System.Windows.Automation.ClientSettings.RegisterClientSideProviders(UIAutomationClientsideProviders.UIAutomationClientSideProviders.ClientSideProviderDescriptionTable);
    }
}
'@
    [CaptionAutomationProviders]::Register()
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    $root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$WindowHandle)
    if ($root.Current.ProcessId -ne $AppProcessId) { throw 'Accessible root ownership changed' }
    $Evidence['root'] = @{processId=$root.Current.ProcessId;name=[string]$root.Current.Name;className=[string]$root.Current.ClassName}
    $Evidence['buttons'] = [Collections.Generic.List[object]]::new()
    $condition = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
    $buttons = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
    $Evidence['buttonCount'] = $buttons.Count
    if ($buttons.Count -gt 200) { throw 'Caption discovery has more than 200 accessible buttons' }
    $matches = @()
    foreach ($button in $buttons) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Accessible caption discovery exceeded 8 seconds' }
        $current = $button.Current
        $rect = $current.BoundingRectangle
        $observation = @{name=[string]$current.Name;processId=$current.ProcessId;offscreen=$current.IsOffscreen;enabled=$current.IsEnabled;bounds=@{x=$rect.X;y=$rect.Y;width=$rect.Width;height=$rect.Height};nativeMatch=$null}
        $Evidence['buttons'].Add($observation)
        if ($current.ProcessId -ne $AppProcessId -or $current.IsOffscreen -or -not $current.IsEnabled) { continue }
        if ($rect.IsEmpty -or $rect.Width -le 0 -or $rect.Height -le 0) { continue }
        $x = [int][Math]::Floor($rect.X + $rect.Width/2)
        $y = [int][Math]::Floor($rect.Y + $rect.Height/2)
        $observation['nativeInspection'] = [ExistingWindowInput]::InspectCaptionPoint($Index,$x,$y)
        $observation.nativeMatch = $observation['nativeInspection']['matched']
        if ($observation.nativeMatch) { $matches += ,$button }
    }
    if ($matches.Count -ne 1) { throw "Expected exactly one accessible owned native caption control; found $($matches.Count)" }
    if ([DateTime]::UtcNow -gt $deadline) { throw 'Accessible caption discovery exceeded 8 seconds' }
    $current = $matches[0].Current
    if ($current.ProcessId -ne $AppProcessId -or $current.IsOffscreen -or -not $current.IsEnabled) { throw 'Accessible caption control changed before input' }
    $rect = $current.BoundingRectangle
    if ($rect.IsEmpty -or $rect.Width -le 0 -or $rect.Height -le 0) { throw 'Accessible caption control no longer has bounds' }
    $x = [int][Math]::Floor($rect.X + $rect.Width/2)
    $y = [int][Math]::Floor($rect.Y + $rect.Height/2)
    if (-not [ExistingWindowInput]::MatchesCaptionPoint($Index,$x,$y)) { throw 'Accessible caption point no longer matches the owned native control' }
    return @{ x=$x; y=$y; name=[string]$current.Name; bounds=@{x=$rect.X;y=$rect.Y;width=$rect.Width;height=$rect.Height}; source='UIAutomation'; nativeHitTest=$(if($Index -eq 2){8}elseif($Index -eq 3){9}else{20}) }
}

$result = [ordered]@{ ok=$false; action=$Action; appProcessId=$AppProcessId; hwnd=$WindowHandle; before=$null; after=$null; inputSent=$false; error=$null; inputTrace=@() }
$canWrite = $false
try {
    if (-not [IO.Path]::IsPathRooted($OutputFile)) { throw 'OutputFile must be an absolute path' }
    $fullOutput = [IO.Path]::GetFullPath($OutputFile)
    $parent = [IO.Path]::GetDirectoryName($fullOutput)
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) { throw 'OutputFile parent directory does not exist' }
    if (Test-Path -LiteralPath $fullOutput) { throw 'OutputFile already exists' }
    $canWrite = $true
    if ($AppProcessId -le 0) { throw 'AppProcessId must be a positive 32-bit integer' }
    if ($WindowHandle -le 0) { throw 'WindowHandle must be a positive 64-bit integer' }
    if ($DeltaX -lt -120 -or $DeltaX -gt 120 -or $DeltaY -lt -120 -or $DeltaY -gt 120) { throw 'DeltaX and DeltaY must be within -120..120' }
    if (@('Snapshot','Activate','Drag','Resize','Minimize','Maximize','Restore','Close','RecoverMouse') -cnotcontains $Action) { throw 'Action is unsupported' }
    if (-not $IsWindows) { throw 'This helper requires Windows PowerShell 7+' }
    if (-not [Environment]::Is64BitProcess) { throw 'This input helper requires 64-bit PowerShell' }
    if ($Action -eq 'RecoverMouse') {
        if (-not $PSBoundParameters.ContainsKey('InputJournal') -or -not $PSBoundParameters.ContainsKey('InputHelperPid')) { throw 'RecoverMouse requires InputJournal and InputHelperPid' }
        if ($PSBoundParameters.ContainsKey('CreationUtc')) { throw 'CreationUtc must be omitted for RecoverMouse' }
        if (-not [IO.Path]::IsPathRooted($InputJournal)) { throw 'InputJournal must be an absolute path' }
        $fullJournal = [IO.Path]::GetFullPath($InputJournal)
        if (-not [string]::Equals([IO.Path]::GetDirectoryName($fullJournal),$parent,[StringComparison]::OrdinalIgnoreCase)) { throw 'InputJournal must be in the OutputFile parent directory' }
        Add-Type -TypeDefinition $script -Language CSharp -ErrorAction Stop
        $native = [ExistingWindowInput]::Recover($AppProcessId,$WindowHandle,$InputHelperPid,$fullJournal)
        foreach ($key in @('ok','before','after','inputSent','error','inputTrace','recoveryButtonWasDown','recoveryButtonStateObserved','recoveryReleaseSent','recovery')) { $result[$key]=$native[$key] }
    } else {
        if ($PSBoundParameters.ContainsKey('InputJournal') -or $PSBoundParameters.ContainsKey('InputHelperPid')) { throw 'InputJournal and InputHelperPid are recovery-only parameters' }
        if ([string]::IsNullOrWhiteSpace($CreationUtc)) { throw 'CreationUtc is required for normal actions' }
        $journalPath = $fullOutput + '.mouse-down'
        $existingJournals = @(Get-ChildItem -LiteralPath $parent -Filter '*.mouse-down' -File -ErrorAction Stop)
        if ($existingJournals.Count -gt 0) { throw 'A preexisting mouse journal blocks normal actions' }
        if (Test-Path -LiteralPath $journalPath) { throw 'A preexisting mouse journal blocks normal actions' }
        $proc = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = $AppProcessId" -ErrorAction Stop
        if ($null -eq $proc) { throw 'Owned process does not exist' }
        $actualCreation = $proc.CreationDate.ToUniversalTime().ToString('o')
        if ($actualCreation -cne $CreationUtc) { throw 'CreationUtc does not match the owned process identity' }
        Add-Type -TypeDefinition $script -Language CSharp -ErrorAction Stop
        $native = [ExistingWindowInput]::Run($AppProcessId,$CreationUtc,$WindowHandle,$Action,$PointX,$PointY,$DeltaX,$DeltaY,$journalPath)
        if (-not $native['ok'] -and -not $native['inputSent'] -and $native['error'] -ceq 'InvalidOperationException: caption button rectangle is empty' -and @('Minimize','Maximize','Restore','Close') -ccontains $Action) {
            if (Test-Path -LiteralPath $journalPath) { throw 'Caption discovery cannot proceed with an existing mouse journal' }
            $index = if($Action -eq 'Minimize'){2}elseif($Action -eq 'Close'){5}else{3}
            $result['captionFallbackInitial'] = $native
            $inspection = @{}
            $result['captionAccessibilityInspection'] = $inspection
            $point = Find-AccessibleCaptionPoint -Index $index -Evidence $inspection
            $result['captionAccessibility'] = $point
            $native = [ExistingWindowInput]::RunWithCaptionPoint($AppProcessId,$CreationUtc,$WindowHandle,$Action,$PointX,$PointY,$DeltaX,$DeltaY,$journalPath,$point.x,$point.y)
        }
        foreach ($key in @('ok','before','after','inputSent','error','inputTrace')) { $result[$key]=$native[$key] }
    }
} catch {
    $result.error = $_.Exception.GetType().Name + ': ' + $_.Exception.Message
}
if ($canWrite) {
    $json = $result | ConvertTo-Json -Depth 10
    $stream = $null
    try {
        $stream = [IO.File]::Open($fullOutput,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
        $writer = [IO.StreamWriter]::new($stream,[Text.UTF8Encoding]::new($false))
        $writer.WriteLine($json); $writer.Flush(); $stream.Flush($true); $writer.Dispose(); $stream=$null
    } catch {
        if ($null -ne $stream) { $stream.Dispose() }
        [Console]::Error.WriteLine('Could not create fresh evidence file: ' + $_.Exception.Message)
        exit 1
    }
}
[Console]::WriteLine(($result | ConvertTo-Json -Depth 10 -Compress))
if ($result.ok) { exit 0 } else { exit 1 }
