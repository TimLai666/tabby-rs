[CmdletBinding()]
param(
    [string]$OutputDirectory = (Get-Location).Path,
    [switch]$Help,
    [switch]$TaoBlur
)

if ($Help) {
    @'
probe-native-custom-caption.ps1 [-OutputDirectory <directory>] [-TaoBlur] [-Help]

Custom-frame diagnostic. -TaoBlur additionally applies the pinned Tao transparency call.
This uses pure Win32 and no Wry child. FullAppAccepted is always false.

Runs a bounded native Windows caption-button input probe. On a non-Windows
host, or with -Help, it exits without creating a window. A successful run
creates result.json in OutputDirectory and may briefly minimize, maximize,
restore, and close one uniquely titled test window. It sends real cursor and
mouse input only to freshly checked native caption-button centers on that
window. It does not automate a full application, use UI Automation, or prove
application-level parity. Hit-test return codes are diagnostic evidence only.
The result always has FullAppAccepted=false. This probe requires Windows and
PowerShell 7; use an output directory that already exists.
'@
    exit 0
}

if (-not $IsWindows) {
    [Console]::Error.WriteLine('This probe requires Windows; no window was created.')
    exit 2
}
if (-not (Test-Path -LiteralPath $OutputDirectory -PathType Container)) {
    [Console]::Error.WriteLine("Output directory does not exist: $OutputDirectory")
    exit 2
}

$source = @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Threading;

public static class NativeCaptionProbe {
    const uint WS_OVERLAPPEDWINDOW = 0x00CF0000;
    const uint WS_VISIBLE = 0x10000000;
    const uint WM_DESTROY = 0x0002;
    const uint WM_NCHITTEST = 0x0084;
    const uint WM_APP = 0x8000;
    const uint WM_NCCALCSIZE = 0x0083;
    const uint WM_ACTIVATE = 0x0006;
    const int SM_CYCAPTION = 4;
    const uint WM_GETTITLEBARINFOEX = 0x033F;
    const uint PM_REMOVE = 0x0001;
    const uint SMTO_BLOCK = 0x0001;
    const uint SMTO_ABORTIFHUNG = 0x0002;
    const uint INPUT_MOUSE = 0;
    const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
    const uint MOUSEEVENTF_LEFTUP = 0x0004;
    const uint STATE_SYSTEM_UNAVAILABLE = 0x00000001;
    const uint STATE_SYSTEM_INVISIBLE = 0x00008000;
    const uint STATE_SYSTEM_OFFSCREEN = 0x00010000;
    const int SW_RESTORE = 9;
    const int TITLEBAR_BUTTON_MINIMIZE = 2;
    const int TITLEBAR_BUTTON_MAXIMIZE_RESTORE = 3;
    const int TITLEBAR_BUTTON_CLOSE = 5;
    const int QUERY_NCHITTEST = 1;
    const int THREAD_DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4;
    static readonly int TotalLimitMs = 30000;
    static readonly int ActionLimitMs = 28000;

