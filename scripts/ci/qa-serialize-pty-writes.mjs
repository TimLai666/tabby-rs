/** Diagnostic only: serialize actual native PTY HTTP requests in one owned page. */
export async function installPtyWriteSerializingProbe (page) {
    return page.evaluate(() => {
        const key = '__TABBY_PTY_SERIAL_PROBE'
        if (Object.hasOwn(window, key)) throw new Error('PTY diagnostic namespace already exists')
        const target = window.__TAURI_INTERNALS__.convertFileSrc('pty_write', 'ipc')
        const original = window.fetch
        const tails = new Map()
        const stats = { target, enqueued: 0, dispatched: 0, finished: 0, failed: 0, bytes: 0, pending: 0, overflow: false, requests: [] }
        const wrapped = function (input, init) {
            if (String(input) !== target) return original.call(this, input, init)
            const request = JSON.parse(init.body).request
            if (!request || typeof request.id !== 'string' || !Array.isArray(request.data)) throw new Error('unexpected PTY diagnostic request shape')
            const row = { sequence: ++stats.enqueued, sessionId: request.id, bytes: [...request.data], dispatched: null, finished: null }
            if (stats.requests.length < 50000 && stats.bytes + row.bytes.length <= 4 * 1024 * 1024) stats.requests.push(row)
            else stats.overflow = true
            stats.bytes += row.bytes.length
            stats.pending++
            const result = (tails.get(request.id) ?? Promise.resolve()).then(() => {
                row.dispatched = ++stats.dispatched
                return original.call(window, input, init)
            }).then(response => { row.finished = ++stats.finished; stats.pending--; return response }, error => {
                stats.failed++; stats.pending--; throw error
            })
            tails.set(request.id, result.catch(() => {}))
            return result
        }
        window.fetch = wrapped
        window[key] = { stats, restore: () => {
            if (window.fetch !== wrapped || stats.pending !== 0) throw new Error('cannot safely restore PTY diagnostic fetch')
            window.fetch = original
            delete window[key]
        } }
        return { installed: window.fetch === wrapped, target, productionSourceChanged: false }
    })
}
export async function collectPtyWriteSerializingProbe (page) {
    return page.evaluate(() => {
        const probe = window.__TABBY_PTY_SERIAL_PROBE
        if (!probe) throw new Error('PTY diagnostic was not installed')
        const stats = structuredClone(probe.stats)
        if (stats.pending === 0) probe.restore()
        return { ...stats, restored: stats.pending === 0, acceptanceScope: 'runtime diagnostic; not a source-built fix' }
    })
}
