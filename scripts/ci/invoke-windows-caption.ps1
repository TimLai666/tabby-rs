<#
.SYNOPSIS
Observe or exercise caption input on one existing owned Windows application.
.DESCRIPTION
Validates the exact PID, creation UTC and root HWND. Saves fresh JSON evidence.
Use a 25-second caller timeout and existing RecoverMouse for an owned remaining journal.
.PARAMETER AppProcessId
Existing application owner PID.
.PARAMETER CreationUtc
Exact launcher CIM creation timestamp in UTC.
.PARAMETER WindowHandle
Owned top-level HWND as a positive decimal integer.
.PARAMETER Action
Snapshot, PressOutside, PressCancel, InvokeMaximize, InvokeRestore, InvokeMinimize,
RestoreWindow, InvokeClose, CancelClose, CloseSecondary, TypeTerminal, or EnumerateWindows.
.PARAMETER OutputFile
Fresh absolute JSON evidence file in an existing owned output directory.
.PARAMETER TextBase64
TypeTerminal only: bounded UTF-16LE base64 text followed by Enter.
.PARAMETER DialogHandle
CancelClose only: exact previously observed native warning HWND.
.PARAMETER ExpectedWarningText
CancelClose only: exact previously observed warning text.
.EXAMPLE
pwsh -NoProfile -File invoke-windows-caption.ps1 -AppProcessId 1234 -CreationUtc 2026-10-17T00:00:00.0000000Z -WindowHandle 12345 -Action Snapshot -OutputFile C:\owned\snapshot.json
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][int]$AppProcessId,
    [Parameter(Mandatory=$true)][string]$CreationUtc,
    [Parameter(Mandatory=$true)][long]$WindowHandle,
    [Parameter(Mandatory=$true)][string]$Action,
    [Parameter(Mandatory=$true)][string]$OutputFile,
    [string]$TextBase64,
    [long]$DialogHandle=0,
    [string]$ExpectedWarningText
)
$ErrorActionPreference='Stop'