    [StructLayout(LayoutKind.Sequential)] struct POINT { public int x, y; }
    [StructLayout(LayoutKind.Sequential)] struct MSG { public IntPtr hwnd; public uint message; public UIntPtr wParam; public IntPtr lParam; public uint time; public POINT pt; }
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] struct WNDCLASSEX {
        public uint cbSize, style; public WndProc lpfnWndProc; public int cbClsExtra, cbWndExtra;
        public IntPtr hInstance, hIcon, hCursor, hbrBackground; public string lpszMenuName, lpszClassName; public IntPtr hIconSm;
    }
    delegate IntPtr WndProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);
    [StructLayout(LayoutKind.Sequential)] struct RECT { public int left, top, right, bottom; }
    [StructLayout(LayoutKind.Sequential)] struct WINDOWPOS { public IntPtr hwnd, after; public int x, y, width, height; public uint flags; }
    [StructLayout(LayoutKind.Sequential)] struct MONITORINFO { public uint size; public RECT monitor, work; public uint flags; }

    [StructLayout(LayoutKind.Sequential)] struct MARGINS { public int left, right, top, bottom; }
    [StructLayout(LayoutKind.Sequential)] struct DWM_BLURBEHIND { public uint flags; public int enable; public IntPtr region; public int transition; }
    [StructLayout(LayoutKind.Sequential)] struct TITLEBARINFOEX {
        public uint cbSize; public RECT rcTitleBar;
        [MarshalAs(UnmanagedType.ByValArray, SizeConst=6)] public uint[] rgstate;
        [MarshalAs(UnmanagedType.ByValArray, SizeConst=6)] public RECT[] rgrect;
    }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public UIntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit, Size=32)] struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mi; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public INPUTUNION U; }
    sealed class QueryData { public int x, y; public IntPtr defResult; public IntPtr dwmResult; public int dwmHandled; }

    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr GetModuleHandle(string moduleName);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern ushort RegisterClassEx(ref WNDCLASSEX wc);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern int UnregisterClass(string name, IntPtr instance);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateWindowEx(uint exStyle, string className, string title, uint style, int x, int y, int width, int height, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr param);
    [DllImport("user32.dll", SetLastError=true)] static extern int DestroyWindow(IntPtr hwnd);
    [DllImport("user32.dll", EntryPoint="DefWindowProcW", CharSet=CharSet.Unicode)] static extern IntPtr DefWindowProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll", EntryPoint="PeekMessageW", CharSet=CharSet.Unicode, SetLastError=true)] static extern int PeekMessage(out MSG msg, IntPtr hwnd, uint min, uint max, uint remove);
    [DllImport("user32.dll")] static extern int TranslateMessage(ref MSG msg);
    [DllImport("user32.dll", EntryPoint="DispatchMessageW", CharSet=CharSet.Unicode)] static extern IntPtr DispatchMessage(ref MSG msg);
    [DllImport("user32.dll")] static extern void PostQuitMessage(int code);
    [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint msg, UIntPtr wParam, IntPtr lParam, uint flags, uint timeout, out UIntPtr result);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT point);
    [DllImport("user32.dll")] static extern int SetCursorPos(int x, int y);
    [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint count, INPUT[] inputs, int size);
    [DllImport("user32.dll")] static extern int IsWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern int IsIconic(IntPtr hwnd);
    [DllImport("user32.dll")] static extern int IsZoomed(IntPtr hwnd);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern int SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll", SetLastError=true)] static extern int ShowWindow(IntPtr hwnd, int command);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern IntPtr GetThreadDpiAwarenessContext();
    [DllImport("user32.dll", SetLastError=true)] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern int GetAwarenessFromDpiAwarenessContext(IntPtr context);
    [DllImport("dwmapi.dll")] static extern int DwmDefWindowProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam, out IntPtr result);
    [DllImport("dwmapi.dll")] static extern int DwmExtendFrameIntoClientArea(IntPtr hwnd, ref MARGINS margins);
    [DllImport("user32.dll")] static extern int GetSystemMetricsForDpi(int index, uint dpi);
    [DllImport("user32.dll", SetLastError=true)] static extern int SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int width, int height, uint flags);


    [DllImport("dwmapi.dll")] static extern int DwmEnableBlurBehindWindow(IntPtr hwnd, ref DWM_BLURBEHIND blur);
    [DllImport("gdi32.dll")] static extern IntPtr CreateRectRgn(int left, int top, int right, int bottom);
    [DllImport("gdi32.dll")] static extern int DeleteObject(IntPtr obj);

    [DllImport("user32.dll")] static extern IntPtr MonitorFromRect(ref RECT rect, uint flags);
    [DllImport("user32.dll", EntryPoint="GetMonitorInfoW")] static extern int GetMonitorInfo(IntPtr monitor, ref MONITORINFO info);
    [DllImport("user32.dll")] static extern int GetWindowRect(IntPtr h, out RECT rect);
    [DllImport("user32.dll")] static extern int ClientToScreen(IntPtr h, ref POINT point);

    static readonly object gate = new object();
    static IntPtr hwnd;
    static string title;
    static string className;
    static IntPtr module;
    static WndProc proc;
    static QueryData query;

    static IntPtr PackPoint(int x, int y) { return new IntPtr(unchecked((int)((uint)(ushort)x | ((uint)(ushort)y << 16)))); }
    static Dictionary<string, object> Dict(params object[] kv) { var d = new Dictionary<string, object>(); for(int i=0;i<kv.Length;i+=2) d[(string)kv[i]]=kv[i+1]; return d; }
    static IntPtr WndProcImpl(IntPtr h, uint msg, IntPtr wp, IntPtr lp) {
        if(msg == WM_APP + QUERY_NCHITTEST) {
            QueryData q; lock(gate) q=query;
            if(q == null) return IntPtr.Zero;
            IntPtr packed=PackPoint(q.x,q.y);
            IntPtr dwm;
            int handled=DwmDefWindowProc(h, WM_NCHITTEST, IntPtr.Zero, packed, out dwm);
            IntPtr def=DefWindowProc(h, WM_NCHITTEST, IntPtr.Zero, packed);
            q.dwmHandled=handled; q.dwmResult=dwm; q.defResult=def;
            return IntPtr.Zero;
        }
        if(msg == WM_DESTROY) { PostQuitMessage(0); return IntPtr.Zero; }
        IntPtr dwmResult;
        bool dwmHandled=DwmDefWindowProc(h,msg,wp,lp,out dwmResult)!=0;
        if(msg == WM_NCCALCSIZE && lp != IntPtr.Zero) {
            RECT before=Marshal.PtrToStructure<RECT>(lp);
            IntPtr result=DefWindowProc(h,msg,wp,lp);
            RECT after=Marshal.PtrToStructure<RECT>(lp);
            uint dpi=GetDpiForWindow(h);
            int caption=dpi==0 ? 0 : GetSystemMetricsForDpi(SM_CYCAPTION,dpi);
            int nativeTop=after.top;
            after.top=before.top; // Diagnostic: no top non-client inset, including maximized.
            Marshal.StructureToPtr(after,lp,false);
            Console.Error.WriteLine("pure-custom-frame NCCALCSIZE zoom={0} outerTop={1} nativeTop={2} caption={3} finalTop={4}",IsZoomed(h)!=0,before.top,nativeTop,caption,after.top);
            return result;
        }
        IntPtr answer=dwmHandled ? dwmResult : DefWindowProc(h,msg,wp,lp);
        if(msg == 0x0046 && lp != IntPtr.Zero && IsZoomed(h)!=0) {
            WINDOWPOS pos=Marshal.PtrToStructure<WINDOWPOS>(lp);
            if((pos.flags & 3)==0) {
                RECT proposed=new RECT{left=pos.x,top=pos.y,right=pos.x+pos.width,bottom=pos.y+pos.height};
                IntPtr monitor=MonitorFromRect(ref proposed,2);
                MONITORINFO info=new MONITORINFO{size=(uint)Marshal.SizeOf<MONITORINFO>()};
                if(monitor!=IntPtr.Zero && GetMonitorInfo(monitor,ref info)!=0) {
                    long delta=(long)info.work.top-pos.y;
                    if(delta>0 && delta<pos.height) {
                        Console.Error.WriteLine("pure-custom-frame MAXPOSITION oldY={0} newY={1} oldHeight={2} delta={3}",pos.y,info.work.top,pos.height,delta);
                        pos.y=info.work.top;pos.height-=(int)delta;
                        Marshal.StructureToPtr(pos,lp,false);
                    }
                }
            }
        }
        if(msg == WM_ACTIVATE) {
            uint dpi=GetDpiForWindow(h);
            MARGINS margins=new MARGINS{top=dpi==0 ? 0 : GetSystemMetricsForDpi(SM_CYCAPTION,dpi)};
            int hr=DwmExtendFrameIntoClientArea(h,ref margins);
            Console.Error.WriteLine("pure-custom-frame extend hr={0} dpi={1}",hr,dpi);
        }
        if(msg == WM_NCHITTEST) Console.Error.WriteLine("pure-custom-frame HIT zoom={0} dwm={1} dwmResult={2} final={3}",IsZoomed(h)!=0,dwmHandled,dwmResult,answer);
        return answer;
    }

    static int Remaining(Stopwatch total, int requested) {
        long remaining=ActionLimitMs-total.ElapsedMilliseconds;
        if(remaining<=0) throw new TimeoutException("30 second total action bound reached");
        return (int)Math.Min((long)requested,remaining);
    }
    static Dictionary<string, object> GetFreshCenter(string action, int index, Stopwatch total) {
        if(IsWindow(hwnd)==0) throw new Exception(action + ": owned window no longer exists before action");
        TITLEBARINFOEX info = new TITLEBARINFOEX { cbSize=(uint)Marshal.SizeOf(typeof(TITLEBARINFOEX)), rgstate=new uint[6], rgrect=new RECT[6] };
        IntPtr p=Marshal.AllocHGlobal(Marshal.SizeOf(typeof(TITLEBARINFOEX)));
        try {
            Marshal.StructureToPtr(info,p,false);
            UIntPtr ignored;
            IntPtr sent=SendMessageTimeout(hwnd,WM_GETTITLEBARINFOEX,UIntPtr.Zero,p,SMTO_BLOCK|SMTO_ABORTIFHUNG,(uint)Remaining(total,1000),out ignored);
            if(sent==IntPtr.Zero) throw new Exception(action + ": WM_GETTITLEBARINFOEX timed out or failed");
            info=(TITLEBARINFOEX)Marshal.PtrToStructure(p,typeof(TITLEBARINFOEX));
        } finally { Marshal.FreeHGlobal(p); }
        RECT r=info.rgrect[index]; uint state=info.rgstate[index];
        if(r.right<=r.left || r.bottom<=r.top) throw new Exception(action + ": caption button rectangle is empty");
        if((state & STATE_SYSTEM_UNAVAILABLE)!=0) throw new Exception(action + ": caption button is unavailable");
        if((state & STATE_SYSTEM_INVISIBLE)!=0) throw new Exception(action + ": caption button is invisible");
        if((state & STATE_SYSTEM_OFFSCREEN)!=0) throw new Exception(action + ": caption button is offscreen");
        int x=r.left+(r.right-r.left)/2, y=r.top+(r.bottom-r.top)/2;
        POINT point=new POINT{x=x,y=y}; IntPtr under=WindowFromPoint(point);
        if(under!=hwnd) throw new Exception(action + ": WindowFromPoint at fresh button center did not resolve to owned HWND");
        return Dict("x",x,"y",y,"rect",Dict("left",r.left,"top",r.top,"right",r.right,"bottom",r.bottom),"state",state,"windowFromPointOwned",true);
    }
    static Dictionary<string, object> QueryHitTest(string action, int x, int y, Stopwatch total) {
        IntPtr packed=PackPoint(x,y); UIntPtr result;
        IntPtr sent=SendMessageTimeout(hwnd,WM_NCHITTEST,UIntPtr.Zero,packed,SMTO_BLOCK|SMTO_ABORTIFHUNG,(uint)Remaining(total,1000),out result);
        if(sent==IntPtr.Zero) throw new Exception(action + ": external WM_NCHITTEST query timed out or failed");
        QueryData q=new QueryData{x=x,y=y}; lock(gate) query=q;
        UIntPtr ignored;
        IntPtr uiSent=SendMessageTimeout(hwnd,WM_APP+QUERY_NCHITTEST,UIntPtr.Zero,IntPtr.Zero,SMTO_BLOCK|SMTO_ABORTIFHUNG,(uint)Remaining(total,1000),out ignored);
        lock(gate) query=null;
        if(uiSent==IntPtr.Zero) throw new Exception(action + ": same-UI-thread hit-test diagnostics timed out or failed");
        return Dict("externalSendMessageTimeoutResult",result.ToUInt64(),"sameUiThreadDefWindowProcResult",q.defResult.ToInt64(),"sameUiThreadDwmDefWindowProcHandled",q.dwmHandled!=0,"sameUiThreadDwmDefWindowProcResult",q.dwmResult.ToInt64(),"note","Diagnostic only; hit-test codes are not input pass criteria.");
    }
    static void RealClick(string action, int x, int y) {
        if(SetCursorPos(x,y)==0) throw new Exception(action + ": SetCursorPos failed, Win32="+Marshal.GetLastWin32Error());
        INPUT[] a=new INPUT[2];
        a[0].type=INPUT_MOUSE; a[0].U.mi=new MOUSEINPUT{dx=0,dy=0,mouseData=0,dwFlags=MOUSEEVENTF_LEFTDOWN,time=0,dwExtraInfo=UIntPtr.Zero};
        a[1].type=INPUT_MOUSE; a[1].U.mi=new MOUSEINPUT{dx=0,dy=0,mouseData=0,dwFlags=MOUSEEVENTF_LEFTUP,time=0,dwExtraInfo=UIntPtr.Zero};
        uint sent=SendInput(2,a,Marshal.SizeOf(typeof(INPUT)));
        if(sent!=2) {
            if(sent==1) {
                INPUT[] release=new INPUT[1]; release[0].type=INPUT_MOUSE;
                release[0].U.mi=new MOUSEINPUT{dx=0,dy=0,mouseData=0,dwFlags=MOUSEEVENTF_LEFTUP,time=0,dwExtraInfo=UIntPtr.Zero};
                SendInput(1,release,Marshal.SizeOf(typeof(INPUT)));
            }
            throw new Exception(action + ": SendInput accepted "+sent+" of 2 events, Win32="+Marshal.GetLastWin32Error());
        }
    }
    static void WaitState(string action, Func<bool> predicate, bool expected, int timeoutMs, Stopwatch total) {
        timeoutMs=Remaining(total,timeoutMs);
        var sw=Stopwatch.StartNew();
        while(sw.ElapsedMilliseconds<timeoutMs) {
            if(IsWindow(hwnd)==0 && action!="close") throw new Exception(action + ": owned window was destroyed unexpectedly");
            if(predicate()==expected) return;
            Thread.Sleep(50);
        }
        throw new Exception(action + ": expected window state was not observed within "+timeoutMs+" ms");
    }
    static void WaitForeground(string action, int timeoutMs, Stopwatch total) {
        timeoutMs=Remaining(total,timeoutMs);
        SetForegroundWindow(hwnd);
        var sw=Stopwatch.StartNew();
        while(sw.ElapsedMilliseconds<timeoutMs) {
            if(GetForegroundWindow()==hwnd) return;
            Thread.Sleep(50);
        }
        throw new Exception(action + ": owned window did not become foreground within "+timeoutMs+" ms");
    }
    static Dictionary<string, object> ActionClick(List<object> actions, string name, int index, bool wantIconic, bool wantZoomed, Stopwatch total) {
        var actionStarted=Stopwatch.GetTimestamp();
        WaitForeground(name,1800,total);
        var center=GetFreshCenter(name,index,total);
        var record=Dict("action",name,"buttonCenter",center,"hitTestDiagnostics",null,"sendInputEventsAccepted",0,"inputAttempted",false,"observedWindowExists",IsWindow(hwnd)!=0,"observedIsIconic",IsWindow(hwnd)!=0 && IsIconic(hwnd)!=0,"observedIsZoomed",IsWindow(hwnd)!=0 && IsZoomed(hwnd)!=0,"observedWindowStatePassed",false,"error",null);
        actions.Add(record);
        try {
            var diagnostic=QueryHitTest(name,(int)center["x"],(int)center["y"],total);
            record["hitTestDiagnostics"]=diagnostic;
            record["inputAttempted"]=true;
            RealClick(name,(int)center["x"],(int)center["y"]);
            record["sendInputEventsAccepted"]=2;
            if(name=="close") WaitState("close",()=>IsWindow(hwnd)==0,true,3000,total);
            else {
                WaitState(name+" state",()=>IsIconic(hwnd)!=0,wantIconic,3000,total);
                WaitState(name+" zoom state",()=>IsZoomed(hwnd)!=0,wantZoomed,1000,total);
            }
            record["observedWindowExists"]=IsWindow(hwnd)!=0;
            record["observedIsIconic"]=IsWindow(hwnd)!=0 && IsIconic(hwnd)!=0;
            record["observedIsZoomed"]=IsWindow(hwnd)!=0 && IsZoomed(hwnd)!=0;
            if(wantZoomed && IsWindow(hwnd)!=0) {
                RECT outer; POINT origin=new POINT();
                if(GetWindowRect(hwnd,out outer)==0 || ClientToScreen(hwnd,ref origin)==0) throw new Exception("Maximized geometry readback failed");
                IntPtr monitor=MonitorFromRect(ref outer,2);
                MONITORINFO info=new MONITORINFO{size=(uint)Marshal.SizeOf<MONITORINFO>()};
                if(monitor==IntPtr.Zero || GetMonitorInfo(monitor,ref info)==0) throw new Exception("Maximized monitor readback failed");
                record["maximizedClientTop"]=origin.y;record["monitorWorkTop"]=info.work.top;
                if(origin.y!=info.work.top) throw new Exception("Maximized client top does not match work area");
            }
            record["observedWindowStatePassed"]=true;
            record["elapsedMs"]=(long)((Stopwatch.GetTimestamp()-actionStarted)*1000.0/Stopwatch.Frequency);
            return record;
        } catch(Exception ex) {
            record["observedWindowExists"]=IsWindow(hwnd)!=0;
            record["observedIsIconic"]=IsWindow(hwnd)!=0 && IsIconic(hwnd)!=0;
            record["observedIsZoomed"]=IsWindow(hwnd)!=0 && IsZoomed(hwnd)!=0;
            record["error"]=ex.GetType().Name+": "+ex.Message;
            record["elapsedMs"]=(long)((Stopwatch.GetTimestamp()-actionStarted)*1000.0/Stopwatch.Frequency);
            throw;
        }
    }
    static void PumpUntil(ManualResetEvent done, int timeoutMs) {
        var sw=Stopwatch.StartNew();
        while(!done.WaitOne(0) && sw.ElapsedMilliseconds<timeoutMs) {
            MSG msg;
            while(PeekMessage(out msg,IntPtr.Zero,0,0,PM_REMOVE)!=0) { TranslateMessage(ref msg); DispatchMessage(ref msg); }
            Thread.Sleep(4);
        }
        if(!done.WaitOne(0)) throw new TimeoutException("UI pump exceeded total 30 second bound");
    }

    public static string Run(string outputDirectory, bool taoBlur) {
        var started=Stopwatch.StartNew(); var actions=new List<object>(); var errors=new List<string>();
        var checks=new List<Dictionary<string,object>>(); var metadata=Dict("hostIsWindows",true,"powershellVersionRequirement","7+","FullAppAccepted",false);
        var done=new ManualResetEvent(false); Exception workerError=null; bool registered=false; bool created=false;
        IntPtr originalDpi=IntPtr.Zero; bool dpiChanged=false; uint uiThread=0; uint ownerPid=0;
        className="CodexNativeCaptionProbe_"+Guid.NewGuid().ToString("N"); title="Codex native caption probe "+Guid.NewGuid().ToString("N"); module=GetModuleHandle(null);
        if(module==IntPtr.Zero) throw new Exception("GetModuleHandleW failed, Win32="+Marshal.GetLastWin32Error());
        try {
            IntPtr requested=new IntPtr(THREAD_DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            originalDpi=SetThreadDpiAwarenessContext(requested); dpiChanged=originalDpi!=IntPtr.Zero;
            IntPtr active=GetThreadDpiAwarenessContext();
            metadata["uiThreadDpiAwarenessContext"]="0x"+active.ToInt64().ToString("X");
            try { metadata["uiThreadDpiAwareness"] = GetAwarenessFromDpiAwarenessContext(active); } catch { metadata["uiThreadDpiAwareness"]="unavailable"; }
            metadata["perMonitorV2ContextSet"] = dpiChanged;
            if(!dpiChanged) metadata["dpiContextSetError"] = Marshal.GetLastWin32Error();
            proc=new WndProc(WndProcImpl);
            WNDCLASSEX wc=new WNDCLASSEX{cbSize=(uint)Marshal.SizeOf(typeof(WNDCLASSEX)),lpfnWndProc=proc,hInstance=module,lpszClassName=className};
            if(RegisterClassEx(ref wc)==0) throw new Exception("RegisterClassEx failed, Win32="+Marshal.GetLastWin32Error()); registered=true;
            hwnd=CreateWindowEx(0,className,title,WS_OVERLAPPEDWINDOW,32,32,640,480,IntPtr.Zero,IntPtr.Zero,module,IntPtr.Zero);
            if(hwnd==IntPtr.Zero) throw new Exception("CreateWindowEx failed, Win32="+Marshal.GetLastWin32Error()); created=true;
            metadata["taoBlur"] = taoBlur;
            if(taoBlur) {
                IntPtr region=CreateRectRgn(0,0,-1,-1);
                if(region==IntPtr.Zero) throw new Exception("Tao blur region allocation failed");
                try {
                    DWM_BLURBEHIND blur=new DWM_BLURBEHIND{flags=3,enable=1,region=region,transition=0};
                    int hr=DwmEnableBlurBehindWindow(hwnd,ref blur);
                    metadata["taoBlurHResult"]=hr;
                    if(hr<0) throw new Exception("Tao blur DWM call failed: "+hr);
                } finally { DeleteObject(region); }
            }
            MARGINS customMargins=new MARGINS{top=GetSystemMetricsForDpi(SM_CYCAPTION,GetDpiForWindow(hwnd))};
            if(customMargins.top<=0 || DwmExtendFrameIntoClientArea(hwnd,ref customMargins)<0) throw new Exception("Pure custom DWM extension failed");
            if(SetWindowPos(hwnd,IntPtr.Zero,0,0,0,0,0x37)==0) throw new Exception("Pure custom frame-change failed");
            metadata["scope"]="Diagnostic maximized outer top aligned with work area and zero top inset; no Wry child or product backend";
            ShowWindow(hwnd,5);

            uiThread=GetWindowThreadProcessId(hwnd,out ownerPid);
            metadata["windowTitle"]=title; metadata["windowOuterBoundsRequested"] = Dict("x",32,"y",32,"width",640,"height",480);
            metadata["windowDpi"] = GetDpiForWindow(hwnd); metadata["windowThreadId"]=uiThread; metadata["windowProcessId"]=ownerPid;
            var worker=new Thread(()=>{
                IntPtr workerOriginalDpi=IntPtr.Zero; bool workerDpiChanged=false;
                try {
                    try {
                        workerOriginalDpi=SetThreadDpiAwarenessContext(new IntPtr(THREAD_DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2));
                        workerDpiChanged=workerOriginalDpi!=IntPtr.Zero;
                        IntPtr workerActiveDpi=GetThreadDpiAwarenessContext();
                        lock(gate) {
                            metadata["workerThreadDpiAwarenessContext"]="0x"+workerActiveDpi.ToInt64().ToString("X");
                            metadata["workerThreadDpiAwareness"] = workerActiveDpi==IntPtr.Zero ? (object)"unavailable" : GetAwarenessFromDpiAwarenessContext(workerActiveDpi);
                            metadata["workerPerMonitorV2ContextSet"] = workerDpiChanged;
                            if(!workerDpiChanged) metadata["workerDpiContextSetError"] = Marshal.GetLastWin32Error();
                        }
                    } catch(Exception dpiEx) { lock(gate) metadata["workerDpiContextError"]=dpiEx.GetType().Name+": "+dpiEx.Message; }
                    var t=Stopwatch.StartNew();
                    ActionClick(actions,"minimize",TITLEBAR_BUTTON_MINIMIZE,true,false,t); checks.Add(Dict("check","real native minimize click and IsIconic observation","passed",true));
                    if(t.ElapsedMilliseconds>ActionLimitMs) throw new TimeoutException("total action bound reached");
                    ShowWindow(hwnd,SW_RESTORE);
                    WaitState("restore by ShowWindow",()=>IsIconic(hwnd)!=0,false,2200,t);
                    WaitForeground("restore by ShowWindow focus",1800,t);
                    actions.Add(Dict("action","restore-by-ShowWindow","observedWindowExists",IsWindow(hwnd)!=0,"observedIsIconic",IsIconic(hwnd)!=0,"observedIsZoomed",IsZoomed(hwnd)!=0,"observedWindowStatePassed",true)); checks.Add(Dict("check","ShowWindow restore and non-iconic state observation","passed",true));
                    ActionClick(actions,"maximize",TITLEBAR_BUTTON_MAXIMIZE_RESTORE,false,true,t); checks.Add(Dict("check","real native maximize click and IsZoomed observation","passed",true));
                    ActionClick(actions,"restore",TITLEBAR_BUTTON_MAXIMIZE_RESTORE,false,false,t); checks.Add(Dict("check","real native restore click and non-zoomed state observation","passed",true));
                    ActionClick(actions,"close",TITLEBAR_BUTTON_CLOSE,false,false,t); checks.Add(Dict("check","real native close click and IsWindow false observation","passed",true));
                } catch(Exception ex) { lock(gate) workerError=ex; }
                finally {
                    if(workerDpiChanged) {
                        bool workerDpiRestored=SetThreadDpiAwarenessContext(workerOriginalDpi)!=IntPtr.Zero;
                        lock(gate) {
                            metadata["workerThreadDpiContextRestored"] = workerDpiRestored;
                            if(!workerDpiRestored) workerError = workerError ?? new Exception("Failed to restore worker thread DPI awareness context, Win32="+Marshal.GetLastWin32Error());
                        }
                    }
                    done.Set();
                }
            });
            worker.IsBackground=true; worker.Name="NativeCaptionProbeActions"; worker.Start();
            PumpUntil(done,TotalLimitMs);
            if(workerError!=null) throw workerError;
            bool dpiMetadataPassed=metadata.ContainsKey("uiThreadDpiAwarenessContext") && metadata.ContainsKey("workerThreadDpiAwarenessContext") && Convert.ToString(metadata["uiThreadDpiAwarenessContext"])!="0x0" && Convert.ToString(metadata["workerThreadDpiAwarenessContext"])!="0x0" && Convert.ToUInt32(metadata["windowDpi"])>0 && !metadata.ContainsKey("workerDpiContextError");
            checks.Add(Dict("check","UI and worker thread DPI contexts and window DPI recorded","passed",dpiMetadataPassed));
            if(!dpiMetadataPassed) throw new Exception("Required thread or window DPI metadata is missing");
            checks.Add(Dict("check","all required native caption actions completed","passed",true));
        } catch(Exception ex) {
            errors.Add(ex.GetType().Name+": "+ex.Message);
            checks.Add(Dict("check","probe completed without action, metadata, or timeout failure","passed",false));
        } finally {
            bool windowGone=true;
            if(created && IsWindow(hwnd)!=0) {
                int destroyedResult=DestroyWindow(hwnd);
                windowGone=IsWindow(hwnd)==0;
                if(!windowGone) errors.Add("Cleanup: DestroyWindow failed; owned HWND still exists, Win32="+Marshal.GetLastWin32Error());
                metadata["destroyWindowReturn"] = destroyedResult;
            } else if(created) windowGone=IsWindow(hwnd)==0;
            checks.Add(Dict("check","owned test HWND removed during cleanup","passed",windowGone));
            bool classRemoved=true;
            if(registered) {
                int removed=UnregisterClass(className,module);
                classRemoved=removed!=0;
                metadata["unregisterClassReturn"] = removed;
                if(!classRemoved) errors.Add("Cleanup: UnregisterClass failed, Win32="+Marshal.GetLastWin32Error());
            }
            checks.Add(Dict("check","owned window class removed during cleanup","passed",classRemoved));
            if(dpiChanged) {
                bool restored=SetThreadDpiAwarenessContext(originalDpi)!=IntPtr.Zero;
                metadata["uiThreadDpiContextRestored"] = restored;
                if(!restored) errors.Add("Cleanup: failed to restore UI thread DPI awareness context, Win32="+Marshal.GetLastWin32Error());
                checks.Add(Dict("check","UI thread DPI context restored","passed",restored));
            }
        }
        bool passed=errors.Count==0 && checks.TrueForAll(c => Convert.ToBoolean(c["passed"]));
        var result=Dict("schemaVersion",1,"probe","native-caption-input","timestampUtc",DateTime.UtcNow.ToString("o"),"passed",passed,"FullAppAccepted",false,"metadata",metadata,"checks",checks,"actions",actions,"errors",errors,"elapsedMs",started.ElapsedMilliseconds);
        string json=JsonSerializer.Serialize(result,new JsonSerializerOptions{WriteIndented=true});
        System.IO.File.WriteAllText(System.IO.Path.Combine(outputDirectory,"result.json"),json+Environment.NewLine);
        return json;
    }
}
'@

try {
    Add-Type -TypeDefinition $source -Language CSharp -ErrorAction Stop
    $json = [NativeCaptionProbe]::Run([System.IO.Path]::GetFullPath($OutputDirectory), [bool]$TaoBlur)
    [Console]::WriteLine($json)
    $result = $json | ConvertFrom-Json
    if (-not $result.passed) { exit 1 }
    exit 0
}
catch {
    $failure = [ordered]@{
        schemaVersion = 1
        probe = 'native-caption-input'
        timestampUtc = [DateTime]::UtcNow.ToString('o')
        passed = $false
        FullAppAccepted = $false
        checks = @(@{ check = 'probe completed without action, metadata, or timeout failure'; passed = $false })
        actions = @()
        errors = @($_.Exception.ToString())
    }
    try {
        $failure | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path ([System.IO.Path]::GetFullPath($OutputDirectory)) 'result.json') -Encoding utf8
    } catch { }
    [Console]::Error.WriteLine($_.Exception.ToString())
    exit 1
}
