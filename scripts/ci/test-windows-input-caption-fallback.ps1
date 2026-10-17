<#
.SYNOPSIS
Checks the actual caption-discovery fallback predicate without sending input.
.DESCRIPTION
Runs positive and negative cases against the helper's current predicate. This
does not establish native UI Automation or physical mouse acceptance.
.PARAMETER HelperPath
Path to exercise-windows-window-input.ps1.
.EXAMPLE
pwsh -NoProfile -File ./test-windows-input-caption-fallback.ps1
#>
[CmdletBinding()]
param([string]$HelperPath = (Join-Path $PSScriptRoot 'exercise-windows-window-input.ps1'))
$ErrorActionPreference = 'Stop'
$source = Get-Content -LiteralPath $HelperPath -Raw
$matches = [regex]::Matches($source, '(?m)^\s*if \((?<condition>-not \$native\[''ok''\].+)\) \{$')
if ($matches.Count -ne 1) { throw 'Expected exactly one actual caption fallback predicate' }
$probe = [scriptblock]::Create('param($native,$Action)' + [Environment]::NewLine + $matches[0].Groups['condition'].Value)
$knownErrors = @('InvalidOperationException: caption button rectangle is empty', 'InvalidOperationException: caption center is not owned root HWND')
$checks = 0
foreach ($message in $knownErrors) {
    foreach ($action in @('Minimize','Maximize','Restore','Close')) {
        foreach ($ok in @($false,$true)) {
            foreach ($sent in @($false,$true)) {
                $actual = & $probe @{ok=$ok;inputSent=$sent;error=$message} $action
                $expected = -not $ok -and -not $sent
                if ($actual -ne $expected) { throw "Fallback predicate mismatch for $action, ok=$ok, inputSent=$sent" }
                $checks++
            }
        }
    }
    foreach ($action in @('Resize','Drag','Snapshot','RecoverMouse')) {
        if (& $probe @{ok=$false;inputSent=$false;error=$message} $action) { throw "Non-caption action $action entered caption discovery" }
        $checks++
    }
}
foreach ($message in @($null,'','InvalidOperationException: foreign window','InvalidOperationException: foreground target changed','InvalidOperationException: WM_NCHITTEST timed out','invalidoperationexception: caption center is not owned root hwnd')) {
    if (& $probe @{ok=$false;inputSent=$false;error=$message} 'Maximize') { throw 'Unrecognized error entered caption discovery' }
    $checks++
}
Write-Output "Caption discovery fallback: $checks controlled checks passed; no input was sent"
