import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { setImmediate } from 'node:timers/promises'
import ts from 'typescript'
import configFactory from '../app/webpack.config.tauri.mjs'

const require = createRequire(import.meta.url)
const { SerialPortStream } = createRequire(new URL('../app/package.json', import.meta.url))('@serialport/stream')
const file = new URL('../tabby-tauri/src/serial/writeQueue.ts', import.meta.url)
const module = { exports: {} }
vm.runInNewContext(ts.transpileModule(fs.readFileSync(file, 'utf8'), {
    compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS },
}).outputText, { module, exports: module.exports, Buffer,
    require: name => name === 'stream' ? require(configFactory().resolve.fallback.stream) : require(name),
})
const { SerialWriteQueue } = module.exports

function sink () {
    const calls = [], pending = [], errors = []
    return { calls, pending, errors, send: data => {
        calls.push([...data])
        return new Promise((resolve, reject) => pending.push({ resolve, reject }))
    } }
}

async function pair () {
    const actual = sink(), expected = sink()
    const queue = new SerialWriteQueue(actual.send, error => actual.errors.push(error.message))
    const stream = new SerialPortStream({ path: '/fixture', baudRate: 115200,
        autoOpen: false, binding: { open: async () => ({ isOpen: true,
            write: expected.send, close: async () => {}, drain: async () => {},
        }) },
    })
    stream.on('error', error => expected.errors.push(error.message))
    await new Promise((resolve, reject) => stream.open(error => error ? reject(error) : resolve()))
    const check = () => {
        assert.deepEqual(actual.calls, expected.calls)
        assert.deepEqual(actual.errors, expected.errors)
    }
    return { actual, expected, queue, stream, check,
        write (data, slow) {
            for (const chunk of slow ? [...data].map(byte => Buffer.from([byte])) : [data]) {
                queue.write(chunk)
                stream.write(chunk)
            }
            check()
        },
        async complete (error) {
            for (const target of [actual, expected]) {
                const pending = target.pending.shift()
                assert.ok(pending)
                error ? pending.reject(error) : pending.resolve()
            }
            await setImmediate()
            check()
        },
        async close () {
            queue.close()
            if (stream.isOpen) await new Promise(resolve => stream.close(resolve))
        },
    }
}

for (const slow of [false, true]) {
    const f = await pair()
    try {
        const first = Buffer.from([0, 255, 0xe5, 0x8f, 0xb0])
        f.write(first, slow)
        f.write(Buffer.from('BC'), slow)
        assert.equal(f.actual.calls.length, 1, 'only one native write in flight')
        await f.complete()
        assert.deepEqual(f.actual.calls[1], slow ? [...first.subarray(1), 66, 67] : [66, 67])
        f.write(Buffer.from('DE'), slow)
        f.write(Buffer.from('F'), slow)
        await f.complete()
        assert.deepEqual(f.actual.calls[2], [68, 69, 70], 'batch new input while previous batch is pending')
        await f.complete()
        f.write(Buffer.from('GH'), slow)
        await f.complete()
        if (slow) await f.complete()
        assert.deepEqual(f.actual.calls.flat(), [...first, ...Buffer.from('BCDEFGH')])
    } finally { await f.close() }
}

{
    const f = await pair()
    try {
        f.write(Buffer.from('ABC'), true)
        await f.complete(new Error('fixture failed'))
        assert.deepEqual(f.actual.calls, [[65]], 'failure discards queued bytes')
        assert.deepEqual(f.actual.errors, ['fixture failed'])
    } finally { await f.close() }
}

for (const rejected of [false, true]) {
    const s = sink()
    const queue = new SerialWriteQueue(s.send, error => s.errors.push(error))
    queue.write(Buffer.alloc(0))
    assert.deepEqual(s.calls, [])
    queue.write(Buffer.from('A'))
    queue.write(Buffer.from('B'))
    queue.close()
    queue.close()
    queue.write(Buffer.from('C'))
    rejected ? s.pending[0].reject(new Error('late failure')) : s.pending[0].resolve()
    await setImmediate()
    assert.deepEqual(s.calls, [[65]], 'close drops queued and future writes')
    assert.deepEqual(s.errors, [], 'late completion is silent after close')
}
for (const closeEarly of [false, true]) {
    const calls = [], pending = []
    const queue = new SerialWriteQueue(data => {
        calls.push(Buffer.from(data))
        return new Promise(resolve => pending.push(resolve))
    }, error => { throw error })
    const data = Buffer.alloc(2 * 1024 * 1024 + 3, 0xa5)
    queue.write(data)
    assert.equal(calls[0].length, 1024 * 1024, 'respect native write limit')
    if (closeEarly) queue.close()
    while (pending.length) {
        pending.shift()()
        await setImmediate()
    }
    assert.deepEqual(Buffer.concat(calls), closeEarly ? data.subarray(0, 1024 * 1024) : data)
    assert.ok(calls.every(chunk => chunk.length <= 1024 * 1024))
    queue.close()
}
console.log('serial write queue: upstream batching, binary order, failure, close, empty input, and native write limit passed')
