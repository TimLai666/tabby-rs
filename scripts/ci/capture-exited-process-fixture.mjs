import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { spawn } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const [sourcePath, outputDirectory] = process.argv.slice(2)
if (!sourcePath || !outputDirectory || process.platform !== 'win32' || !path.isAbsolute(sourcePath) || !path.isAbsolute(outputDirectory)) {
    throw new Error('Windows fixture requires absolute source and fresh output paths')
}
if (existsSync(outputDirectory)) throw new Error(`refusing existing output directory: ${outputDirectory}`)
mkdirSync(outputDirectory)

const source = readFileSync(sourcePath, 'utf8')
const boundary = source.lastIndexOf('\nlet exitCode = 1\n')
assert.ok(boundary > 0, 'actual controller entry boundary must exist')
const eventPath = path.join(outputDirectory, 'race-trigger.json')
const injection = String.raw`function Get-Process {
    [CmdletBinding()]
    param([int[]]$Id)
    if ($Id -contains [int]$env:TABBY_FIXTURE_TARGET_PID -and -not $script:tabbyFixtureRaceTriggered) {
        $script:tabbyFixtureRaceTriggered = $true
        $before = Get-CimInstance Win32_Process -Filter "ProcessId = $($env:TABBY_FIXTURE_TARGET_PID)" -ErrorAction Stop
        if (-not $before -or $before.CreationDate.ToUniversalTime().Ticks -ne ([DateTime]$env:TABBY_FIXTURE_TARGET_CREATION).ToUniversalTime().Ticks) {
            throw 'fault injection refused: target CIM creation identity did not match'
        }
        $real = Microsoft.PowerShell.Management\Get-Process -Id ([int]$env:TABBY_FIXTURE_TARGET_PID) -ErrorAction Stop
        if ($real.Count -ne 1 -or $real[0].Id -ne [int]$env:TABBY_FIXTURE_TARGET_PID) {
            throw 'fault injection refused: expected one real target process handle'
        }
        $startUtc = $real[0].StartTime.ToUniversalTime()
        if ([Math]::Abs(($startUtc - $before.CreationDate.ToUniversalTime()).TotalMilliseconds) -gt 1) {
            throw 'fault injection refused: held process handle creation identity did not match'
        }
        $real[0].Kill()
        $waited = $real[0].WaitForExit(5000)
        $event = [ordered]@{
            kind = 'actual-get-process-exited-between-cim-and-handle-lookup'
            pid = [int]$env:TABBY_FIXTURE_TARGET_PID
            cimCreationTimeUTC = $before.CreationDate.ToUniversalTime().ToString('o')
            processStartTimeUTC = $startUtc.ToString('o')
            exitCode = if ($waited) { $real[0].ExitCode } else { $null }
            waitedForExit = [bool]$waited
        }
        [IO.File]::WriteAllText($env:TABBY_FIXTURE_EVENT_PATH, ($event | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new($false))
        if (-not $waited) { throw 'fault injection failed: real target process did not exit within five seconds' }
        Microsoft.PowerShell.Management\Get-Process -Id ([int]$env:TABBY_FIXTURE_TARGET_PID) -ErrorAction Stop
        throw 'fault injection failed: module-qualified Get-Process unexpectedly found the exited target'
    }
    Microsoft.PowerShell.Management\Get-Process -Id $Id -ErrorAction Stop
}`
const runPowerShellNeedle = "const result = spawnSync(powerShell, ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script], {"
assert.equal(source.split(runPowerShellNeedle).length - 1, 1, 'actual PowerShell runner boundary must occur exactly once')
const loadedSource = `${source.slice(0, boundary)}\nexport { snapshotProcessTree, teardownApp, runPowerShell };\n`
    .replace(runPowerShellNeedle, `const injectedScript = script.includes('$entries = @([Text.Encoding]') ? ${JSON.stringify(injection)} + "\\n" + script : script\n    const result = spawnSync(powerShell, ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', injectedScript], {`)
const loadedPath = path.join(outputDirectory, 'actual-controller-functions.mjs')
writeFileSync(loadedPath, loadedSource, { flag: 'wx' })
const { snapshotProcessTree, teardownApp } = await import(pathToFileURL(loadedPath))

const workers = []
const result = {
    kind: 'actual-windows-exited-process-cleanup-race',
    ok: false,
    productionAppVerified: false,
    applicationQuitAccepted: false,
    controllerSha256: createHash('sha256').update(source).digest('hex'),
    error: null,
    raceEventPath: eventPath,
    raceEvent: null,
    targetIdentity: null,
    siblingIdentity: null,
    cleanup: null,
    targetAbsentAfterCleanup: null,
    siblingPreserved: null,
    unverified: ['This fixture uses real pwsh workers, not the production application.', 'Application Quit behavior and desktop acceptance are not verified.'],
}
const watchdog = setTimeout(() => {
    result.error ||= '45-second fixture watchdog expired'
    for (const child of workers) {
        if (child.exitCode === null && child.signalCode === null) child.kill()
        child.stdout?.destroy(); child.stderr?.destroy(); child.unref()
    }
    try { writeFileSync(path.join(outputDirectory, 'result.json'), JSON.stringify(result, null, 2), { flag: 'wx' }) } catch {}
    process.exit(1)
}, 45000)
watchdog.unref()

