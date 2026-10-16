import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { spawn } from 'node:child_process'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
const [sourcePath, output] = process.argv.slice(2)
if (!sourcePath || !output || process.platform !== 'win32' || !path.isAbsolute(sourcePath) || !path.isAbsolute(output)) throw new Error('Windows fixture requires absolute source and fresh output paths')
mkdirSync(output)
const source = readFileSync(sourcePath, 'utf8')
const boundary = source.lastIndexOf('\nlet exitCode = 1\n')
assert.ok(boundary > 0, 'actual controller entry boundary must exist')
const loadedSource = `${source.slice(0, boundary)}\nexport { snapshotProcessTree, teardownApp, runPowerShell };\n`
const loadedPath = path.join(output, 'actual-controller-functions.mjs')
writeFileSync(loadedPath, loadedSource, { flag: 'wx' })
const { snapshotProcessTree, teardownApp, runPowerShell } = await import(pathToFileURL(loadedPath))
const workers = []
const result = { kind: 'actual-windows-capture-process-cleanup', ok: false, productionAppVerified: false, controllerSha256: createHash('sha256').update(source).digest('hex'), error: null }
const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
function startWorker () {
    const p = spawn('pwsh', ['-NoProfile', '-Command', 'Start-Sleep -Seconds 120'], { windowsHide: true, shell: false, stdio: ['ignore', 'pipe', 'pipe'] })
    p.on('error', error => { result.spawnError = error.message })
    workers.push(p)
    return p
}
try {
    const target = startWorker()
    const sibling = startWorker()
    await delay(500)
    const context = { powerShell: 'pwsh', cleanupDeadline: Date.now() + 30000 }
    const initial = snapshotProcessTree('pwsh', target.pid, context.cleanupDeadline)
    const identity = initial.processes.find(p => p.pid === target.pid && p.parentPid === process.pid)
    assert.ok(identity, 'actual owned worker identity must be observed')
    const siblingInitial = snapshotProcessTree('pwsh', sibling.pid, context.cleanupDeadline)
    const siblingIdentity = siblingInitial.processes.find(p => p.pid === sibling.pid)
    assert.ok(siblingIdentity, 'independent control worker must be observed')
    const dateProbe = runPowerShell('pwsh', `$ErrorActionPreference='Stop'
$p = Get-CimInstance Win32_Process -Filter "ProcessId = $PID"
$original = $p.CreationDate.ToUniversalTime().ToString('o')
$entry = @{ creationTimeUTC=$original } | ConvertTo-Json -Compress | ConvertFrom-Json
@{ ps=$PSVersionTable.PSVersion.ToString(); type=$entry.creationTimeUTC.GetType().FullName; original=$original; wrongStringComparison=($original -eq $entry.creationTimeUTC); ticksMatch=($p.CreationDate.ToUniversalTime().Ticks -eq ([DateTime]$entry.creationTimeUTC).ToUniversalTime().Ticks) } | ConvertTo-Json -Compress`, 10000)
    result.dateProbe = dateProbe.ok ? JSON.parse(dateProbe.stdout) : dateProbe
    result.identity = identity
    result.cleanup = await teardownApp(context, { role: 'owned-fixture', pid: target.pid, rootIdentity: identity, initialProcessTree: initial })
    await delay(300)
    result.after = snapshotProcessTree('pwsh', target.pid, Date.now() + 10000)
    result.siblingAfter = snapshotProcessTree('pwsh', sibling.pid, Date.now() + 10000)
    result.actualOriginalStillAlive = result.after.processes.some(p => p.pid === identity.pid && p.creationTimeUTC === identity.creationTimeUTC)
    result.siblingPreserved = result.siblingAfter.processes.some(p => p.pid === siblingIdentity.pid && p.creationTimeUTC === siblingIdentity.creationTimeUTC)
    assert.equal(result.actualOriginalStillAlive, false, 'cleanup must actually stop the original worker; an empty survivor list alone is insufficient')
    assert.equal(result.siblingPreserved, true, 'cleanup must preserve the independent worker')
    assert.equal(result.cleanup.error, null)
    assert.deepEqual(result.cleanup.survivors, [])
    assert.ok(result.cleanup.termination.stopped.includes(identity.pid), 'actual original worker must be in confirmed stopped results')
    result.ok = true
} catch (error) { result.error = error.stack }
finally {
    result.fixtureCleanup = []
    for (const p of workers) {
        if (p.exitCode === null && p.signalCode === null) { p.kill(); await delay(100) }
        p.stdout?.destroy(); p.stderr?.destroy(); p.unref()
        result.fixtureCleanup.push({pid:p.pid, exitCode:p.exitCode, signal:p.signalCode})
    }
    writeFileSync(path.join(output, 'result.json'), JSON.stringify(result, null, 2), { flag: 'wx' })
}
console.log(JSON.stringify({kind:result.kind,ok:result.ok,error:result.error,dateProbe:result.dateProbe,actualOriginalStillAlive:result.actualOriginalStillAlive,siblingPreserved:result.siblingPreserved}))
process.exit(result.ok ? 0 : 1)
