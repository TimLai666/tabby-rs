#!/usr/bin/env node

import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import vm from 'node:vm'
import ts from 'typescript'

const source = await readFile('tabby-terminal/src/progressDetector.ts', 'utf8')
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
        strict: true,
    },
    fileName: 'progressDetector.ts',
})
const module = { exports: {} }
vm.runInNewContext(compiled.outputText, { module, exports: module.exports, Buffer }, { filename: 'progressDetector.cjs' })
const { OSCProgressDetector, detectHeuristicProgress } = module.exports

const detector = new OSCProgressDetector()
assert.equal(detector.consume(Buffer.from('\x1b]9;4;1;42')), null)
assert.equal(JSON.stringify(detector.consume(Buffer.from('\x07'))), JSON.stringify({ value: 42, state: 'normal', source: 'osc' }))
assert.equal(JSON.stringify(detector.consume(Buffer.from('\x1b]9;4;4;0\x1b\\'))), JSON.stringify({ value: null, state: 'paused', source: 'osc' }))
assert.equal(detector.consume(Buffer.from('\x1b]9;4;2;101\x07')), null)
assert.equal(JSON.stringify(detector.consume(Buffer.from('\x1b]9;4;3\x07'))), JSON.stringify({ value: null, state: 'indeterminate', source: 'osc' }))
assert.equal(JSON.stringify(detector.consume(Buffer.from('\x1b]9;4;0\x07'))), JSON.stringify({ value: null, state: 'none', source: 'osc' }))

const input = Buffer.from('prefix \x1b]9;4;1;75\x07 suffix')
const copy = Buffer.from(input)
assert.equal(JSON.stringify(detector.consume(input)), JSON.stringify({ value: 75, state: 'normal', source: 'osc' }))
assert.deepEqual(input, copy)
assert.equal(JSON.stringify(detectHeuristicProgress('download 12.5%')), JSON.stringify({ value: 12.5, state: 'normal', source: 'heuristic' }))
assert.equal(detectHeuristicProgress('1000%'), null)

console.log('Progress detector tests passed.')
