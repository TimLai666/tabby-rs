#!/usr/bin/env node

import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import vm from 'node:vm'
import ts from 'typescript'

const source = await readFile('tabby-terminal/src/pastePolicy.ts', 'utf8')
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
        strict: true,
    },
    fileName: 'pastePolicy.ts',
})

const module = { exports: {} }
const context = vm.createContext({
    module,
    exports: module.exports,
    TextEncoder,
})
vm.runInContext(compiled.outputText, context, { filename: 'pastePolicy.cjs' })
const { DefaultPastePolicy } = module.exports

const policy = new DefaultPastePolicy({
    windows: false,
    replaceNewlinesWithSpaces: false,
    trimWhitespace: true,
})
const inspection = policy.inspect('echo one\necho two\x1b[201~', {
    alternateScreenActive: false,
    warnOnMultilinePaste: true,
    bracketedPaste: true,
})

assert.equal(inspection.lineCount, 2)
assert.equal(inspection.containsBracketedPasteEnd, true)
assert.equal(inspection.shouldConfirm, true)
assert.equal(inspection.reasons.includes('multiline'), true)
assert.equal(policy.inspect('echo\u0000', {
    alternateScreenActive: false,
    warnOnMultilinePaste: true,
    bracketedPaste: false,
}).containsControlCharacters, true)
assert.deepEqual([...policy.encode(inspection.text, {
    alternateScreenActive: false,
    warnOnMultilinePaste: true,
    bracketedPaste: true,
})], [...new TextEncoder().encode('\x1b[200~echo one\recho two\x1b[201~')])

const windowsPolicy = new DefaultPastePolicy({
    windows: true,
    replaceNewlinesWithSpaces: false,
    trimWhitespace: true,
})
assert.equal(windowsPolicy.inspect(' one\r\ntwo\r\n', {
    alternateScreenActive: false,
    warnOnMultilinePaste: false,
    bracketedPaste: false,
}).text, ' one\rtwo\r')

console.log('Paste policy tests passed.')
