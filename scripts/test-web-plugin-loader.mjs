import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'web/entry.preload.ts'), 'utf8')
const functionSource = source.match(/async function webRequire \(url\) \{[\s\S]*?\n\}/)?.[0]
assert.ok(functionSource, 'webRequire implementation must remain discoverable')
const javascriptSource = ts.transpileModule(functionSource, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
    },
}).outputText

const loaded = []
const removed = []
const window = { module: { exports: {} }, exports: {} }
const head = {
    appendChild (script) {
        loaded.push(script.src)
        if (script.src.endsWith('/missing.js')) {
            script.onerror(new Error('network error'))
            return
        }
        window.module.exports = { loaded: script.src }
        script.onload()
    },
}
const document = {
    createElement () {
        return {
            remove () {
                removed.push(this.src)
            },
        }
    },
    querySelector (selector) {
        assert.equal(selector, 'head')
        return head
    },
}
const context = { console, document, window }
vm.runInNewContext(`${javascriptSource}\nglobalThis.webRequire = webRequire`, context, {
    filename: 'web/entry.preload.ts',
})

const loadedModule = await context.webRequire('https://cdn.example/plugin.js')
assert.deepEqual(loadedModule, { loaded: 'https://cdn.example/plugin.js' })
await assert.rejects(
    context.webRequire('https://cdn.example/missing.js'),
    error => error?.message === 'Failed to load web plugin https://cdn.example/missing.js',
)
assert.deepEqual(loaded, [
    'https://cdn.example/plugin.js',
    'https://cdn.example/missing.js',
])
assert.deepEqual(removed, [
    'https://cdn.example/plugin.js',
    'https://cdn.example/missing.js',
])

console.log('Web plugin loader failure contract passed')
