/** QA-only native start history. The owned child is always stopped and reaped. */
import { spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
const HERE = path.dirname(fileURLToPath(import.meta.url))

export async function startRestartProcessObserver ({ powerShell, directory, deadline }) {
    if (!path.isAbsolute(directory) || !Number.isFinite(deadline) || deadline - Date.now() < 10000) throw new Error('invalid bounded native observer options')
    const nonce = randomUUID()
    const eventsFile = path.join(directory, 'process-start-events.jsonl')
    const readyFile = path.join(directory, 'process-observer-ready.json')
    const stopFile = path.join(directory, 'process-observer-stop')
    for (const file of [eventsFile, readyFile, stopFile]) if (existsSync(file)) throw new Error('native observer files must be fresh')
    const child = spawn(powerShell, ['-NoProfile', '-NonInteractive', '-File', path.join(HERE, 'observe-windows-restart.ps1'),
        '-EventsFile', eventsFile, '-ReadyFile', readyFile, '-StopFile', stopFile, '-Nonce', nonce, '-MaximumSeconds', '180'],
    { windowsHide: true, shell: false, stdio: ['ignore', 'pipe', 'pipe'] })
    let ended = false, spawnError = null, stderr = '', stdout = '', closed = false
    child.stdout.on('data', bytes => { stdout = (stdout + bytes.toString('utf8')).slice(-4000) })
    child.stderr.on('data', bytes => { stderr = (stderr + bytes.toString('utf8')).slice(-4000) })
    const completion = new Promise(resolve => {
        child.once('error', error => { spawnError = error })
        child.once('close', (code, signal) => { ended = true; resolve({ code, signal }) })
    })
    let ready
    const descriptor = { pid: child.pid, nonce, eventsFile, readyFile, stopFile, ready: null, cleanup: null }
    const trace = () => {
        if (spawnError || ended) throw new Error('native observer exited before observation finished: ' + (spawnError?.message || stderr))
        if (!ready || !existsSync(eventsFile)) throw new Error('native observer is not ready')
        const bytes = readFileSync(eventsFile, 'utf8')
        const lastNewline = bytes.lastIndexOf('\n')
        if (lastNewline < 0) throw new Error('native observer has no complete records')
        const records = bytes.slice(0, lastNewline).split('\n').filter(Boolean).map(line => JSON.parse(line))
        if (records.some(row => row.nonce !== nonce)) throw new Error('native observer record nonce mismatch')
        const header = records[0]
        if (header.kind !== 'ready' || header.pid !== child.pid || header.creationTimeUTC !== ready.creationTimeUTC || header.readyUtc !== ready.readyUtc) throw new Error('native observer identity mismatch')
        const events = []
        let heartbeat
        for (const row of records.slice(1)) {
            if (row.kind === 'start') events.push({ pid: row.pid, parentPid: row.parentPid, fileTime: row.fileTime })
            else if (row.kind === 'heartbeat') {
                if (row.eventsCount !== events.length) throw new Error('native observer record count mismatch')
                heartbeat = row
            } else throw new Error('native observer reported ' + row.kind + ': ' + (row.message || ''))
        }
        const observed = Date.parse(heartbeat?.observedUtc)
        if (!Number.isFinite(observed) || Date.now() - observed > 5000 || observed > Date.now() + 1000) throw new Error('native observer heartbeat is missing or stale')
        return { readyUtc: ready.readyUtc, overflow: false, error: null, events }
    }
    const close = async () => {
        if (closed) return descriptor.cleanup
        closed = true
        let stopError = null
        if (!ended) {
            try { writeFileSync(stopFile, nonce, { flag: 'wx' }) } catch (error) { stopError = String(error) }
        }
        const until = Date.now() + 5000
        while (!ended && Date.now() < until) await delay(50)
        let forced = false
        if (!ended) { forced = true; child.kill() }
        const completionUntil = Date.now() + 5000
        while (!ended && Date.now() < completionUntil) await delay(50)
        if (!ended) throw new Error('owned native observer could not be reaped')
        const outcome = await completion
        descriptor.cleanup = { ...outcome, forced, stopError, spawnError: spawnError?.message ?? null, stderr, stdout }
        if (outcome.code !== 0 || forced || stopError || spawnError) throw new Error('native observer cleanup failed: ' + JSON.stringify(descriptor.cleanup))
        return descriptor.cleanup
    }
    try {
        const until = Math.min(deadline - 1000, Date.now() + 15000)
        while (Date.now() < until && !existsSync(readyFile)) {
            if (ended || spawnError) throw new Error('native observer failed before readiness: ' + (spawnError?.message || stderr))
            await delay(50)
        }
        if (!existsSync(readyFile)) throw new Error('native observer readiness timed out')
        ready = JSON.parse(readFileSync(readyFile, 'utf8'))
        if (ready.kind !== 'ready' || ready.nonce !== nonce || ready.pid !== child.pid || !Number.isFinite(Date.parse(ready.creationTimeUTC)) || !Number.isFinite(Date.parse(ready.readyUtc))) throw new Error('invalid native observer readiness identity')
        descriptor.ready = ready
        const heartbeatUntil = Math.min(deadline - 1000, Date.now() + 5000)
        let initial
        while (Date.now() < heartbeatUntil) {
            try { initial = trace(); break } catch (error) {
                if (ended || spawnError) throw error
                await delay(50)
            }
        }
        if (!initial) throw new Error('native observer did not publish a valid initial heartbeat')
        return { trace, close, descriptor }
    } catch (error) {
        try { await close() } catch (cleanupError) { error.observerCleanupError = String(cleanupError) }
        throw error
    }
}