$validActions = @('Snapshot','PressOutside','PressCancel','InvokeMaximize','InvokeRestore','InvokeMinimize','RestoreWindow','InvokeClose','CancelClose','CloseSecondary','TypeTerminal','EnumerateWindows')
$out = [ordered]@{ok=$false;identity=[ordered]@{pid=$AppProcessId;creationUtc=$CreationUtc;hwnd=$WindowHandle};action=$Action;before=$null;after=$null;error=$null}
$canWrite=$false
function Get-ProcessTopWindows { param([int]$Owner)
 $tops=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$Owner));return $tops
}
function Get-DialogWarningText { param($Element)
 $deadline=[DateTime]::UtcNow.AddSeconds(8);$cond=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Text)
 $nodes=$Element.FindAll([Windows.Automation.TreeScope]::Descendants,$cond);if([DateTime]::UtcNow -gt $deadline){throw 'Dialog text UIA search exceeded 8 seconds'};$parts=[Collections.Generic.List[string]]::new()
 foreach($node in $nodes){if([DateTime]::UtcNow -gt $deadline){throw 'Dialog text UIA search exceeded 8 seconds'};$v=[string]$node.Current.Name;if(-not [string]::IsNullOrWhiteSpace($v)){$parts.Add($v.Trim())}}
 return (($parts | Select-Object -Unique) -join "`n")
}
function Get-UniqueCaptionButton {
 param([IntPtr]$Root,[string]$Name,[int]$Owner)
 $rootElement=[Windows.Automation.AutomationElement]::FromHandle($Root)
 if($rootElement.Current.ProcessId -ne $Owner){throw 'Accessible root owner mismatch'}
 $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Button)
 $uiaDeadline=[DateTime]::UtcNow.AddSeconds(8);$all=$rootElement.FindAll([Windows.Automation.TreeScope]::Descendants,$condition);if([DateTime]::UtcNow -gt $uiaDeadline){throw 'Caption UIA search exceeded 8 seconds'}; $found=@(); $inventory=[Collections.Generic.List[object]]::new()
 $r=$rootElement.Current.BoundingRectangle
 foreach($b in $all){if([DateTime]::UtcNow -gt $uiaDeadline){throw 'Caption UIA search exceeded 8 seconds'};$c=$b.Current;$q=$c.BoundingRectangle;$entry=@{name=[string]$c.Name;processId=$c.ProcessId;enabled=$c.IsEnabled;offscreen=$c.IsOffscreen;bounds=@{x=$q.X;y=$q.Y;width=$q.Width;height=$q.Height}};$inventory.Add($entry)
   if(-not $q.IsEmpty -and $q.Width -gt 0 -and $q.Height -gt 0 -and $c.ProcessId -eq $Owner -and $c.Name -ceq $Name -and $c.IsEnabled -and -not $c.IsOffscreen -and -not $q.IsEmpty -and $q.Width -gt 0 -and $q.Height -gt 0){
    if($q.X -lt ($r.X+$r.Width*0.60) -or $q.Y -lt $r.Y -or $q.Y+$q.Height -gt $r.Y+80){continue};$found+=,$b
   }
 }
 $script:captionInventory=$inventory
 if($found.Count -ne 1){throw "Expected exactly one visible enabled $Name button in right caption band, found $($found.Count)"}
 $z=$found[0].Current.BoundingRectangle; return @{x=[int][Math]::Floor($z.X+$z.Width/2);y=[int][Math]::Floor($z.Y+$z.Height/2);bottom=[int][Math]::Ceiling($z.Y+$z.Height);element=$found[0]}
}
try {
    if (-not [IO.Path]::IsPathRooted($OutputFile)) { throw 'OutputFile must be absolute' }
    $full=[IO.Path]::GetFullPath($OutputFile); $dir=[IO.Path]::GetDirectoryName($full)
    if (-not (Test-Path -LiteralPath $dir -PathType Container)) { throw 'OutputFile directory does not exist' }
    if (Test-Path -LiteralPath $full) { throw 'OutputFile already exists' }
    $canWrite=$true
    if ($AppProcessId -le 0 -or $WindowHandle -le 0) { throw 'AppProcessId and WindowHandle must be positive' }
    if ($validActions -cnotcontains $Action) { throw 'Unsupported Action' }
    if (-not $IsWindows -or -not [Environment]::Is64BitProcess) { throw 'Requires 64-bit PowerShell on Windows' }
    if ($Action -eq 'TypeTerminal') { if (-not $TextBase64) { throw 'TextBase64 is required for TypeTerminal' } } elseif ($PSBoundParameters.ContainsKey('TextBase64')) { throw 'TextBase64 is only valid for TypeTerminal' }
    if ($Action -eq 'CancelClose') { if ($DialogHandle -le 0 -or [string]::IsNullOrEmpty($ExpectedWarningText)) { throw 'CancelClose requires DialogHandle and ExpectedWarningText' } } elseif ($PSBoundParameters.ContainsKey('DialogHandle') -or $PSBoundParameters.ContainsKey('ExpectedWarningText')) { throw 'DialogHandle and ExpectedWarningText are only valid for CancelClose' }
    $process=Get-CimInstance Win32_Process -Filter "ProcessId = $AppProcessId" -ErrorAction Stop
    if ($null -eq $process) { throw 'Owned process does not exist' }
    $actual=$process.CreationDate.ToUniversalTime().ToString('o')
    if ($actual -cne $CreationUtc) { throw 'CreationUtc does not match CIM process identity' }
    $filetime=$process.CreationDate.ToUniversalTime().ToFileTimeUtc()
    $journal=$full+'.mouse-down'
    if (@(Get-ChildItem -LiteralPath $dir -Filter '*.mouse-down' -File).Count -gt 0) { throw 'A preexisting mouse journal blocks input actions' }
    Add-Type -AssemblyName UIAutomationClient; Add-Type -AssemblyName UIAutomationTypes
    $providers=[Reflection.Assembly]::Load('UIAutomationClientSideProviders')
    Add-Type -ReferencedAssemblies @([Windows.Automation.AutomationElement].Assembly.Location,[Windows.Automation.ControlType].Assembly.Location,$providers.Location) -TypeDefinition @'
public static class CaptionProviders { public static void Register(){System.Windows.Automation.ClientSettings.RegisterClientSideProviders(UIAutomationClientsideProviders.UIAutomationClientSideProviders.ClientSideProviderDescriptionTable);} }
'@
    [CaptionProviders]::Register()
    Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices; using System.Text; using System.Collections.Generic; using System.Threading; using System.Diagnostics; using System.IO; using System.Globalization;
public static class CaptionNative {
 [StructLayout(LayoutKind.Sequential)] public struct RECT { public int l,t,r,b; } [StructLayout(LayoutKind.Sequential)] struct POINT{public int x,y;}
 [StructLayout(LayoutKind.Sequential)] public struct GUITHREADINFO { public int cbSize,flags; public IntPtr hwndActive,hwndFocus,hwndCapture,hwndMenuOwner,hwndMoveSize,hwndCaret; public RECT rcCaret; }
 [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx,dy; public uint data,flags,time; public UIntPtr extra; }
 [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk,scan; public uint flags,time; public UIntPtr extra; }
 [StructLayout(LayoutKind.Explicit,Size=32)] struct U { [FieldOffset(0)] public MOUSEINPUT m; [FieldOffset(0)] public KEYBDINPUT k; }
 [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public U u; }
 [StructLayout(LayoutKind.Sequential)] struct FT {public uint lo,hi;}
 [DllImport("user32.dll")] static extern bool IsWindow(IntPtr h); [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h); [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h); [DllImport("user32.dll")] static extern bool IsZoomed(IntPtr h);
 [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid); [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h,uint f); [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr h,int cmd); [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr h); [DllImport("user32.dll")] static extern int GetWindowRect(IntPtr h,out RECT r); [DllImport("user32.dll")] static extern int GetClientRect(IntPtr h,out RECT r); [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h,ref POINT p); [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,EnumProc cb,IntPtr l); [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb,IntPtr l); delegate bool EnumProc(IntPtr h,IntPtr l); [DllImport("user32.dll")] static extern int GetGUIThreadInfo(uint id,ref GUITHREADINFO i); [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h,StringBuilder b,int n); [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll",SetLastError=true)] static extern uint SendInput(uint n,INPUT[] a,int size); [DllImport("user32.dll",SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint m,UIntPtr w,IntPtr l,uint f,uint ms,out UIntPtr result); [DllImport("user32.dll")] static extern short GetAsyncKeyState(int k); [DllImport("user32.dll")] static extern int GetSystemMetrics(int i); [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT p);
 [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint a,bool inh,int p); [DllImport("kernel32.dll")] static extern bool GetProcessTimes(IntPtr h,out FT c,out FT e,out FT k,out FT u); [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c); [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr h);
 static IntPtr root; static int owner; static long creation;
 static void C(bool b,string m){if(!b)throw new Exception(m);} static long FTVal(FT x){return ((long)x.hi<<32)|x.lo;}
 public static void Init(int p,long h,long ft){owner=p;root=new IntPtr(h);creation=ft; Validate();}
 public static void Validate(){ C(IsWindow(root)&&IsWindowVisible(root),"root HWND is not visible/live"); uint p; GetWindowThreadProcessId(root,out p); C(p==(uint)owner&&GetAncestor(root,2)==root,"root HWND ownership mismatch");C(GetDpiForWindow(root)>0,"GetDpiForWindow failed"); IntPtr ph=OpenProcess(0x1000,false,owner); C(ph!=IntPtr.Zero,"OpenProcess failed"); try{FT c,e,k,u;C(GetProcessTimes(ph,out c,out e,out k,out u)&&FTVal(c)/10==creation/10,"native process creation identity mismatch");}finally{CloseHandle(ph);} }
 static Dictionary<string,object> Win(IntPtr h){RECT r;C(GetWindowRect(h,out r)!=0,"GetWindowRect failed");return new Dictionary<string,object>{{"x",r.l},{"y",r.t},{"width",r.r-r.l},{"height",r.b-r.t},{"minimized",IsIconic(h)},{"maximized",IsZoomed(h)}};}
 public static Dictionary<string,object> State(){Validate();uint tid=GetWindowThreadProcessId(root,out _);GUITHREADINFO i=new GUITHREADINFO();i.cbSize=Marshal.SizeOf(typeof(GUITHREADINFO));C(GetGUIThreadInfo(tid,ref i)!=0,"GetGUIThreadInfo failed");return new Dictionary<string,object>{{"bounds",Win(root)}, {"minimized",IsIconic(root)},{"maximized",IsZoomed(root)},{"nativeFocus",new Dictionary<string,object>{{"focus",i.hwndFocus.ToInt64()},{"capture",i.hwndCapture.ToInt64()},{"active",i.hwndActive.ToInt64()}}},{"dpi",GetDpiForWindow(root)}};}
 public static List<object> Children(){var a=new List<object>();EnumChildWindows(root,delegate(IntPtr h,IntPtr l){uint p;GetWindowThreadProcessId(h,out p);if(p==(uint)owner){StringBuilder c=new StringBuilder(256);GetClassNameW(h,c,c.Capacity);a.Add(new {hwnd=h.ToInt64(),parent=GetParent(h).ToInt64(),@class=c.ToString(),visible=IsWindowVisible(h)});}return true;},IntPtr.Zero);return a;}
 public static List<object> Owned(){var a=new List<object>();EnumWindows(delegate(IntPtr h,IntPtr l){uint p;GetWindowThreadProcessId(h,out p);if(p==(uint)owner&&IsWindowVisible(h)&&GetWindow(h,4)==IntPtr.Zero){StringBuilder c=new StringBuilder(256);GetClassNameW(h,c,c.Capacity);if(c.ToString()!="#32770")a.Add(new {hwnd=h.ToInt64(),@class=c.ToString()});}return true;},IntPtr.Zero);return a;}
 public static bool IsUnownedWindow(IntPtr h){uint p;GetWindowThreadProcessId(h,out p);return IsWindow(h)&&IsWindowVisible(h)&&p==(uint)owner&&GetAncestor(h,2)==h&&GetWindow(h,4)==IntPtr.Zero&&!IsDialog(h);}
 public static bool LeftDown(){return (((ushort)GetAsyncKeyState(1))&0x8000)!=0;}
 static void SendMouse(uint flags){INPUT i=new INPUT();i.type=0;i.u.m.flags=flags;C(SendInput(1,new[]{i},Marshal.SizeOf(typeof(INPUT)))==1,"SendInput mouse failed");}
 static void SendKey(ushort vk,bool up){INPUT i=new INPUT();i.type=1;i.u.k.vk=vk;i.u.k.flags=up?2u:0u;C(SendInput(1,new[]{i},Marshal.SizeOf(typeof(INPUT)))==1,"SendInput keyboard failed");} static void Enter(){SendKey(0x0d,false);try{SendKey(0x0d,true);}catch{try{SendKey(0x0d,true);}catch{throw new Exception("inputCleanupFailed: Enter key-up could not be confirmed");}throw;}}
 public static void Mouse(string action,int downX,int downY,int captionBottom){Validate();C(!LeftDown(),"left mouse button already held");RECT initialBounds;C(GetWindowRect(root,out initialBounds)!=0,"initial bounds unavailable");string jp=Environment.GetEnvironmentVariable("CAPTION_JOURNAL");using(var f=new FileStream(jp,FileMode.CreateNew,FileAccess.Write,FileShare.None))using(var w=new StreamWriter(f,new UTF8Encoding(false))){w.WriteLine(owner);w.WriteLine(root.ToInt64());w.WriteLine(Process.GetCurrentProcess().Id);w.Flush();f.Flush(true);}bool down=false;bool journalActive=true;try{SetInputPoint(downX,downY);Validate();C(GetAncestor(WindowFromPoint(new POINT{x=downX,y=downY}),2)==root,"Maximize input point no longer belongs to root");SendMouse(2);down=true;uint capTid=GetWindowThreadProcessId(root,out _);GUITHREADINFO initial=new GUITHREADINFO();initial.cbSize=Marshal.SizeOf(typeof(GUITHREADINFO));var capWait=Stopwatch.StartNew();do{C(GetGUIThreadInfo(capTid,ref initial)!=0,"GetGUIThreadInfo failed after LEFTDOWN");if(initial.hwndCapture!=IntPtr.Zero)break;Thread.Sleep(10);}while(capWait.ElapsedMilliseconds<1000);C(initial.hwndCapture!=IntPtr.Zero&&GetAncestor(initial.hwndCapture,2)==root,"initial capture is not in exact owned app tree");if(action=="PressCancel"){uint tid=GetWindowThreadProcessId(root,out _);GUITHREADINFO gi=new GUITHREADINFO();gi.cbSize=Marshal.SizeOf(typeof(GUITHREADINFO));C(GetGUIThreadInfo(tid,ref gi)!=0&&gi.hwndCapture!=IntPtr.Zero,"no owned native capture");C(GetAncestor(gi.hwndCapture,2)==root,"capture is outside owned tree");UIntPtr z;C(SendMessageTimeoutW(gi.hwndCapture,0x001f,UIntPtr.Zero,IntPtr.Zero,3,1000,out z)!=IntPtr.Zero,"WM_CANCELMODE failed");}else {RECT client;C(GetClientRect(root,out client)!=0,"client bounds unavailable");POINT origin=new POINT{x=client.l+(client.r-client.l)/2,y=client.t+(client.b-client.t)/2};C(ClientToScreen(root,ref origin),"ClientToScreen failed");int x=origin.x,y=origin.y;C(y>captionBottom,"outside point is not below caption control band");var pt=new POINT{x=x,y=y};C(y>0&&GetAncestor(WindowFromPoint(pt),2)==root,"outside-caption client point not owned");SendInputMove(x,y);
 }Validate();RECT finalBounds;C(GetWindowRect(root,out finalBounds)!=0&&finalBounds.l==initialBounds.l&&finalBounds.t==initialBounds.t&&finalBounds.r==initialBounds.r&&finalBounds.b==initialBounds.b,"mouse cancellation changed root bounds");SendMouse(4);var sw=Stopwatch.StartNew();while(LeftDown()&&sw.ElapsedMilliseconds<500)Thread.Sleep(10);C(!LeftDown(),"mouse button remained down");down=false;string[] ownedLines=File.ReadAllLines(jp,new UTF8Encoding(false,true));C(ownedLines.Length==3&&ownedLines[0]==owner.ToString(CultureInfo.InvariantCulture)&&ownedLines[1]==root.ToInt64().ToString(CultureInfo.InvariantCulture)&&ownedLines[2]==Process.GetCurrentProcess().Id.ToString(CultureInfo.InvariantCulture),"journal identity changed; retained");File.Delete(jp);journalActive=false;}finally{Exception clean=null;if(down){try{SendMouse(4);var releaseWait=Stopwatch.StartNew();while(LeftDown()&&releaseWait.ElapsedMilliseconds<500)Thread.Sleep(10);C(!LeftDown(),"cleanup LEFTUP was not observed");down=false;}catch(Exception e){clean=e;}}if(journalActive&&clean==null){try{string[] lines=File.ReadAllLines(jp,new UTF8Encoding(false,true));C(lines.Length==3&&lines[0]==owner.ToString(CultureInfo.InvariantCulture)&&lines[1]==root.ToInt64().ToString(CultureInfo.InvariantCulture)&&lines[2]==Process.GetCurrentProcess().Id.ToString(CultureInfo.InvariantCulture),"journal ownership changed; retained");File.Delete(jp);journalActive=false;}catch(Exception e){clean=e;}}if(clean!=null)throw new Exception("inputCleanupFailed: "+clean.Message);}}
 [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT p);
 static void SendInputMove(int x,int y){int l=GetSystemMetrics(76),t=GetSystemMetrics(77),w=GetSystemMetrics(78),h=GetSystemMetrics(79);C(w>1&&h>1&&x>=l&&x<l+w&&y>=t&&y<t+h,"input point outside virtual desktop");INPUT i=new INPUT();i.type=0;i.u.m.dx=(int)Math.Round((x-l)*65535.0/(w-1));i.u.m.dy=(int)Math.Round((y-t)*65535.0/(h-1));i.u.m.flags=0x0001|0x4000|0x8000;C(SendInput(1,new[]{i},Marshal.SizeOf(typeof(INPUT)))==1,"SendInput move failed");POINT p;var wait=Stopwatch.StartNew();do{C(GetCursorPos(out p),"GetCursorPos failed");if(p.x==x&&p.y==y)break;Thread.Sleep(2);}while(wait.ElapsedMilliseconds<500);C(p.x==x&&p.y==y,"SendInput cursor did not reach exact point");C(GetAncestor(WindowFromPoint(new POINT{x=x,y=y}),2)==root,"cursor move escaped owned root");}
 static void SetInputPoint(int x,int y){SendInputMove(x,y);}
 public static int Type(string text){int sent=0;for(int offset=0;offset<text.Length;offset+=128){Validate();C(GetForegroundWindow()==root,"root must remain foreground");C(!LeftDown(),"mouse button held");for(int k=0;k<3;k++)C((GetAsyncKeyState(new[]{0x11,0x12,0x10}[k])&0x8000)==0,"modifier key held");int end=Math.Min(text.Length,offset+128);for(int n=offset;n<end;n++){INPUT d=new INPUT();d.type=1;d.u.k.scan=text[n];d.u.k.flags=4;INPUT u=d;u.u.k.flags=6;uint sentCount=SendInput(2,new[]{d,u},Marshal.SizeOf(typeof(INPUT)));if(sentCount!=2){if(sentCount>0){u.u.k.flags=6;if(SendInput(1,new[]{u},Marshal.SizeOf(typeof(INPUT)))!=1)throw new Exception("inputCleanupFailed: Unicode key-up could not be confirmed");}throw new Exception("Unicode SendInput failed; key-up cleanup attempted");}sent++;}}Validate();C(GetForegroundWindow()==root,"root lost foreground before Enter");Enter();return sent;}
 public static bool Alive(IntPtr h){return IsWindow(h);} public static bool IsDialog(IntPtr h){StringBuilder b=new StringBuilder(128);GetClassNameW(h,b,b.Capacity);return b.ToString()=="#32770";} public static bool DialogOwnedByRoot(IntPtr h){return GetWindow(h,4)==root&&GetAncestor(h,3)==root;}
}
'@ -ErrorAction Stop
    [CaptionNative]::Init($AppProcessId,$WindowHandle,$filetime)
    $old=[CaptionNative]::SetThreadDpiAwarenessContext([IntPtr](-4)); if ($old -eq [IntPtr]::Zero) { throw 'Could not set per-monitor DPI context' }
    try {
      $before=[CaptionNative]::State(); $out.before=$before; $out.children=[CaptionNative]::Children()
      if ($Action -eq 'Snapshot') { $out.after=[CaptionNative]::State() }
      elseif ($Action -eq 'EnumerateWindows') { $out.ownedWindows=[CaptionNative]::Owned(); $out.after=[CaptionNative]::State() }
      elseif ($Action -eq 'TypeTerminal') { $bytes=[Convert]::FromBase64String($TextBase64); if($bytes.Length%2 -or $bytes.Length -gt 16384){throw 'Text must be at most 8192 UTF-16LE characters'}; $text=[Text.Encoding]::Unicode.GetString($bytes); if($text.Length -gt 8192){throw 'Text exceeds 8192 characters'}; $out.textLength=$text.Length; $out.textSha256=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant(); $out.sentCharacters=[CaptionNative]::Type($text); $out.after=[CaptionNative]::State() }
      elseif ($Action -in @('PressOutside','PressCancel')) { if($before.minimized -or $before.maximized){throw 'PressOutside/PressCancel require an ordinary root with Maximize control'}; $index=3; $max=Get-UniqueCaptionButton -Root ([IntPtr]$WindowHandle) -Name 'Maximize' -Owner $AppProcessId; $out.buttons=$captionInventory; $env:CAPTION_JOURNAL=$journal; [CaptionNative]::Mouse($Action,[int]$max.x,[int]$max.y,[int]$max.bottom); $after=[CaptionNative]::State(); $out.after=$after; $out.physicalMouseDown=$true; $out.releasedOutside=($Action -eq 'PressOutside'); $out.cancelled=($Action -eq 'PressCancel'); $out.captureCleared=($after.nativeFocus.capture -eq 0); $out.mouseButtonUp=(-not [CaptionNative]::LeftDown()); $out.cancellationMethod=$(if($Action -eq 'PressCancel'){'WM_CANCELMODE'}else{'physical outside release'}); if(-not $out.captureCleared -or -not $out.mouseButtonUp){throw 'Mouse cleanup/capture verification failed'};if(($before.bounds|ConvertTo-Json -Compress) -cne ($after.bounds|ConvertTo-Json -Compress) -or $before.minimized -ne $after.minimized -or $before.maximized -ne $after.maximized){throw 'Mouse cancellation changed root state or bounds'} }
      else {
        $target=[IntPtr]$WindowHandle; $name=switch($Action){'InvokeMaximize'{'Maximize'}'InvokeRestore'{'Restore'}'InvokeMinimize'{'Minimize'}'RestoreWindow'{'Restore'}'InvokeClose'{'Close'}'CancelClose'{'Cancel'}'CloseSecondary'{'Close'}}
        $uiaRoot=[Windows.Automation.AutomationElement]::FromHandle($target); if($uiaRoot.Current.ProcessId -ne $AppProcessId){throw 'UIA root ownership mismatch'}
        if($Action -eq 'CancelClose'){$rootTarget=$target; $target=[IntPtr]$DialogHandle; [CaptionNative]::Validate(); $uiaRoot=[Windows.Automation.AutomationElement]::FromHandle($target); $dlg=$uiaRoot; if($dlg.Current.ProcessId -ne $AppProcessId -or $dlg.Current.ClassName -ne '#32770' -or -not [CaptionNative]::DialogOwnedByRoot($target)){throw 'CancelClose dialog owner/root identity mismatch'}; $actualWarning=Get-DialogWarningText $uiaRoot; if($actualWarning -cne $ExpectedWarningText){throw 'Dialog warning text does not match expected text'}; $out.dialogHwnd=$DialogHandle;$out.warningText=$actualWarning }
        if($Action -eq 'CloseSecondary' -and -not [CaptionNative]::IsUnownedWindow($target)){throw 'CloseSecondary target must be this owner process visible unowned top-level HWND'}
        if($Action -eq 'RestoreWindow'){
          [CaptionNative]::Validate();if($uiaRoot.Current.ProcessId -ne $AppProcessId){throw 'RestoreWindow UIA root changed'}
          if(-not $before.minimized){throw 'RestoreWindow requires a minimized root'}
          $wp=$uiaRoot.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern);$out.invocationCount=1;$wp.SetWindowVisualState([Windows.Automation.WindowVisualState]::Normal)
          $deadline=[DateTime]::UtcNow.AddSeconds(3);do{$out.after=[CaptionNative]::State();if(-not $out.after.minimized){break};Start-Sleep -Milliseconds 50}while([DateTime]::UtcNow -lt $deadline)
          if($out.after.minimized){throw 'WindowPattern restore did not restore minimized root'}
        } else {
        $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Button)
        $uiaDeadline=[DateTime]::UtcNow.AddSeconds(8);$buttons=$uiaRoot.FindAll([Windows.Automation.TreeScope]::Descendants,$condition);if([DateTime]::UtcNow -gt $uiaDeadline){throw 'Button UIA search exceeded 8 seconds'}; $matches=@(); $inventory=[Collections.Generic.List[object]]::new(); foreach($b in $buttons){if([DateTime]::UtcNow -gt $uiaDeadline){throw 'Button UIA search exceeded 8 seconds'};$c=$b.Current;$br=$c.BoundingRectangle;$inventory.Add(@{name=[string]$c.Name;processId=$c.ProcessId;enabled=$c.IsEnabled;offscreen=$c.IsOffscreen;bounds=@{x=$br.X;y=$br.Y;width=$br.Width;height=$br.Height}});if($c.ProcessId -eq $AppProcessId -and $c.Name -ceq $name -and $c.IsEnabled -and -not $c.IsOffscreen -and -not $br.IsEmpty -and $br.Width -gt 0 -and $br.Height -gt 0){if($Action -eq 'CancelClose' -or ($br.X -ge ($before.bounds.x+$before.bounds.width*0.6) -and $br.Y -ge $before.bounds.y -and $br.Y+$br.Height -le $before.bounds.y+80)){$matches+=,$b}}};$out.buttons=$inventory;$out.accessibleRoot=@{processId=$uiaRoot.Current.ProcessId;name=[string]$uiaRoot.Current.Name;className=[string]$uiaRoot.Current.ClassName}
        if($matches.Count -ne 1){throw "Expected one enabled visible '$name' button, found $($matches.Count)"}
        $button=$matches[0]; $rect=$button.Current.BoundingRectangle; $win=$before.bounds; if($Action -ne 'CancelClose' -and ($rect.X -lt ($win.x+$win.width*0.6) -or $rect.Y -lt $win.y -or ($rect.Y+$rect.Height) -gt ($win.y+80))){throw 'UIA button is outside right caption band'}
        if($Action -eq 'InvokeMaximize' -and ($before.maximized -or $before.minimized)){throw 'InvokeMaximize requires ordinary window'};if($Action -eq 'InvokeRestore' -and (-not $before.maximized -or $before.minimized)){throw 'InvokeRestore requires maximized window'};if($Action -eq 'InvokeMinimize' -and $before.minimized){throw 'InvokeMinimize requires non-minimized window'};if($Action -eq 'InvokeClose'){foreach($w in [Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$AppProcessId))){$wh=$w.Current.NativeWindowHandle;if($wh -and [CaptionNative]::IsDialog([IntPtr]$wh) -and [CaptionNative]::DialogOwnedByRoot([IntPtr]$wh)){throw 'An owned native warning already exists before InvokeClose'}}}
        $currentButton=$button.Current;[CaptionNative]::Validate();if($Action -eq 'CloseSecondary' -and -not [CaptionNative]::IsUnownedWindow($target)){throw 'CloseSecondary HWND identity changed before Invoke'};if(($Action -eq 'CancelClose' -and -not [CaptionNative]::DialogOwnedByRoot($target)) -or $uiaRoot.Current.ProcessId -ne $AppProcessId -or $currentButton.ProcessId -ne $AppProcessId -or $currentButton.Name -cne $name -or -not $currentButton.IsEnabled -or $currentButton.IsOffscreen){throw 'UIA target identity/state changed before Invoke'};$out.invocationCount=1; $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
        $deadline=[DateTime]::UtcNow.AddSeconds(3); if($Action -eq 'InvokeClose' -or $Action -eq 'CancelClose'){Start-Sleep -Milliseconds 100}
        if($Action -eq 'InvokeClose'){ $found=$null; while([DateTime]::UtcNow -lt $deadline -and $null -eq $found){$tops=Get-ProcessTopWindows $AppProcessId;foreach($w in $tops){$wh=$w.Current.NativeWindowHandle;if($wh -and [CaptionNative]::IsDialog([IntPtr]$wh) -and [CaptionNative]::DialogOwnedByRoot([IntPtr]$wh)){$body=Get-DialogWarningText $w;if(-not [string]::IsNullOrWhiteSpace($body)){$found=@{hwnd=$wh;body=$body};break}}};if($null -eq $found){Start-Sleep -Milliseconds 50}};if($null -eq $found){throw 'Owned close warning dialog with text did not appear'};if($found.body -match '(?i)paste' -or $found.body -notmatch '(?i)running'){throw 'Dialog text is not a running-process Close warning'};$out.dialogHwnd=$found.hwnd;$out.warningText=$found.body;$out.closeWarningPending=$true;$out.after=[CaptionNative]::State() }
        elseif($Action -eq 'CancelClose'){$deadline=[DateTime]::UtcNow.AddSeconds(3);while([CaptionNative]::Alive([IntPtr]$DialogHandle) -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 50};if([CaptionNative]::Alive([IntPtr]$DialogHandle)){throw 'Warning dialog remained after Cancel'};[CaptionNative]::Init($AppProcessId,$WindowHandle,$filetime);$out.cancelled=$true;$out.windowRetained=[CaptionNative]::Alive($rootTarget);if(-not $out.windowRetained){throw 'Original root did not remain after Cancel'};$out.after=[CaptionNative]::State() }
        elseif($Action -in @('InvokeMaximize','InvokeRestore','InvokeMinimize','RestoreWindow')){$deadline=[DateTime]::UtcNow.AddSeconds(3);do{$out.after=[CaptionNative]::State();$settled=if($Action -eq 'InvokeMaximize'){$out.after.maximized}elseif($Action -eq 'InvokeMinimize'){$out.after.minimized}else{(-not $out.after.minimized -and -not $out.after.maximized)};if($settled){break};Start-Sleep -Milliseconds 50}while([DateTime]::UtcNow -lt $deadline);if(-not $settled){throw ($Action+' resulting state was not observed within 3 seconds')}}
        elseif($Action -eq 'CloseSecondary'){$deadline=[DateTime]::UtcNow.AddSeconds(3);while([CaptionNative]::Alive($target) -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 50};$out.windowDestroyed=(-not [CaptionNative]::Alive($target));if(-not $out.windowDestroyed){throw 'Secondary root HWND remained after Close'};$out.after=$null}
        }
      }
      $out.ok=$true
    } finally { if([CaptionNative]::SetThreadDpiAwarenessContext($old) -eq [IntPtr]::Zero){throw 'Could not restore DPI context'} }
} catch { $out.error=$_.Exception.GetType().Name+': '+$_.Exception.Message; if($_.Exception.Message -match 'inputCleanupFailed'){$out.inputCleanupFailed=$true} if($canWrite){try{$out.after=[CaptionNative]::State()}catch{}} }
if($canWrite){$json=$out|ConvertTo-Json -Depth 12;$stream=$null;try{$stream=[IO.File]::Open($full,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None);$writer=[IO.StreamWriter]::new($stream,[Text.UTF8Encoding]::new($false));$writer.WriteLine($json);$writer.Flush();$stream.Flush($true);$writer.Dispose();$stream=$null}catch{if($stream){$stream.Dispose()};[Console]::Error.WriteLine('Evidence write failed: '+$_.Exception.Message);exit 1}}
$out|ConvertTo-Json -Depth 12 -Compress
if($out.ok){exit 0}else{exit 1}
