<#
.SYNOPSIS
Checks the actual restart identity filters with JSON-decoded native timestamps.
.DESCRIPTION
Uses controlled process rows to verify live, missing, and reused identities.
This test sends no input and does not terminate any process.
.PARAMETER SourcePath
Path to capture-windows-apps.mjs containing the actual PowerShell query.
.EXAMPLE
pwsh -NoProfile -File ./test-windows-restart-identity.ps1
#>
[CmdletBinding()]
param([string]$SourcePath = (Join-Path $PSScriptRoot 'capture-windows-apps.mjs'))
$ErrorActionPreference = 'Stop'
$source = Get-Content -LiteralPath $SourcePath -Raw
$lines = foreach ($name in @('root','remaining','remainingTracked')) {
    $found = [regex]::Matches($source, '(?m)^\$' + $name + '=.+$')
    if ($found.Count -ne 1) { throw "Missing unique actual $name identity filter" }
    $found[0].Value
}
$probe = [scriptblock]::Create('param($all,$cfg)' + [Environment]::NewLine + ($lines -join [Environment]::NewLine) + [Environment]::NewLine + '@($root.Count,$remaining.Count,$remainingTracked.Count)')
$stamp = '2026-10-17T02:47:22.1157510Z'
$when = [DateTime]::ParseExact($stamp, 'o', [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::RoundtripKind)
$json = @{identity=@{pid=101;creationUtc=$stamp};sessions=@(@{pid=201;creationUtc=$stamp});tracked=@(@{pid=301;creationTimeUTC=$stamp})}|ConvertTo-Json -Depth 6 -Compress
$decoded = $json | ConvertFrom-Json
$literal = [pscustomobject]@{identity=[pscustomobject]@{pid=101;creationUtc=$stamp};sessions=@([pscustomobject]@{pid=201;creationUtc=$stamp});tracked=@([pscustomobject]@{pid=301;creationTimeUTC=$stamp})}
$rows = @(@{ProcessId=101;CreationDate=$when},@{ProcessId=201;CreationDate=$when},@{ProcessId=301;CreationDate=$when})
$cases = @(
    @{name='live';rows=$rows;expected=@(1,1,1)},
    @{name='all gone';rows=@();expected=@(0,0,0)},
    @{name='root only';rows=@($rows[0]);expected=@(1,0,0)},
    @{name='session only';rows=@($rows[1]);expected=@(0,1,0)},
    @{name='tracked only';rows=@($rows[2]);expected=@(0,0,1)},
    @{name='PID reused one tick later';rows=@(@{ProcessId=101;CreationDate=$when.AddTicks(1)},@{ProcessId=201;CreationDate=$when.AddTicks(1)},@{ProcessId=301;CreationDate=$when.AddTicks(1)});expected=@(0,0,0)},
    @{name='other PID';rows=@(@{ProcessId=401;CreationDate=$when});expected=@(0,0,0)}
)
$checks=0
foreach ($cfgValue in @($decoded,$literal)) {
    foreach ($case in $cases) {
        $actual = @(& $probe $case.rows $cfgValue)
        if (($actual -join ',') -cne ($case.expected -join ',')) { throw "Restart identity mismatch: $($case.name); expected $($case.expected -join ','); got $($actual -join ',')" }
        $checks++
    }
}
$condition = [regex]::Matches($source, 'present=\((?<condition>\$null -ne \$p -and .+?)\)\}')
if ($condition.Count -ne 1) { throw 'Missing unique actual initial CLI child presence filter' }
$childProbe = [scriptblock]::Create('param($p,$x)' + [Environment]::NewLine + $condition[0].Groups['condition'].Value)
foreach ($childIdentity in @((@{pid=101;creationUtc=$stamp}|ConvertTo-Json -Compress|ConvertFrom-Json),[pscustomobject]@{pid=101;creationUtc=$stamp})) {
    foreach ($case in @(@{row=$rows[0];expected=$true},@{row=$null;expected=$false},@{row=@{ProcessId=101;CreationDate=$when.AddTicks(1)};expected=$false})) {
        if ((& $childProbe $case.row $childIdentity) -ne $case.expected) { throw 'Initial CLI child identity mismatch' }
        $checks++
    }
}
Write-Output "Restart native identity: $checks controlled checks passed; JSON timestamp type=$($decoded.identity.creationUtc.GetType().FullName); no process was terminated"
