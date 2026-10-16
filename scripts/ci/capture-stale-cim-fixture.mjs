import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { spawn } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const USAGE = `Windows stale-CIM teardown fixture

Usage:
  node capture-stale-cim-fixture.mjs CONTROLLER_SOURCE_ABSOLUTE_PATH FRESH_OUTPUT_ABSOLUTE_PATH
  node capture-stale-cim-fixture.mjs --help

Arguments:
  CONTROLLER_SOURCE_ABSOLUTE_PATH  Actual capture-windows-apps.mjs source file.
  FRESH_OUTPUT_ABSOLUTE_PATH      New directory for the injected controller and evidence.

Example:
  node capture-stale-cim-fixture.mjs /repo/scripts/ci/capture-windows-apps.mjs /tmp/stale-cim-run

The fixture requires Windows and pwsh. It never uses the GUI or process-tree termination.`

const EVENT_FILE = 'stale-cim-events.jsonl'
const KILL_EVENT_FILE = 'target-kill-event.json'
const FIXTURE_TIMEOUT_MS = 60000
const RUNNER_NEEDLE = "const result = spawnSync(powerShell, ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script], {"
const CONTROLLER_BOUNDARY = '\nlet exitCode = 1\n'

function delay (ms) {
    return new Promise(resolve => setTimeout(resolve, ms))
}

function parseArgs (argv) {
    if (argv.length === 1 && argv[0] === '--help') return { help: true }
    if (argv.length !== 2) return { error: 'expected the controller source path and fresh output path' }
    if (!path.isAbsolute(argv[0]) || !path.isAbsolute(argv[1])) return { error: 'both paths must be absolute' }
    return { sourcePath: argv[0], outputDirectory: argv[1] }
}

function sameIdentity (a, b) {
    return !!a && !!b && a.pid === b.pid && a.parentPid === b.parentPid && a.creationTimeUTC === b.creationTimeUTC
}

function isValidUtcTimestamp (value) {
    return typeof value === 'string' && /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{7}Z$/.test(value) && Number.isFinite(Date.parse(value))
}

function identityScript (pid) {
    return `$ErrorActionPreference = 'Stop'\n` +
        `$row = Get-CimInstance -ClassName Win32_Process -Filter 'ProcessId = ${pid}' -ErrorAction Stop\n` +
        `if (-not $row) { [Console]::Out.WriteLine('null'); exit 0 }\n` +
        `$identity = @{ pid = [int]$row.ProcessId; parentPid = [int]$row.ParentProcessId; creationTimeUTC = $row.CreationDate.ToUniversalTime().ToString('o') }\n` +
        `[Console]::Out.WriteLine((ConvertTo-Json -InputObject $identity -Compress))\n`
}

function parseIdentityResult (output, label) {
    if (!output.ok) throw new Error(`${label} PowerShell query failed: ${output.error || output.stderr || output.status}`)
    const identity = JSON.parse(output.stdout)
    if (identity !== null && (!Number.isSafeInteger(identity.pid) || !Number.isSafeInteger(identity.parentPid) || typeof identity.creationTimeUTC !== 'string')) {
        throw new Error(`${label} returned a malformed CIM identity`)
    }
    return identity
}

