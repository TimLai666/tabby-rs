#!/usr/bin/env node

import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import vm from 'node:vm'
import ts from 'typescript'

const source = await readFile('tabby-auto-sudo-password/src/promptDetector.ts', 'utf8')
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
        strict: true,
    },
    fileName: 'promptDetector.ts',
})

const module = { exports: {} }
const context = vm.createContext({
    module,
    exports: module.exports,
    console,
})
vm.runInContext(compiled.outputText, context, { filename: 'promptDetector.cjs' })
const { SudoPromptDetector } = module.exports

function detector () {
    return new SudoPromptDetector()
}

{
    const value = detector()
    assert.equal(value.feed('[sudo] pass'), null)
    assert.deepEqual(value.feed('word for tim: '), { username: 'tim' })
}

{
    const value = detector()
    assert.equal(value.feed('[sudo] 測試'), null)
    assert.deepEqual(value.feed(' 的密碼：'), { username: '測試' })
}

{
    const value = detector()
    assert.deepEqual(value.feed('[sudo: authenticate] Password:'), { username: null })
}

{
    const value = detector()
    assert.equal(value.feed('\u001b[31m[sudo]\u001b[0m pass'), null)
    assert.deepEqual(value.feed('word for tim: '), { username: 'tim' })
    assert.equal(value.feed('unrelated output'), null)
    assert.deepEqual(value.feed('\n[sudo] password for tim: '), { username: 'tim' })
}

{
    const value = detector()
    assert.equal(value.feed('x'.repeat(10_000)), null)
    assert.deepEqual(value.feed('\n[sudo] password for bounded: '), { username: 'bounded' })
}

console.log('Streaming sudo prompt detector tests passed.')
