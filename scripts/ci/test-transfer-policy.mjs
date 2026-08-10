#!/usr/bin/env node

import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import vm from 'node:vm'
import ts from 'typescript'

const source = await readFile('tabby-core/src/api/fileTransfer.ts', 'utf8')
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
        strict: true,
    },
    fileName: 'fileTransfer.ts',
})
const module = { exports: {} }
vm.runInNewContext(compiled.outputText, { module, exports: module.exports }, { filename: 'fileTransfer.cjs' })
const { sanitizeTransferName, sanitizeTransferRelativePath } = module.exports

assert.equal(sanitizeTransferName('../report.txt'), '.._report.txt')
assert.equal(sanitizeTransferName('CON.txt'), '_CON.txt')
assert.equal(sanitizeTransferName('\0\n'), 'download')
assert.equal(sanitizeTransferName(''), 'download')
assert.equal(sanitizeTransferRelativePath('folder\\report.txt'), 'folder/report.txt')
assert.throws(() => sanitizeTransferRelativePath('../escape'))
assert.throws(() => sanitizeTransferRelativePath('/tmp/escape'))
assert.throws(() => sanitizeTransferRelativePath('C:\\escape'))
assert.throws(() => sanitizeTransferRelativePath('\\\\server\\share'))

console.log('Transfer policy tests passed.')