function makeInjection () {
    return String.raw`$script:tabbyFixtureTargetPid = [int]$env:TABBY_FIXTURE_TARGET_PID
$script:tabbyFixtureTargetCreation = ([DateTime]$env:TABBY_FIXTURE_TARGET_CREATION).ToUniversalTime()
$script:tabbyFixtureEventsPath = $env:TABBY_FIXTURE_EVENTS_PATH
$script:tabbyFixtureKillEventPath = $env:TABBY_FIXTURE_KILL_EVENT_PATH
$script:tabbyFixtureRealGetCimInstance = 'CimCmdlets\Get-CimInstance'
$script:tabbyFixtureTargetCimRow = $null
$script:tabbyFixtureTargetExited = $false
$script:tabbyFixtureStaleRowsDelivered = 0
$script:tabbyFixtureStaleRowsArmed = $false
$script:tabbyFixtureGetProcessTriggered = $false

function Get-CimInstance {
    [CmdletBinding()]
    param([string]$ClassName, [string]$Filter)
    $pidMatch = [regex]::Match([string]$Filter, 'ProcessId\s*=\s*(\d+)')
    $isTargetQuery = $ClassName -eq 'Win32_Process' -and $pidMatch.Success -and [int]$pidMatch.Groups[1].Value -eq $script:tabbyFixtureTargetPid
    if ($script:tabbyFixtureStaleRowsArmed -and $script:tabbyFixtureTargetExited -and $isTargetQuery -and $script:tabbyFixtureStaleRowsDelivered -lt 2) {
        $script:tabbyFixtureStaleRowsDelivered++
        $observedCreation = $script:tabbyFixtureTargetCimRow.CreationDate.ToUniversalTime().ToString('o')
        $event = [ordered]@{
            kind = 'stale-target-cim-row-delivered'
            sequence = [int]$script:tabbyFixtureStaleRowsDelivered
            pid = [int]$script:tabbyFixtureTargetCimRow.ProcessId
            creationTimeUTC = $observedCreation
            exactPreKillRow = ($script:tabbyFixtureTargetCimRow.CreationDate.ToUniversalTime().Ticks -eq $script:tabbyFixtureTargetCreation.Ticks)
            targetWaitedForExit = [bool]$script:tabbyFixtureTargetExited
        }
        [IO.File]::AppendAllText($script:tabbyFixtureEventsPath, (($event | ConvertTo-Json -Compress) + [Environment]::NewLine), [Text.UTF8Encoding]::new($false))
        return $script:tabbyFixtureTargetCimRow
    }
    if ($script:tabbyFixtureStaleRowsArmed -and $isTargetQuery -and $script:tabbyFixtureStaleRowsDelivered -ge 2) {
        $actualRows = @(& $script:tabbyFixtureRealGetCimInstance -ClassName $ClassName -Filter $Filter -ErrorAction Stop)
        $actualRow = if ($actualRows.Count) { $actualRows[0] } else { $null }
        $event = [ordered]@{
            kind = 'actual-target-cim-query-after-two-stale-rows'
            staleRowsDelivered = [int]$script:tabbyFixtureStaleRowsDelivered
            pid = [int]$script:tabbyFixtureTargetPid
            actualIdentity = if ($actualRow) { @{ pid = [int]$actualRow.ProcessId; parentPid = [int]$actualRow.ParentProcessId; creationTimeUTC = $actualRow.CreationDate.ToUniversalTime().ToString('o') } } else { $null }
        }
        [IO.File]::AppendAllText($script:tabbyFixtureEventsPath, (($event | ConvertTo-Json -Compress) + [Environment]::NewLine), [Text.UTF8Encoding]::new($false))
        return $actualRows
    }
    & $script:tabbyFixtureRealGetCimInstance -ClassName $ClassName -Filter $Filter -ErrorAction Stop
}

function Get-Process {
    [CmdletBinding()]
    param([int[]]$Id)
    if ($Id -contains $script:tabbyFixtureTargetPid -and -not $script:tabbyFixtureGetProcessTriggered) {
        $script:tabbyFixtureGetProcessTriggered = $true
        $rows = @(& $script:tabbyFixtureRealGetCimInstance -ClassName Win32_Process -Filter "ProcessId = $($script:tabbyFixtureTargetPid)" -ErrorAction Stop)
        if ($rows.Count -ne 1) { throw 'fault injection refused: target exact CIM row is unavailable before termination' }
        $before = $rows[0]
        if ([int]$before.ProcessId -ne $script:tabbyFixtureTargetPid -or $before.CreationDate.ToUniversalTime().Ticks -ne $script:tabbyFixtureTargetCreation.Ticks) {
            throw 'fault injection refused: target CIM creation identity did not match'
        }
        $real = @(Microsoft.PowerShell.Management\Get-Process -Id $script:tabbyFixtureTargetPid -ErrorAction Stop)
        if ($real.Count -ne 1 -or $real[0].Id -ne $script:tabbyFixtureTargetPid) { throw 'fault injection refused: expected one real target process handle' }
        $startUtc = $real[0].StartTime.ToUniversalTime()
        $startTimeDeltaTicks = [long][Math]::Abs(($startUtc - $before.CreationDate.ToUniversalTime()).Ticks)
        if ($startTimeDeltaTicks -gt 10000) {
            throw 'fault injection refused: held process handle creation identity did not match'
        }
        $script:tabbyFixtureTargetCimRow = $before
        $real[0].Kill()
        $waited = $real[0].WaitForExit(5000)
        $event = [ordered]@{
            kind = 'actual-get-process-killed-and-waited-for-target'
            pid = [int]$script:tabbyFixtureTargetPid
            cimCreationTimeUTC = $before.CreationDate.ToUniversalTime().ToString('o')
            processStartTimeUTC = $startUtc.ToString('o')
            processStartToCimDeltaTicks = $startTimeDeltaTicks
            processStartMatchesCimWithin1ms = ($startTimeDeltaTicks -le 10000)
            exitCode = if ($waited) { $real[0].ExitCode } else { $null }
            waitedForExit = [bool]$waited
        }
        [IO.File]::WriteAllText($script:tabbyFixtureKillEventPath, ($event | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new($false))
        if (-not $waited) { throw 'fault injection failed: real target process did not exit within five seconds' }
        $script:tabbyFixtureTargetExited = $true
        $script:tabbyFixtureStaleRowsArmed = $true
        Microsoft.PowerShell.Management\Get-Process -Id $script:tabbyFixtureTargetPid -ErrorAction Stop
        throw 'fault injection failed: module-qualified Get-Process unexpectedly found the exited target'
    }
    Microsoft.PowerShell.Management\Get-Process -Id $Id -ErrorAction Stop
}`
}

