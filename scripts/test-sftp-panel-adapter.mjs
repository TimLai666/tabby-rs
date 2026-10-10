import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/sftpPanelTransport.ts'), 'utf8')
const module = { exports: {} }
const output = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS } }).outputText
vm.runInNewContext(output, { module, exports: module.exports, Date, Error }, { filename: 'sftpPanelTransport.js' })
const { TauriSftpPanelTransport } = module.exports
const failures = []
let count = 0
async function check (name, run) {
    count++
    try { await run(); console.log(`${name}: PASS`) } catch (error) { failures.push(name); console.error(`${name}: FAIL: ${error.message}`) }
}
const entry = { name: 'alias', fullPath: '/data/alias', isDirectory: false, isSymlink: true, mode: 0o120777, size: 12, modified: 1720000000, isOperable: false, unoperableReason: 'display-only' }
await check('listing converts Unix seconds to the original Date and preserves remote identity', async () => {
    let listed
    const adapter = new TauriSftpPanelTransport({ readdir: async p => { listed = p; return [entry] } })
    const [result] = await adapter.readdir('/data')
    assert.equal(listed, '/data')
    assert.ok(result.modified instanceof Date)
    assert.equal(result.modified.getTime(), 1720000000000)
    assert.equal(result.fullPath, entry.fullPath)
    assert.equal(result.isSymlink, true)
    assert.equal(result.mode, entry.mode)
    assert.equal(result.isOperable, false)
    assert.equal(result.unoperableReason, 'display-only')
    assert.equal(entry.modified, 1720000000, 'adapter must not mutate the native listing')
})
await check('missing modification time retains the original epoch Date', async () => {
    const adapter = new TauriSftpPanelTransport({ readdir: async () => [{ ...entry, modified: null }, { ...entry, modified: 0 }] })
    const results = await adapter.readdir('/data')
    assert.deepEqual(Array.from(results, result => result.modified.getTime()), [0, 0])
})
await check('panel stat follows links and returns the original Date shape', async () => {
    const calls = []
    const adapter = new TauriSftpPanelTransport({ stat: async (...args) => { calls.push(args); return { ...entry, isSymlink: false } } })
    const result = await adapter.stat('/data/target')
    assert.deepEqual(calls, [['/data/target', true]])
    assert.equal(result.modified.getTime(), 1720000000000)
    assert.equal(result.isSymlink, false)
})
await check('failed listing/stat propagate the actual transport error', async () => {
    const failure = new Error('remote connection closed')
    const adapter = new TauriSftpPanelTransport({ readdir: async () => { throw failure }, stat: async () => { throw failure } })
    await assert.rejects(adapter.readdir('/data'), error => error === failure)
    await assert.rejects(adapter.stat('/data'), error => error === failure)
})
await check('native structured listing errors retain their text in the shared controller', async () => {
    const failure = { code: 'permissionDenied', details: 'Cannot list /private' }
    const adapter = new TauriSftpPanelTransport({ readdir: async () => { throw failure } })
    await assert.rejects(adapter.readdir('/private'), error => error instanceof Error && error.message === failure.details)
})
await check('upload/download preserve streaming handles and returned descriptors', async () => {
    const transfer = {}; const descriptor = {}; const calls = []
    const adapter = new TauriSftpPanelTransport({ upload: async (...args) => { calls.push(['upload', ...args]); return descriptor }, download: async (...args) => { calls.push(['download', ...args]); return descriptor } })
    assert.equal(await adapter.upload('/data/a', transfer), descriptor)
    assert.equal(await adapter.download('/data/a', transfer), descriptor)
    assert.deepEqual(calls, [['upload', '/data/a', transfer, 'skip'], ['download', '/data/a', transfer]])
})
await check('readlink/mkdir preserve paths and errors without renderer file buffering', async () => {
    const calls = []; const failure = new Error('permission denied')
    const adapter = new TauriSftpPanelTransport({ readlink: async p => { calls.push(p); return '../target' }, mkdir: async () => { throw failure } })
    assert.equal(await adapter.readlink('/data/alias'), '../target')
    assert.deepEqual(calls, ['/data/alias'])
    await assert.rejects(adapter.mkdir('/data/new'), error => error === failure)
})
console.log(`SFTP panel transport adapter: ${count - failures.length} passed; ${failures.length} failed; transport boundaries are simulated`)
assert.equal(failures.length, 0, failures.join('; '))
