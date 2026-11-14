import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
import { Subject } from 'rxjs'
import ts from 'typescript'

// Upstream 9e31ec6d surfaces SFTP upload errors through the transfer status
// before cancelling; the native session must do the same.
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = file => fs.readFileSync(path.join(root, file), 'utf8')
function cls (text, name) {
    const tree = ts.createSourceFile('source.ts', text, ts.ScriptTarget.Latest, true)
    const node = tree.statements.find(x => ts.isClassDeclaration(x) && x.name?.text === name)
    assert.ok(node, `${name} must exist`)
    return ts.createPrinter().printNode(ts.EmitHint.Unspecified, node, tree)
}
function run (text) {
    const module = { exports: {} }
    vm.runInNewContext(ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS } }).outputText,
        { module, exports: module.exports, require: id => { assert.equal(id, 'rxjs'); return { Subject } }, Error, Date, Uint8Array, Array, String })
    return module.exports
}
const Session = run("const { Subject } = require('rxjs');" + cls(source('tabby-tauri/src/ssh/sftp.ts'), 'TauriSftpSession') + ';module.exports=TauriSftpSession')

function transferFixture (chunks) {
    const events = []
    let index = 0
    return {
        events,
        transfer: {
            getSize: () => 6,
            read: async () => index < chunks.length ? chunks[index++] : new Uint8Array(),
            setStatus: status => events.push(['status', status]),
            cancel: () => events.push(['cancel']),
            closeAsync: async () => events.push(['close']),
        },
    }
}

const failures = []
async function check (name, test) { try { await test(); console.log(name + ': PASS') } catch (e) { failures.push(name); console.error(name + ': FAIL: ' + e.message) } }

await check('a rejected native write sets the error message as the transfer status before cancelling', async () => {
    const calls = []
    const session = await Session.open({ invoke: async (command, request) => {
        calls.push(command)
        if (command === 'sftp.write') { throw new Error('Permission denied (remote quota)') }
        return { id: 'transfer-1', name: 'a.bin' }
    } }, 'ssh-id')
    const { transfer, events } = transferFixture([new Uint8Array([1, 2, 3])])
    await assert.rejects(session.upload('/data/a.bin', transfer), /Permission denied \(remote quota\)/)
    assert.deepEqual(events, [['status', 'Permission denied (remote quota)'], ['cancel']])
    assert.ok(calls.includes('sftp.cancelTransfer'), 'the native transfer is cancelled after the error')
})

await check('a non-Error rejection is stringified into the status', async () => {
    const session = await Session.open({ invoke: async command => {
        if (command === 'sftp.closeTransfer') { throw 'disk full' } // eslint-disable-line no-throw-literal
        return { id: 'transfer-2', name: 'a.bin' }
    } }, 'ssh-id')
    const { transfer, events } = transferFixture([new Uint8Array([1])])
    await assert.rejects(session.upload('/data/a.bin', transfer))
    assert.deepEqual(events, [['status', 'disk full'], ['cancel']])
})

await check('a successful upload never sets a status', async () => {
    const session = await Session.open({ invoke: async () => ({ id: 'transfer-3', name: 'a.bin' }) }, 'ssh-id')
    const { transfer, events } = transferFixture([new Uint8Array([1, 2, 3]), new Uint8Array([4, 5, 6])])
    await session.upload('/data/a.bin', transfer)
    assert.deepEqual(events, [['close']])
})

if (failures.length) { console.error(`SFTP upload errors: ${failures.length} failed`); process.exit(1) }
console.log('SFTP upload errors: 3 passed; native IPC is simulated')
