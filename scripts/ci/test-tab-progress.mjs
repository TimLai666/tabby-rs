#!/usr/bin/env node

import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import vm from 'node:vm'
import ts from 'typescript'

async function load (fileName) {
    const source = await readFile(fileName, 'utf8')
    const compiled = ts.transpileModule(source, {
        compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020, strict: true },
        fileName,
    })
    const module = { exports: {} }
    vm.runInNewContext(compiled.outputText, { module, exports: module.exports }, { filename: `${fileName}.cjs` })
    return module.exports
}

const { aggregateTabProgress } = await load('tabby-core/src/api/tabProgress.ts')
const normal = value => ({ value, state: 'normal', source: 'heuristic' })
const error = { value: null, state: 'error', source: 'osc' }

assert.equal(aggregateTabProgress([
    { tabId: 'active', active: true, progress: normal(10) },
    { tabId: 'other', active: false, progress: normal(90) },
]).value, 10)
assert.equal(aggregateTabProgress([
    { tabId: 'active', active: false, progress: normal(10) },
    { tabId: 'other', active: false, progress: error },
]).state, 'error')
assert.equal(aggregateTabProgress([
    { tabId: 'one', active: false, progress: normal(10) },
    { tabId: 'two', active: false, progress: normal(90) },
]).value, 90)
assert.equal(aggregateTabProgress([]).state, 'none')

const { redactProcessCommand } = await load('tabby-core/src/api/processCompletion.ts')
assert.equal(redactProcessCommand('/usr/bin/node --token=secret'), 'node')
assert.equal(redactProcessCommand('"C:\\Program Files\\tool.exe" --password=secret'), 'tool.exe')
assert.equal(redactProcessCommand(undefined), undefined)

console.log('Tab progress and completion metadata tests passed.')
