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
} else if (mode === '--output') {
    const bytes = Number(process.argv[3])
    process.stdout.write(Buffer.alloc(bytes, 0x61))
} else {
    throw new Error(`unknown benchmark fixture mode: ${mode}`)
}