function makePostTeardownProbe () {
    return String.raw`$null = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = $($env:TABBY_FIXTURE_TARGET_PID)" -ErrorAction Stop`
}

function loadController (sourcePath, outputDirectory, injection, postTeardownProbe) {
    const source = readFileSync(sourcePath, 'utf8')
    const boundary = source.lastIndexOf(CONTROLLER_BOUNDARY)
    assert.ok(boundary > 0, 'actual controller entry boundary must exist')
    assert.equal(source.split(RUNNER_NEEDLE).length - 1, 1, 'actual PowerShell runner boundary must occur exactly once')
    const runnerReplacement = `const injectedScript = script.includes('$entries = @([Text.Encoding]') ? ${JSON.stringify(injection)} + "\\n" + script + "\\n" + ${JSON.stringify(postTeardownProbe)} : script\n    const result = spawnSync(powerShell, ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', injectedScript], {`
    const loadedSource = `${source.slice(0, boundary)}\nexport { snapshotProcessTree, teardownApp, runPowerShell };\n`
        .replace(RUNNER_NEEDLE, runnerReplacement)
    const loadedPath = path.join(outputDirectory, 'actual-controller-functions.mjs')
    writeFileSync(loadedPath, loadedSource, { flag: 'wx', encoding: 'utf8' })
    return { source, loadedPath }
}

function startWorker (powerShell, name, workers) {
    const child = spawn(powerShell, ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', 'Start-Sleep -Seconds 120'], {
        windowsHide: true,
        shell: false,
        stdio: ['ignore', 'pipe', 'pipe'],
    })
    const record = { name, child, spawnError: null, exit: null, closed: null }
    child.once('error', error => { record.spawnError = error.message })
    record.exited = new Promise(resolve => child.once('exit', (code, signal) => { record.exit = { code, signal }; resolve(record.exit) }))
    record.closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal, spawnError: record.spawnError })))
    workers.push(record)
    return record
}

async function findWorkerIdentity (snapshotProcessTree, powerShell, worker, deadline) {
    let last = { processes: [], error: 'worker process identity has not been observed' }
    while (Date.now() < deadline) {
        if (!worker.child.pid) throw new Error(`${worker.name} failed to spawn: ${worker.spawnError || 'missing process id'}`)
        last = snapshotProcessTree(powerShell, worker.child.pid, deadline)
        if (last.error) throw new Error(`${worker.name} process snapshot failed: ${last.error}`)
        const identity = last.processes.find(entry => entry.pid === worker.child.pid && entry.parentPid === process.pid)
        if (identity) return { identity, tree: last }
        await delay(100)
    }
    throw new Error(`${worker.name} exact child CIM identity did not appear before its deadline`)
}

