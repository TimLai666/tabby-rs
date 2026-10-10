import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'
import { Subject } from 'rxjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/sftpPanel.component.ts'), 'utf8')
const deleteSource = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/sftpDeleteModal.component.ts'), 'utf8')
const upstream = execFileSync('git', ['show', '14e2d60:tabby-ssh/src/sftpContextMenu.ts'], { cwd: root, encoding: 'utf8' })
const deleteModule = { exports: {} }
vm.runInNewContext(ts.transpileModule(deleteSource, {
    compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true },
}).outputText, {
    module: deleteModule, exports: deleteModule.exports, Error,
    require: name => {
        if (name === '@angular/core') return { Component: () => target => target }
        if (name === 'tabby-core') return { BaseComponent: class { destroyed$ = new Subject() } }
        if (name === '@ng-bootstrap/ng-bootstrap' || name.endsWith('.pug')) return {}
        throw new Error(`Unexpected dependency: ${name}`)
    },
})
const { TauriSftpDeleteModalComponent } = deleteModule.exports

function method (source, className, methodName) {
    const tree = ts.createSourceFile(`${className}.ts`, source, ts.ScriptTarget.Latest, true)
    const cls = tree.statements.find(node => ts.isClassDeclaration(node) && node.name?.text === className)
    const member = cls?.members.find(node => ts.isMethodDeclaration(node) && node.name?.getText(tree) === methodName)
    assert.ok(member, `${className}.${methodName} must exist`)
    return ts.createPrinter().printNode(ts.EmitHint.Unspecified, member, tree)
}

function instantiate (name, body, globals = {}) {
    const result = { exports: {} }
    const code = ts.transpileModule(`class ${name} { ${body} }; module.exports = ${name}`, {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS },
    }).outputText
    vm.runInNewContext(code, { module: result, ...globals }, { filename: `${name}.js` })
    return new result.exports()
}

const directory = { name: 'folder', fullPath: '/remote/folder', isDirectory: true, isOperable: true }
const file = { name: 'file.txt', fullPath: '/remote/file.txt', isDirectory: false, isOperable: true }

function native (action, confirmed) {
    const calls = { prompts: [], confirmations: [], removes: [], refreshes: 0, errors: [], prevented: 0 }
    const panel = instantiate('NativePanel', [
        method(source, 'TauriSftpPanelComponent', 'buildContextMenu'),
        method(source, 'TauriSftpPanelComponent', 'showContextMenu'),
    ].join('\n'), { TauriSftpDeleteModalComponent, Error })
    panel.path = '/remote'
    panel.sftp = {
        readdir: async () => [{ ...file, fullPath: '/remote/folder/file.txt' }],
        remove: async (...args) => { calls.removes.push(args) },
    }
    panel.translate = { instant: text => text }
    panel.platform = {
        setClipboard: () => {},
        showMessageBox: async options => { calls.confirmations.push(options); return { response: confirmed ? 0 : 1 } },
        popupContextMenu: items => {
            calls.prompts.push(items)
            if (action === 'delete') { calls.pending = items.find(item => item.label === 'Delete').click() }
        },
    }
    panel.ngbModal = { open: () => {
        let close; let dismiss
        const result = new Promise((resolve, reject) => { close = resolve; dismiss = reject })
        const instance = new TauriSftpDeleteModalComponent({ close, dismiss })
        queueMicrotask(() => instance.ngOnInit())
        return { componentInstance: instance, result }
    } }
    panel.navigate = async () => { calls.refreshes++ }
    panel.showError = error => calls.errors.push(error.message)
    panel.showMenu = async (...args) => { await panel.showContextMenu(...args); await calls.pending }
    const event = { preventDefault: () => { calls.prevented++ } }
    return { panel, calls, event }
}

const failures = []
async function check (name, run) {
    try { await run(); console.log(`${name}: PASS`) } catch (error) {
        failures.push(name)
        console.error(`${name}: FAIL: ${error.message}`)
    }
}

await check('fixed upstream cancels directory deletion before opening its delete modal', async () => {
    const menu = instantiate('ReferenceMenu', method(upstream, 'CommonSFTPContextMenu', 'getItems'), { Platform: { Web: 'web' } })
    let dialogs = 0
    let deletes = 0
    let refreshes = 0
    menu.platform = { showMessageBox: async () => { dialogs++; return { response: 1 } } }
    menu.translate = { instant: text => text }
    menu.hostApp = { platform: 'windows' }
    menu.deleteItem = async () => { deletes++ }
    const panel = { path: '/remote', sftp: {}, navigate: () => { refreshes++ } }
    const items = await menu.getItems(directory, panel)
    await items.find(item => item.label === 'Delete').click()
    assert.equal(dialogs, 1)
    assert.equal(deletes, 0)
    assert.equal(refreshes, 0)
})

await check('native directory cancellation sends no remove request or refresh', async () => {
    const { panel, calls, event } = native('delete', false)
    await panel.showMenu(directory, event)
    assert.equal(calls.prevented, 1)
    assert.equal(calls.confirmations.length, 1)
    assert.equal(calls.removes.length, 0, 'Cancel must not send sftp.remove')
    assert.equal(calls.refreshes, 0, 'Cancel must leave the current listing unchanged')
    assert.equal(calls.errors.length, 0)
})

await check('native approved directory deletion remains recursive and refreshes once', async () => {
    const { panel, calls, event } = native('delete', true)
    await panel.showMenu(directory, event)
    assert.equal(calls.removes.length, 2)
    assert.equal(calls.removes[0][0], '/remote/folder/file.txt')
    assert.equal(calls.removes[0][1], false)
    assert.equal(calls.removes[1][0], directory.fullPath)
    assert.equal(calls.removes[1][1], false)
    assert.equal(calls.refreshes, 1)
})

await check('native file deletion preserves the nonrecursive request', async () => {
    const { panel, calls, event } = native('delete', true)
    await panel.showMenu(file, event)
    assert.equal(calls.confirmations.length, 1)
    assert.equal(calls.removes.length, 1)
    assert.equal(calls.removes[0][0], file.fullPath)
    assert.equal(calls.removes[0][1], false)
    assert.equal(calls.refreshes, 1)
})

await check('dismissing the native context menu sends no remove request', async () => {
    const { panel, calls, event } = native(null, true)
    await panel.showMenu(directory, event)
    assert.equal(calls.confirmations.length, 0)
    assert.equal(calls.removes.length, 0)
})

await check('display-only remote names never reach a deletion prompt or request', async () => {
    const { panel, calls, event } = native('delete', true)
    await panel.showMenu({ ...directory, isOperable: false, unoperableReason: 'display-only fixture' }, event)
    assert.equal(calls.prompts.length, 0)
    assert.equal(calls.confirmations.length, 0)
    assert.equal(calls.removes.length, 0)
    assert.equal(calls.errors.length, 1)
    assert.equal(calls.errors[0], 'display-only fixture')
})

assert.equal(failures.length, 0, `SFTP deletion cancellation failed: ${failures.join(', ')}`)
console.log('SFTP deletion cancellation contract: 6 passed; method boundaries are simulated, desktop interaction is not verified')
