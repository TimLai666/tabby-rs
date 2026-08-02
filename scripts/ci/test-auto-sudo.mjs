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

function username (value) {
    return value?.username ?? null
}

{
    const value = detector()
    assert.equal(value.feed('[sudo] pass'), null)
    assert.equal(username(value.feed('word for tim: ')), 'tim')
}

{
    const value = detector()
    assert.equal(value.feed('[sudo] 測試'), null)
    assert.equal(username(value.feed(' 的密碼：')), '測試')
}

{
    const value = detector()
    assert.equal(username(value.feed('[sudo: authenticate] Password:')), null)
}

{
    const value = detector()
    assert.equal(value.feed('\u001b[31m[sudo]\u001b[0m pass'), null)
    assert.equal(username(value.feed('word for tim: ')), 'tim')
    assert.equal(value.feed('unrelated output'), null)
    assert.equal(username(value.feed('\n[sudo] password for tim: ')), 'tim')
}

{
    const value = detector()
    assert.equal(value.feed('x'.repeat(10_000)), null)
    assert.equal(username(value.feed('\n[sudo] password for bounded: ')), 'bounded')
}

console.log('Streaming sudo prompt detector tests passed.')