async function queryExactIdentity (runPowerShell, powerShell, pid, deadline, label) {
    const output = runPowerShell(powerShell, identityScript(pid), Math.max(1000, deadline - Date.now()))
    return parseIdentityResult(output, label)
}

async function observeExactPidUntilGone (runPowerShell, powerShell, identity, deadline) {
    const observations = []
    const pollDeadline = Math.min(deadline, Date.now() + 10000)
    while (Date.now() < pollDeadline) {
        try {
            const current = await queryExactIdentity(runPowerShell, powerShell, identity.pid, Math.min(pollDeadline, Date.now() + 5000), 'post-teardown target')
            observations.push({ observedAt: new Date().toISOString(), identity: current })
            if (!sameIdentity(identity, current)) return { observations, identity: current, gone: true }
        } catch (error) {
            observations.push({ observedAt: new Date().toISOString(), error: error.message })
            return { observations, error: error.message, gone: false }
        }
        await delay(100)
    }
    return { observations, identity: observations.at(-1)?.identity ?? null, gone: false, timedOut: true }
}

async function reapWorkers (workers) {
    const records = []
    for (const worker of workers) {
        let killAttempted = false
        if (worker.child.exitCode === null && worker.child.signalCode === null && worker.child.pid) {
            killAttempted = true
            try { worker.child.kill() } catch { }
        }
        let close = null
        try {
            close = await Promise.race([
                worker.closed,
                delay(6000).then(() => { throw new Error(`${worker.name} did not close within cleanup deadline`) }),
            ])
        } catch (error) {
            close = { error: error.message }
            worker.child.stdout?.destroy()
            worker.child.stderr?.destroy()
        }
        records.push({
            name: worker.name,
            pid: worker.child.pid ?? null,
            killAttempted,
            exit: worker.exit,
            close,
            reaped: !!worker.exit && !!close && !close.error,
        })
    }
    return records
}

