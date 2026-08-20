import { spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

const mode = process.argv[2]
if (mode === '--ready') {
    const marker = process.env.TABBY_RS_BENCHMARK_READY_FILE
    if (!marker) {
        throw new Error('TABBY_RS_BENCHMARK_READY_FILE is required')
    }
    fs.mkdirSync(path.dirname(marker), { recursive: true })
    fs.writeFileSync(marker, 'ready\n')
    setInterval(() => undefined, 1000)
} else if (mode === '--ready-with-child') {
    const marker = process.env.TABBY_RS_BENCHMARK_READY_FILE
    const childPidFile = process.argv[3]
    const childCleanupFile = process.argv[4]
    if (!marker || !childPidFile || !childCleanupFile) {
        throw new Error('ready-with-child requires marker, child PID file, and child cleanup file')
    }
    const child = spawn(process.execPath, ['-e', [
        "process.on('SIGTERM', () => {",
        "  require('node:fs').appendFileSync(process.env.TABBY_RS_BENCHMARK_CHILD_CLEANUP_FILE, 'cleaned\\n')",
        '  process.exit(0)',
        '})',
        'if (process.send) process.send("ready")',
        'setInterval(() => undefined, 1000)',
    ].join('\n')], {
        env: { ...process.env, TABBY_RS_BENCHMARK_CHILD_CLEANUP_FILE: childCleanupFile },
        stdio: ['ignore', 'ignore', 'ignore', 'ipc'],
    })
    child.once('message', () => {
        fs.appendFileSync(childPidFile, `${child.pid}\n`)
        fs.mkdirSync(path.dirname(marker), { recursive: true })
        fs.writeFileSync(marker, 'ready\n')
    })
    setInterval(() => undefined, 1000)
} else if (mode === '--output') {
    const bytes = Number(process.argv[3])
    process.stdout.write(Buffer.alloc(bytes, 0x61))
} else {
    throw new Error(`unknown benchmark fixture mode: ${mode}`)
}
