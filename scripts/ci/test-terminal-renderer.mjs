#!/usr/bin/env node

import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import vm from 'node:vm'
import ts from 'typescript'

const source = await readFile('tabby-terminal/src/renderer/writeQueue.ts', 'utf8')
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
        strict: true,
    },
    fileName: 'writeQueue.ts',
})

const module = { exports: {} }
const context = vm.createContext({
    module,
    exports: module.exports,
    Error,
    Promise,
})
vm.runInContext(compiled.outputText, context, { filename: 'writeQueue.cjs' })
const { RendererWriteQueue } = module.exports

const started = []
const completions = []
const queue = new RendererWriteQueue((data, done) => {
    started.push(data)
    completions.push(done)
})

const resolved = []
const first = queue.write('first').then(() => resolved.push('first'))
const second = queue.write('second').then(() => resolved.push('second'))

await Promise.resolve()
assert.deepEqual(started, ['first'])
assert.deepEqual(resolved, [])

completions.shift()()
await first
await Promise.resolve()
assert.deepEqual(started, ['first', 'second'])
assert.deepEqual(resolved, ['first'])

completions.shift()()
await second
assert.deepEqual(resolved, ['first', 'second'])

queue.dispose()
await assert.rejects(queue.write('after-dispose'), /disposed/)

console.log('Terminal renderer write completion tests passed.')