async function runFixture (sourcePath, outputDirectory) {
    const workers = []
    const startedAt = new Date().toISOString()
    const deadline = Date.now() + FIXTURE_TIMEOUT_MS
    const result = {
        kind: 'actual-controller-stale-cim-cleanup',
        status: 'FAIL',
        platform: process.platform,
        startedAt,
        controllerSourcePath: sourcePath,
        outputDirectory,
        controllerSha256: null,
        targetIdentity: null,
        siblingIdentity: null,
        injection: { targetKill: null, staleRowsDelivered: [], actualCimAfterStaleRows: null, eventFiles: [KILL_EVENT_FILE, EVENT_FILE] },
        teardown: null,
        observations: { targetIdentityAfterTeardown: null, targetSnapshotAfterTeardown: null, siblingIdentityAfterTeardown: null, siblingSnapshotAfterTeardown: null },
        assertions: {},
        fixtureCleanup: [],
        error: null,
        unverified: ['This fixture does not exercise the production application or GUI.'],
    }
    const envNames = ['TABBY_FIXTURE_TARGET_PID', 'TABBY_FIXTURE_TARGET_CREATION', 'TABBY_FIXTURE_EVENTS_PATH', 'TABBY_FIXTURE_KILL_EVENT_PATH']
    const oldEnvironment = new Map(envNames.map(name => [name, process.env[name]]))
    try {
        const { source, loadedPath } = loadController(sourcePath, outputDirectory, makeInjection(), makePostTeardownProbe())
        result.controllerSha256 = createHash('sha256').update(source).digest('hex')
        const { snapshotProcessTree, teardownApp, runPowerShell } = await import(pathToFileURL(loadedPath))
        const powerShell = 'pwsh'
        const targetWorker = startWorker(powerShell, 'target', workers)
        const siblingWorker = startWorker(powerShell, 'sibling', workers)
        const observationDeadline = Math.min(deadline - 20000, Date.now() + 15000)
        const targetObservation = await findWorkerIdentity(snapshotProcessTree, powerShell, targetWorker, observationDeadline)
        const siblingObservation = await findWorkerIdentity(snapshotProcessTree, powerShell, siblingWorker, observationDeadline)
        result.targetIdentity = targetObservation.identity
        result.siblingIdentity = siblingObservation.identity
        assert.notEqual(result.targetIdentity.pid, result.siblingIdentity.pid, 'target and sibling must have different process identities')
        assert.equal(result.targetIdentity.parentPid, process.pid, 'target must be an owned direct pwsh child')
        assert.equal(result.siblingIdentity.parentPid, process.pid, 'sibling must be an independent owned direct pwsh child')
        writeFileSync(path.join(outputDirectory, EVENT_FILE), '', { flag: 'wx', encoding: 'utf8' })

        process.env.TABBY_FIXTURE_TARGET_PID = String(result.targetIdentity.pid)
        process.env.TABBY_FIXTURE_TARGET_CREATION = result.targetIdentity.creationTimeUTC
        process.env.TABBY_FIXTURE_EVENTS_PATH = path.join(outputDirectory, EVENT_FILE)
        process.env.TABBY_FIXTURE_KILL_EVENT_PATH = path.join(outputDirectory, KILL_EVENT_FILE)
        try {
            result.teardown = await teardownApp({ powerShell, cleanupDeadline: deadline }, {
                role: 'stale-cim-cleanup-fixture',
                pid: result.targetIdentity.pid,
                rootIdentity: result.targetIdentity,
                initialProcessTree: targetObservation.tree,
            })
        } catch (error) {
            result.error = { stage: 'actual-controller-teardown', message: error?.message || String(error), stack: error?.stack || null }
        }

        if (existsSync(path.join(outputDirectory, KILL_EVENT_FILE))) {
            result.injection.targetKill = JSON.parse(readFileSync(path.join(outputDirectory, KILL_EVENT_FILE), 'utf8'))
        }
        const injectionEvents = readFileSync(path.join(outputDirectory, EVENT_FILE), 'utf8')
            .split(/\r?\n/).filter(Boolean).map(line => JSON.parse(line))
        result.injection.staleRowsDelivered = injectionEvents.filter(event => event.kind === 'stale-target-cim-row-delivered')
        result.injection.actualCimAfterStaleRows = injectionEvents.find(event => event.kind === 'actual-target-cim-query-after-two-stale-rows') ?? null

        try {
            const targetObservation = await observeExactPidUntilGone(runPowerShell, powerShell, result.targetIdentity, deadline)
            result.observations.targetIdentityHistory = targetObservation.observations
            result.observations.targetIdentityAfterTeardown = targetObservation.identity
            if (targetObservation.error) result.observations.targetIdentityQueryError = targetObservation.error
            if (!targetObservation.gone && !targetObservation.error) result.observations.targetIdentityStillPresentOrUnverified = true
        } catch (error) {
            result.observations.targetIdentityQueryError = error.message
        }
        try {
            result.observations.targetSnapshotAfterTeardown = snapshotProcessTree(powerShell, result.targetIdentity.pid, deadline)
        } catch (error) {
            result.observations.targetSnapshotQueryError = error.message
        }
        try {
            result.observations.siblingIdentityAfterTeardown = await queryExactIdentity(runPowerShell, powerShell, result.siblingIdentity.pid, deadline, 'post-teardown sibling')
        } catch (error) {
            result.observations.siblingIdentityQueryError = error.message
        }
        try {
            result.observations.siblingSnapshotAfterTeardown = snapshotProcessTree(powerShell, result.siblingIdentity.pid, deadline)
        } catch (error) {
            result.observations.siblingSnapshotQueryError = error.message
        }

        const staleRows = result.injection.staleRowsDelivered
        const exactStaleRows = staleRows.every(row => row.pid === result.targetIdentity.pid && row.creationTimeUTC === result.targetIdentity.creationTimeUTC && row.exactPreKillRow && row.targetWaitedForExit)
        result.assertions = {
            actualTargetKillAndWait: result.injection.targetKill?.pid === result.targetIdentity.pid && result.injection.targetKill?.waitedForExit === true && result.injection.targetKill?.cimCreationTimeUTC === result.targetIdentity.creationTimeUTC && isValidUtcTimestamp(result.injection.targetKill?.cimCreationTimeUTC) && isValidUtcTimestamp(result.injection.targetKill?.processStartTimeUTC) && Number.isSafeInteger(result.injection.targetKill?.processStartToCimDeltaTicks) && result.injection.targetKill.processStartToCimDeltaTicks >= 0 && result.injection.targetKill.processStartToCimDeltaTicks <= 10000 && result.injection.targetKill.processStartMatchesCimWithin1ms === true,
            atLeastTwoExactStaleCimRowsDelivered: staleRows.length >= 2 && exactStaleRows,
            wrapperReturnsActualCimAfterTwoStaleRows: result.injection.actualCimAfterStaleRows?.staleRowsDelivered === 2,
            targetAbsentInActualCimQuery: !Object.hasOwn(result.observations, 'targetIdentityQueryError') && !result.observations.targetIdentityStillPresentOrUnverified && result.observations.targetIdentityHistory?.length > 0 && !sameIdentity(result.targetIdentity, result.observations.targetIdentityAfterTeardown),
            targetAbsentInFreshProcessSnapshot: !!result.observations.targetSnapshotAfterTeardown && result.observations.targetSnapshotAfterTeardown.error === null && Array.isArray(result.observations.targetSnapshotAfterTeardown.processes) && !result.observations.targetSnapshotAfterTeardown.processes.some(entry => sameIdentity(entry, result.targetIdentity)),
            siblingPreservedInActualCimQuery: sameIdentity(result.observations.siblingIdentityAfterTeardown, result.siblingIdentity),
            siblingPreservedInFreshProcessSnapshot: !!result.observations.siblingSnapshotAfterTeardown && result.observations.siblingSnapshotAfterTeardown.error === null && Array.isArray(result.observations.siblingSnapshotAfterTeardown.processes) && result.observations.siblingSnapshotAfterTeardown.processes.some(entry => sameIdentity(entry, result.siblingIdentity)),
            teardownSurvivorsEmpty: Array.isArray(result.teardown?.survivors) && result.teardown.survivors.length === 0,
            teardownErrorNull: result.teardown?.error === null,
        }
    } catch (error) {
        result.error ||= { stage: 'fixture', message: error?.message || String(error), stack: error?.stack || null }
    } finally {
        for (const [name, value] of oldEnvironment) {
            if (value === undefined) delete process.env[name]
            else process.env[name] = value
        }
        result.fixtureCleanup = await reapWorkers(workers)
        result.finishedAt = new Date().toISOString()
        const requiredAssertions = Object.values(result.assertions)
        const cleanupPassed = result.fixtureCleanup.length === workers.length && result.fixtureCleanup.every(item => item.reaped)
        result.assertions.fixtureChildrenReaped = cleanupPassed
        result.status = !result.error && requiredAssertions.length === 9 && requiredAssertions.every(Boolean) && cleanupPassed ? 'PASS' : 'FAIL'
        writeFileSync(path.join(outputDirectory, 'result.json'), `${JSON.stringify(result, null, 2)}\n`, { flag: 'wx', encoding: 'utf8' })
    }
    return result
}

