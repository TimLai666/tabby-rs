<#
.SYNOPSIS
Records bounded native process-start history for one owned restart observation.
.DESCRIPTION
Run before the UI restart click. Wait for the unique ReadyFile, then consume
complete JSON lines from EventsFile. Create StopFile to stop and reap this helper.
No GUI input, application launch, process termination, or system setting changes.
.PARAMETER EventsFile
Fresh absolute path for append-only JSONL evidence.
.PARAMETER ReadyFile
Fresh absolute path for the observer PID, creation time, and caller nonce.
.PARAMETER StopFile
Fresh absolute path; the caller creates it to end this observation.
.PARAMETER Nonce
Unique caller-provided identifier returned in every evidence record.
.PARAMETER MaximumSeconds
Observation deadline from 1 to 180 seconds. Expiration is an error.
.PARAMETER Help
Print this overview and exit without observing processes.
.EXAMPLE
pwsh -NoProfile -File observe-windows-restart.ps1 -EventsFile C:\qa\events.jsonl -ReadyFile C:\qa\ready.json -StopFile C:\qa\stop -Nonce unique-token
#>
[CmdletBinding()]
param(
    [string]$EventsFile,
    [string]$ReadyFile,
    [string]$StopFile,
    [string]$Nonce,
    [ValidateRange(1,180)][int]$MaximumSeconds = 120,
    [switch]$Help
)
$ErrorActionPreference = 'Stop'
if ($Help -or -not $EventsFile -or -not $ReadyFile -or -not $StopFile -or -not $Nonce) {
    Get-Help $PSCommandPath -Full | Out-String | Write-Output
    if ($Help) { exit 0 }
    exit 2
}
if ($env:OS -ne 'Windows_NT') { throw 'Native restart history requires Windows' }
$paths = @($EventsFile, $ReadyFile, $StopFile)
if (@($paths | Select-Object -Unique).Count -ne 3) { throw 'Evidence paths must be distinct' }
foreach ($path in $paths) {
    if (-not [IO.Path]::IsPathRooted($path) -or (Test-Path -LiteralPath $path)) { throw 'Evidence paths must be fresh and absolute' }
    if (-not (Test-Path -LiteralPath ([IO.Path]::GetDirectoryName($path)) -PathType Container)) { throw 'Evidence parent directory is missing' }
}
$sourceId = 'TabbyRestartOrigin-' + [Guid]::NewGuid().ToString('N')
$stream = $null
$writer = $null
$subscribed = $false
$failed = $false
$eventsCount = 0
$utf8 = [Text.UTF8Encoding]::new($false)
function Write-Record([hashtable]$record) {
    $record.nonce = $Nonce
    $writer.WriteLine(($record | ConvertTo-Json -Compress -Depth 6))
    $writer.Flush()
}
try {
    $stream = [IO.File]::Open($EventsFile, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    $writer = [IO.StreamWriter]::new($stream, $utf8)
    $writer.AutoFlush = $true
    $readyUtc = [DateTime]::UtcNow.ToString('o')
    Register-CimIndicationEvent -ClassName Win32_ProcessStartTrace -SourceIdentifier $sourceId -ErrorAction Stop | Out-Null
    $subscribed = $true
    $self = Get-CimInstance Win32_Process -Filter "ProcessId = $PID" -ErrorAction Stop
    if (-not $self) { throw 'Observer creation identity is missing' }
    $ready = @{kind='ready';nonce=$Nonce;pid=$PID;creationTimeUTC=$self.CreationDate.ToUniversalTime().ToString('o');readyUtc=$readyUtc}
    Write-Record $ready
    $readyTemporary = $ReadyFile + '.' + $Nonce + '.tmp'
    $readyStream = [IO.File]::Open($readyTemporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try {
        $bytes = $utf8.GetBytes(($ready | ConvertTo-Json -Compress))
        $readyStream.Write($bytes,0,$bytes.Length)
        $readyStream.Flush()
    } finally { $readyStream.Dispose() }
    [IO.File]::Move($readyTemporary, $ReadyFile)
    $deadline = [DateTime]::UtcNow.AddSeconds($MaximumSeconds)
    while ($true) {
        foreach ($eventRecord in @(Get-Event -SourceIdentifier $sourceId -ErrorAction SilentlyContinue)) {
            $native = $eventRecord.SourceEventArgs.NewEvent
            if (-not $native -or [long]$native.ProcessID -le 0 -or [uint64]$native.TIME_CREATED -eq 0) { throw 'Malformed native start event' }
            $eventsCount++
            if ($eventsCount -gt 10000) {
                Write-Record @{kind='overflow';eventsCount=$eventsCount}
                throw 'Native process event bound exceeded'
            }
            Write-Record @{kind='start';pid=[long]$native.ProcessID;parentPid=[long]$native.ParentProcessID;fileTime=([uint64]$native.TIME_CREATED).ToString([Globalization.CultureInfo]::InvariantCulture)}
            Remove-Event -EventIdentifier $eventRecord.EventIdentifier -ErrorAction Stop
        }
        Write-Record @{kind='heartbeat';observedUtc=[DateTime]::UtcNow.ToString('o');eventsCount=$eventsCount}
        if (Test-Path -LiteralPath $StopFile -PathType Leaf) { break }
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Native restart observer deadline expired' }
        Start-Sleep -Milliseconds 100
    }
    Write-Record @{kind='stopped';eventsCount=$eventsCount}
} catch {
    $failed = $true
    if ($writer) {
        try { Write-Record @{kind='error';message=$_.Exception.Message;eventsCount=$eventsCount} } catch { }
    }
    Write-Error -Message $_.Exception.Message -ErrorAction Continue
} finally {
    if ($subscribed) {
        Unregister-Event -SourceIdentifier $sourceId -ErrorAction SilentlyContinue
        Get-Event -SourceIdentifier $sourceId -ErrorAction SilentlyContinue | Remove-Event -ErrorAction SilentlyContinue
    }
    if ($writer) { $writer.Dispose() } elseif ($stream) { $stream.Dispose() }
}
if ($failed) { exit 1 }
exit 0
