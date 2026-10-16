/** QA-only proof from retained native process-start events. */
const FILETIME_EPOCH = 116444736000000000n
const validPid = value => Number.isSafeInteger(value) && value > 0

function isoFileTime (value) {
    if (typeof value !== 'string') return null
    const parts = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(?:\.(\d{1,7}))?Z$/.exec(value)
    if (!parts) return null
    const milliseconds = Date.parse(parts[1] + 'Z')
    if (!Number.isFinite(milliseconds) || new Date(milliseconds).toISOString().slice(0, 19) !== parts[1]) return null
    return FILETIME_EPOCH + BigInt(milliseconds) * 10000n + BigInt((parts[2] ?? '').padEnd(7, '0'))
}

export function proveRestartOrigin (input) {
    const chain = []
    const reject = reason => ({ owned: false, reason, chain })
    if (!input || typeof input !== 'object') return reject('missing native origin evidence')
    const { candidate, seeds, trace, triggeredUtc } = input
    if (!candidate || !validPid(candidate.pid) || !validPid(candidate.parentPid)) return reject('invalid candidate PID')
    if (!trace || trace.overflow !== false || trace.error !== null || !Array.isArray(trace.events) || trace.events.length > 10000) return reject('incomplete native event history')
    if (!Array.isArray(seeds) || !seeds.length || seeds.length > 10000) return reject('missing owned seed processes')
    const ready = isoFileTime(trace.readyUtc), triggered = isoFileTime(triggeredUtc), created = isoFileTime(candidate.creationTimeUTC)
    if (ready === null || triggered === null || created === null || ready > triggered || created < triggered) return reject('invalid observer, trigger or candidate time')
    const seedByPid = new Map()
    for (const seed of seeds) {
        const time = isoFileTime(seed?.creationTimeUTC)
        if (!validPid(seed?.pid) || time === null || time >= ready || seedByPid.has(seed.pid)) return reject('invalid or ambiguous pre-observer seed')
        seedByPid.set(seed.pid, time)
    }
    const starts = new Map()
    for (const event of trace.events) {
        if (!validPid(event?.pid) || !Number.isSafeInteger(event?.parentPid) || event.parentPid < 0 || typeof event.fileTime !== 'string' || !/^[0-9]{1,20}$/.test(event.fileTime)) return reject('malformed native process start')
        const time = BigInt(event.fileTime)
        if (time < ready || time > 18446744073709551615n) return reject('native process start is outside observation')
        const rows = starts.get(event.pid) ?? []
        rows.push({ ...event, time })
        starts.set(event.pid, rows)
    }
    const candidateStarts = starts.get(candidate.pid)
    if (candidateStarts?.length !== 1) return reject('candidate native start is missing or ambiguous')
    const initial = candidateStarts[0]
    if (initial.parentPid !== candidate.parentPid || initial.time < created || initial.time > created + 10000000n) return reject('candidate CIM identity disagrees with native start')
    const visited = new Set()
    let current = candidate.pid, childTime = initial.time
    for (let depth = 0; depth <= 64; depth++) {
        if (visited.has(current)) return reject('cycle in native process ancestry')
        visited.add(current)
        chain.push(current)
        if (seedByPid.has(current)) {
            if (starts.has(current)) return reject('owned seed PID restarted during observation')
            return { owned: true, reason: 'native start ancestry reaches an unchanged owned seed', chain }
        }
        if (depth === 64) return reject('native ancestry exceeds 64 hops')
        const rows = starts.get(current)
        if (rows?.length !== 1) return reject('ancestor start is missing or PID was reused')
        const event = rows[0]
        if (event.time > childTime || !validPid(event.parentPid)) return reject('native ancestor time or parent is invalid')
        childTime = event.time
        current = event.parentPid
    }
    return reject('native ancestry did not reach an owned seed')
}
