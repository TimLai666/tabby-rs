import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
import { Subject } from 'rxjs'
import ts from 'typescript'
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/sftpDeleteModal.component.ts'), 'utf8')
const module = { exports: {} }
const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true } }).outputText
vm.runInNewContext(code, { module, exports: module.exports, require: id => id === '@angular/core' ? { Component: () => () => {} } : id === 'tabby-core' ? { BaseComponent: class { destroyed$ = new Subject() } } : '', Error }, { filename: 'sftpDeleteModal.js' })
const { TauriSftpDeleteModalComponent } = module.exports
const file = { name: 'a.txt', fullPath: '/folder/a.txt', isDirectory: false, isSymlink: false, isOperable: true }
const directory = { name: 'folder', fullPath: '/folder', isDirectory: true, isSymlink: false, isOperable: true }
const failures = []; let count = 0
async function check (name, run) { count++; try { await run(); console.log(`${name}: PASS`) } catch (error) { failures.push(name); console.error(`${name}: FAIL: ${error.message}`) } }
function fixture () {
    const calls = []; const closes = []; const dismisses = []
    const modal = new TauriSftpDeleteModalComponent({ close: x => closes.push(x), dismiss: x => dismisses.push(x) })
    modal.item = directory
    modal.sftp = { readdir: async p => { calls.push(['list', p]); return [file] }, remove: async (...args) => calls.push(['remove', ...args]) }
    return { modal, calls, closes, dismisses }
}
await check('recursive deletion awaits each child before removing its parent', async () => {
    const { modal, calls, closes, dismisses } = fixture()
    await modal.ngOnInit()
    assert.deepEqual(calls, [['list', '/folder'], ['remove', '/folder/a.txt', false], ['remove', '/folder', false]])
    assert.deepEqual(closes, [true]); assert.deepEqual(dismisses, [])
})
await check('cancel before starting sends no remote operation', async () => {
    const { modal, calls, closes } = fixture(); modal.cancel(); await modal.ngOnInit()
    assert.deepEqual(calls, []); assert.deepEqual(closes, [false])
})
await check('cancel during a pending child stops siblings and parent removal', async () => {
    const { modal, calls, closes } = fixture()
    let resume; const pending = new Promise(resolve => { resume = resolve })
    modal.sftp.readdir = async () => [file, { ...file, name: 'b.txt', fullPath: '/folder/b.txt' }]
    modal.sftp.remove = async (...args) => { calls.push(args); await pending }
    const running = modal.ngOnInit(); await new Promise(resolve => setTimeout(resolve, 0)); modal.cancel(); resume(); await running
    assert.deepEqual(calls, [['/folder/a.txt', false]]); assert.deepEqual(closes, [false])
})
await check('failed child removal dismisses with the original error and retains parent', async () => {
    const { modal, calls, closes, dismisses } = fixture(); const error = new Error('permission denied')
    modal.sftp.remove = async (...args) => { calls.push(['remove', ...args]); throw error }
    await modal.ngOnInit()
    assert.deepEqual(calls, [['list', '/folder'], ['remove', '/folder/a.txt', false]])
    assert.deepEqual(closes, []); assert.equal(dismisses[0], error)
})
await check('symbolic link is removed as an alias without traversing its target', async () => {
    const { modal, calls } = fixture();modal.item = { ...file, isSymlink: true };await modal.ngOnInit()
    assert.deepEqual(calls, [['remove', '/folder/a.txt', false]])
})
await check('display-only child stops deletion without removing any remote path', async () => {
    const { modal, calls, dismisses } = fixture()
    modal.sftp.readdir = async p => { calls.push(['list', p]); return [{ ...file, isOperable: false, unoperableReason: 'display-only' }] }
    await modal.ngOnInit();assert.deepEqual(calls,[['list','/folder']]);assert.match(dismisses[0].message,/display-only/)
})
await check('component destruction does not re-close a completed modal', async () => {
    const { modal, closes } = fixture()
    modal.modalInstance.close = value => {
        closes.push(value)
        if (closes.length === 1) { modal.destroyed$.next() }
    }
    await modal.ngOnInit()
    assert.deepEqual(closes, [true])
    assert.equal(modal.cancelled, true)
    await modal.settled
})
await check('duplicate cancellation closes the modal once', async () => {
    const { modal, closes } = fixture()
    modal.cancel(); modal.cancel()
    assert.deepEqual(closes, [false])
})
console.log(`SFTP deletion progress: ${count - failures.length} passed; ${failures.length} failed; native operations are simulated`)
assert.equal(failures.length,0,failures.join('; '))