function startWorker () {
    const child = spawn('pwsh', ['-NoProfile', '-Command', 'Start-Sleep -Seconds 120'], {
        windowsHide: true,
        shell: false,
        stdio: ['ignore', 'pipe', 'pipe'],
    })
    workers.push(child)
    child.on('error', error => { result.spawnError = error.message })
    return child
}

try {
    const target = startWorker()
    const sibling = startWorker()
    const startedBy = Date.now()
    const deadline = startedBy + 35000
    let initial = { processes: [], error: 'worker process identity has not been observed' }
    let siblingInitial = { processes: [], error: 'sibling process identity has not been observed' }
    while (Date.now() < Math.min(deadline, startedBy + 10000)) {
        if (!target.pid || !sibling.pid) throw new Error(`pwsh worker spawn failed: ${result.spawnError || 'missing process id'}`)
        initial = snapshotProcessTree('pwsh', target.pid, deadline)
        siblingInitial = snapshotProcessTree('pwsh', sibling.pid, deadline)
        if (initial.processes.some(entry => entry.pid === target.pid) && siblingInitial.processes.some(entry => entry.pid === sibling.pid)) break
        await new Promise(resolve => setTimeout(resolve, 100))
    }
    const targetIdentity = initial.processes.find(entry => entry.pid === target.pid && entry.parentPid === process.pid)
    const siblingIdentity = siblingInitial.processes.find(entry => entry.pid === sibling.pid && entry.parentPid === process.pid)
    assert.ok(targetIdentity, 'real target worker CIM identity must include its exact parent and creation time')
    assert.ok(siblingIdentity, 'real sibling worker CIM identity must include its exact parent and creation time')
    result.targetIdentity = targetIdentity
    result.siblingIdentity = siblingIdentity
    assert.equal(existsSync(eventPath), false, 'race event path must be fresh before teardown')

    process.env.TABBY_FIXTURE_TARGET_PID = String(targetIdentity.pid)
    process.env.TABBY_FIXTURE_TARGET_CREATION = targetIdentity.creationTimeUTC
    process.env.TABBY_FIXTURE_EVENT_PATH = eventPath
    const context = { powerShell: 'pwsh', cleanupDeadline: deadline }
    const cleanup = await teardownApp(context, {
        role: 'exited-process-race-fixture',
        pid: targetIdentity.pid,
        rootIdentity: targetIdentity,
        initialProcessTree: initial,
    })
    result.cleanup = cleanup
    if (existsSync(eventPath)) result.raceEvent = JSON.parse(readFileSync(eventPath, 'utf8'))

    const afterTarget = snapshotProcessTree('pwsh', targetIdentity.pid, deadline)
    const afterSibling = snapshotProcessTree('pwsh', siblingIdentity.pid, deadline)
    assert.equal(afterTarget.error, null, 'fresh target process observation must succeed')
    assert.equal(afterSibling.error, null, 'fresh sibling process observation must succeed')
    result.targetAbsentAfterCleanup = !afterTarget.processes.some(entry => entry.pid === targetIdentity.pid && entry.creationTimeUTC === targetIdentity.creationTimeUTC)
    result.siblingPreserved = afterSibling.processes.some(entry => entry.pid === siblingIdentity.pid && entry.creationTimeUTC === siblingIdentity.creationTimeUTC)

    assert.ok(result.raceEvent, 'fault injection must record the actual target exit race')
    assert.equal(result.raceEvent.kind, 'actual-get-process-exited-between-cim-and-handle-lookup')
    assert.equal(result.raceEvent.pid, targetIdentity.pid)
    assert.equal(result.raceEvent.cimCreationTimeUTC, targetIdentity.creationTimeUTC)
    assert.equal(result.raceEvent.waitedForExit, true)
    assert.equal(result.targetAbsentAfterCleanup, true, 'the original real worker must be absent after cleanup')
    assert.equal(result.siblingPreserved, true, 'cleanup must preserve the independently identified sibling')
    assert.equal(cleanup.error, null, 'the exited-process race must not become a cleanup error')
    assert.deepEqual(cleanup.survivors, [])
    result.ok = true
} catch (error) {
    result.error = error.stack || String(error)
} finally {
    clearTimeout(watchdog)
    delete process.env.TABBY_FIXTURE_TARGET_PID
    delete process.env.TABBY_FIXTURE_TARGET_CREATION
    delete process.env.TABBY_FIXTURE_EVENT_PATH
    result.fixtureCleanup = []
    for (const child of workers) {
        if (child.exitCode === null && child.signalCode === null) child.kill()
        child.stdout?.destroy(); child.stderr?.destroy(); child.unref()
        result.fixtureCleanup.push({ pid: child.pid, exitCode: child.exitCode, signal: child.signalCode })
    }
    writeFileSync(path.join(outputDirectory, 'result.json'), JSON.stringify(result, null, 2), { flag: 'wx' })
}
console.log(JSON.stringify({ kind: result.kind, ok: result.ok, error: result.error, raceEvent: result.raceEvent, targetAbsentAfterCleanup: result.targetAbsentAfterCleanup, siblingPreserved: result.siblingPreserved }))
process.exit(result.ok ? 0 : 1)