async function main () {
    const args = parseArgs(process.argv.slice(2))
    if (args.help) {
        process.stdout.write(`${USAGE}\n`)
        return 0
    }
    if (args.error) {
        process.stderr.write(`${args.error}\n\n${USAGE}\n`)
        return 2
    }
    if (process.platform !== 'win32') {
        process.stdout.write(`${JSON.stringify({ status: 'SKIP', nativePass: false, platform: process.platform, reason: 'Windows CIM and pwsh process handles are required; no fixture ran.' }, null, 2)}\n`)
        return 0
    }
    if (!existsSync(args.sourcePath)) {
        process.stderr.write(`controller source does not exist: ${args.sourcePath}\n`)
        return 2
    }
    if (existsSync(args.outputDirectory)) {
        process.stderr.write(`refusing existing output directory: ${args.outputDirectory}\n`)
        return 2
    }
    mkdirSync(args.outputDirectory)
    const result = await runFixture(args.sourcePath, args.outputDirectory)
    process.stdout.write(`${JSON.stringify({ status: result.status, resultFile: path.join(args.outputDirectory, 'result.json'), assertions: result.assertions, error: result.error }, null, 2)}\n`)
    return result.status === 'PASS' ? 0 : 1
}

main().then(code => { process.exitCode = code }).catch(error => {
    process.stderr.write(`${error?.stack || error}\n`)
    process.exitCode = 1
})
