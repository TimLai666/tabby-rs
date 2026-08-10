#!/usr/bin/env node

import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import vm from 'node:vm'
import ts from 'typescript'

const source = await readFile('tabby-linkifier/src/uriPolicy.ts', 'utf8')
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
        strict: true,
    },
    fileName: 'uriPolicy.ts',
})
const module = { exports: {} }
vm.runInNewContext(compiled.outputText, { module, exports: module.exports, URL, encodeURIComponent }, { filename: 'uriPolicy.cjs' })
const { decideUri } = module.exports
const context = { source: 'terminal-output', cwd: '/tmp/tabby', allowedSchemes: ['http', 'https', 'mailto'] }

assert.equal(decideUri('https://example.com/a', context).action, 'open')
assert.equal(decideUri('www.example.com', context).normalized, 'https://www.example.com/')
assert.equal(decideUri('mailto:test@example.com', context).action, 'open')
assert.match(decideUri('https://例子.测试', context).normalized, /^https:\/\/xn--fsqu00a\.xn--0zwm56d\//)
assert.equal(JSON.stringify(decideUri('file:///tmp/a.txt', context)), JSON.stringify({ action: 'confirm', normalized: 'file:///tmp/a.txt', reason: 'local-file' }))
assert.equal(decideUri('app://open/item', context).action, 'confirm')
assert.equal(decideUri('./notes.txt', context).normalized, 'file:///tmp/tabby/notes.txt')
assert.equal(decideUri('C:\\Users\\tabby\\notes.txt', context).action, 'confirm')
assert.equal(decideUri('./notes.txt', { ...context, cwd: null }).action, 'reject')
assert.equal(decideUri('javascript:alert(1)', context).action, 'reject')
assert.equal(decideUri('data:text/html,hello', context).action, 'reject')
assert.equal(decideUri('https://example.com\\@evil.test', context).action, 'reject')
assert.equal(decideUri('https:javascript:alert(1)', context).action, 'reject')
assert.equal(decideUri(`https://example.com/${'a'.repeat(8192)}`, context).action, 'reject')
assert.equal(decideUri('https://example.com/\nsecret', context).action, 'reject')
assert.equal(decideUri('https://example.com/%00', context).action, 'reject')

console.log('URI policy tests passed.')
