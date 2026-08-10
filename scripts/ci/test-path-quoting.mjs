#!/usr/bin/env node

import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import vm from 'node:vm'
import ts from 'typescript'

const source = await readFile('tabby-terminal/src/pathQuoting.ts', 'utf8')
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
        strict: true,
    },
    fileName: 'pathQuoting.ts',
})

const module = { exports: {} }
const context = vm.createContext({ module, exports: module.exports })
vm.runInContext(compiled.outputText, context, { filename: 'pathQuoting.cjs' })
const { encodeTerminalPath, quoteTerminalPath } = module.exports

assert.equal(quoteTerminalPath("/tmp/a'b", 'unix'), "'/tmp/a'\\''b'")
assert.equal(quoteTerminalPath("C:\\a'b", 'powershell'), "'C:\\a''b'")
assert.equal(quoteTerminalPath('C:\\a^b!c%d', 'cmd'), '"C:\\a^^b^!c%%d"')
assert.equal(quoteTerminalPath('bad\u0000path', 'unix'), "'badpath'")
assert.equal(encodeTerminalPath('/tmp/file', 'unix', true), "\x1b[200~'/tmp/file' \x1b[201~")

console.log('Terminal path quoting tests passed.')
