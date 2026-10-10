import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(root + '/package.json'), ts = require('typescript'), rx = require('rxjs')
const source = name => fs.readFileSync(root + '/' + name, 'utf8')
const angular = { Injectable: () => x => x, Directive: () => x => x, Inject: () => () => {}, Self: () => () => {}, Component: () => x => x, Input: () => () => {}, Output: () => () => {}, Optional: () => () => {}, EventEmitter: class extends rx.Subject { emit(v) { this.next(v) } } }
function load(text, dependencies = {}, globals = {}) {
    const module = { exports: {} }
    vm.runInNewContext(ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true } }).outputText,
        { exports: module.exports, module, require: n => { if (n === 'rxjs') return rx; if (n === '@angular/core') return angular; if (n in dependencies) return dependencies[n]; throw new Error('Unprovided ' + n) }, console, Error, Uint8Array, ...globals })
    return module.exports
}
const core = load(source('tabby-core/src/api/platform.ts'))
const native = load(source('tabby-tauri/src/services/platform.service.ts'), { 'tabby-core': core, '../api/hostBridge': {} }, { window: { devicePixelRatio: 2, matchMedia: () => ({ matches: false }) } })
const tree = (name, directory, children = []) => ({ name, path: '/local/' + name, directory, size: directory ? 0 : 4, children })
const nested = tree('nested', true, [tree('binary.dat', false)])
const directory = tree('folder', true, [nested, tree('empty', true)])
let seq = 0
function platformFor(failPath) {
    const calls = [], transfers = [], entries = { '/local/folder': directory, '/local/plain.txt': tree('plain.txt', false) }
    const platform = Object.create(native.TauriPlatformService.prototype)
    platform.runtimeInfo = { platform: 'windows' }; platform.fileDropped$ = new rx.Subject()
    platform.fileTransferStarted = new rx.Subject(); platform.fileTransferStarted.subscribe(t => transfers.push(t))
    platform.bridge = { invoke: async (command, request) => {
        calls.push([command, request])
        if (command === 'transfer.listDirectory') { if (request.path === failPath) throw new Error('cannot list'); return entries[request.path] }
        if (command === 'transfer.openUpload') return request.paths.map(p => ({ id: 'local-' + ++seq, name: p.split('/').pop(), size: 4 }))
        if (command === 'transfer.cancel') return null
        if (command === 'transfer.read') return [0, 255, 1, 254]
        throw new Error('unexpected chooser or command: ' + command)
    } }
    return { platform, calls, transfers }
}
const failures = [], checks = []
async function check(name, test) { try { await test(); checks.push(name); console.log('PASS ' + name) } catch (e) { failures.push(name); console.error('FAIL ' + name + ': ' + e.message) } }
await check('native coordinates use AppKit points on macOS and scaled pixels on Windows/Linux', async () => {
    const { platform } = platformFor()
    for (const os of ['macos', 'windows', 'linux']) {
        platform.runtimeInfo.platform = os
        const result = platform.getFileDropPosition({ x: 100, y: 80 })
        assert.equal(result.x, os === 'macos' ? 100 : 50)
        assert.equal(result.y, os === 'macos' ? 80 : 40)
    }
})
await check('mixed file and nested folders preserve top-level names and binary transfer', async () => {
    const { platform, calls, transfers } = platformFor()
    const result = await platform.startUploadFromPaths(['/local/folder', '/local/plain.txt'], true)
    assert.equal(result.getName(), '')
    const children = result.getChildrens(); assert.equal(children.length, 2); assert.equal(children[0].getName(), 'folder'); assert.equal(children[1].getName(), 'plain.txt')
    assert.equal(children[0].getChildrens()[1].getName(), 'empty'); assert.equal(children[0].getChildrens()[1].getChildrens().length, 0)
    const leaf = children[0].getChildrens()[0].getChildrens()[0]; assert.equal(leaf, transfers[0]); assert.equal(leaf.getName(), 'binary.dat')
    assert.deepEqual(Array.from(await leaf.read()), [0, 255, 1, 254]); assert.equal(transfers.length, 2)
    assert.deepEqual(calls.filter(c => c[0] === 'transfer.listDirectory').map(c => c[1].path), ['/local/folder', '/local/plain.txt'])
})
await check('single path and empty input do not open a chooser', async () => {
    const { platform, calls } = platformFor()
    const empty = await platform.startUploadFromPaths([], true); assert.equal(empty.getChildrens().length, 0); assert.equal(calls.length, 0)
    const single = await platform.startUploadFromPaths(['/local/plain.txt', '/local/folder']); assert.equal(single.getChildrens().length, 1); assert.equal(single.getChildrens()[0].getName(), 'plain.txt')
})
await check('partial preparation error cancels earlier handles and preserves failure', async () => {
    const { platform, calls, transfers } = platformFor('/local/missing')
    await assert.rejects(platform.startUploadFromPaths(['/local/folder', '/local/missing'], true), /cannot list/)
    assert.equal(transfers.length, 1); assert.equal(transfers[0].isCancelled(), true)
    assert.equal(calls.filter(c => c[0] === 'transfer.cancel').length, 1)
})
await check('directory chooser preserves existing named tree', async () => {
    const { platform } = platformFor(); const result = await platform.startUploadDirectory(['/local/folder'])
    assert.equal(result.getName(), 'folder'); assert.equal(result.getChildrens()[0].getName(), 'nested')
})
const targetElement = { closest: () => ownedElement }, ownedElement = {}
let expectedPoint = [50,40]
let hit = targetElement, errors = [], emitted = []
const globals = { document: { elementFromPoint: (x,y) => { assert.equal(x,expectedPoint[0]); assert.equal(y,expectedPoint[1]); return hit } }, window: { devicePixelRatio: 2 } }
const directive = load(source('tabby-tauri/src/fileDrop.directive.ts'), { 'tabby-core': core, '../../tabby-core/src/directives/dropZone.directive': {}, './api/hostBridge': {}, './services/platform.service': {} }, globals).TauriFileDropDirective
function instance(platform) { return new directive({ nativeElement: ownedElement }, { transfer: { emit: t => emitted.push(t) } }, platform, { error: e => errors.push(e) }) }
async function emitDrop(platform) { platform.fileDropped$.next(event); await new Promise(resolve => setImmediate(resolve)) }
const event = { paths: ['/local/folder','/local/plain.txt'], x: 100, y: 80 }
await check('native SFTP drop emits exactly one shared tree at scaled coordinates', async () => {
    const { platform } = platformFor(), d = instance(platform); d.ngOnInit(); await emitDrop(platform)
    assert.equal(emitted.length, 1); assert.ok(emitted[0] instanceof core.DirectoryUpload); assert.equal(emitted[0].getChildrens().length, 2); d.ngOnDestroy()
})
await check('macOS drop uses logical AppKit coordinates at Retina scale', async () => {
    expectedPoint = [100,80]; const { platform } = platformFor(); platform.runtimeInfo.platform = 'macos'; const d = instance(platform)
    d.ngOnInit(); const count = emitted.length; await emitDrop(platform)
    assert.equal(emitted.length, count + 1); assert.equal(emitted.at(-1).getChildrens().length, 2); d.ngOnDestroy(); expectedPoint = [50,40]
})
await check('another drop zone or overlay does not upload', async () => {
    hit = { closest: () => ({ other: true }) }; const { platform, calls } = platformFor(); const d = instance(platform); d.ngOnInit(); await emitDrop(platform)
    assert.equal(calls.length, 0); d.ngOnDestroy(); hit = targetElement
})
await check('failed native preparation reports actual error', async () => {
    const { platform } = platformFor('/local/folder'); const d = instance(platform); d.ngOnInit(); const count = errors.length; await emitDrop(platform)
    assert.equal(errors.length, count + 1); assert.match(errors.at(-1), /cannot list/); d.ngOnDestroy()
})
await check('destruction during preparation cancels and never emits', async () => {
    let resolve; const waiting = new Promise(r => resolve = r); const { platform } = platformFor(); const result = await platform.startUploadFromPaths(['/local/folder'], true)
    const waitingPlatform = { startUploadFromPaths: () => waiting, getFileDropPosition: () => ({ x:50,y:40 }), fileDropped$: new rx.Subject() }
    const d = instance(waitingPlatform); d.ngOnInit(); const count = emitted.length
    waitingPlatform.fileDropped$.next(event); d.ngOnDestroy(); resolve(result); await new Promise(r => setImmediate(r)); assert.equal(emitted.length, count)
    assert.equal(result.getChildrens()[0].getChildrens()[0].getChildrens()[0].isCancelled(), true)
})
await check('immediate drop after reopening reaches only current owner without async registration', async () => {
    const { platform, calls } = platformFor(), d = instance(platform); d.ngOnInit(); d.ngOnDestroy()
    const reopened = instance(platform); reopened.ngOnInit(); const count = emitted.length; await emitDrop(platform)
    assert.equal(emitted.length, count + 1); assert.equal(calls.filter(c => c[0] === 'transfer.listDirectory').length, 2)
    reopened.ngOnDestroy(); await emitDrop(platform); assert.equal(emitted.length, count + 1)
})
const decoratorType = load(source('tabby-terminal/src/api/decorator.ts')).TerminalDecorator
const terminalModule = load(source('tabby-tauri/src/pathDrop.ts'), { 'tabby-terminal': { TerminalDecorator: decoratorType, encodeTerminalPath: p => 'quoted:' + p }, '../../tabby-local/src/components/terminalTab.component': { TerminalTabComponent: class {} } }, globals)
await check('terminal receives paths only when SFTP does not own drop', async () => {
    const sent = [], platform = platformFor().platform, d = new terminalModule.TauriPathDropDecorator(platform)
    const terminal = { content: { nativeElement: { getBoundingClientRect: () => ({ left:0,right:100,top:0,bottom:100 }) } }, config: { store: { terminal: { bracketedPaste:false } } }, sendInput:p => sent.push(p) }
    d.attach(terminal)
    platform.fileDropped$.next(event); assert.equal(sent.length,0)
    hit = { closest: () => null }; platform.fileDropped$.next(event); assert.deepEqual(sent,['quoted:/local/folder','quoted:/local/plain.txt']); d.detach(terminal); platform.fileDropped$.next(event); assert.equal(sent.length,2); hit = targetElement
})
await check('bootstrap waits for one native file-drop listener before rendering consumers', async () => {
    let resolve, nativeHandler; const waiting = new Promise(r => resolve = r), registrations = []
    const nativeBridge = {
        listen: (name, handler) => { registrations.push(name); if (name === 'desktop:fileDrop') { nativeHandler = handler; return waiting } return Promise.resolve(() => {}) },
        invoke: async () => '',
    }
    const platform = new native.TauriPlatformService(nativeBridge, { platform: 'windows' }, {})
    assert.equal(registrations.filter(n => n === 'desktop:fileDrop').length, 1)
    const ast = ts.createSourceFile('index.ts', source('tabby-tauri/src/index.ts'), ts.ScriptTarget.Latest, true)
    const node = ast.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'initializeDesktop')
    const factory = load(ts.createPrinter().printNode(ts.EmitHint.Unspecified, node, ast) + '\nexports.factory = initializeDesktop').factory
    let initialized = false; const pending = factory({ initialize: async () => initialized = true }, platform)()
    await Promise.resolve(); assert.equal(initialized, false, 'bootstrap must await native registration')
    resolve(() => {}); await pending; assert.equal(initialized, true)
    const events = []; platform.fileDropped$.subscribe(e => events.push(e)); nativeHandler(event); assert.equal(events.length, 1)
})
const sharedController = load(source('tabby-ssh/src/components/sftpPanel.controller.ts'), {
    'tabby-core': core, constants: require('node:constants'), path: require('node:path'), './sftpCreateDirectoryModal.component': {},
})
const sessionType = load(source('tabby-tauri/src/ssh/sftp.ts'), { 'tabby-core': core }).TauriSftpSession
const panelType = load(source('tabby-tauri/src/ssh/sftpPanel.component.ts'), {
    'tabby-core': core, path: require('node:path'), '../../../tabby-ssh/src/api/sftpContextMenu': {},
    '../../../tabby-ssh/src/components/sftpPanel.controller': sharedController, './sftpPanelTransport': {}, './sftpDeleteModal.component': {},
    '../../../tabby-ssh/src/components/sftpPanel.component.pug': '', '../../../tabby-ssh/src/components/sftpPanel.component.scss': '',
}).TauriSftpPanelComponent
async function uploadFixture(failure) {
    const { platform, calls, transfers } = platformFor(), invoke = platform.bridge.invoke, reads = new Set(), remoteCalls = [], messages = [], prompts = []
    platform.bridge.invoke = async (command, request) => {
        if (command === 'transfer.read') {
            if (reads.has(request.id)) return []
            reads.add(request.id); return invoke(command, request)
        }
        if (command === 'transfer.close') { calls.push([command, request]); return null }
        if (!command.startsWith('sftp.')) return invoke(command, request)
        remoteCalls.push([command, request])
        if (command === 'sftp.open') return { id: 'sftp-test' }
        if (command === 'sftp.uploadOpen') {
            const error = failure(request)
            if (error) throw new Error(error)
            return { id: 'remote-' + remoteCalls.length }
        }
        if (['sftp.mkdir', 'sftp.write', 'sftp.closeTransfer', 'sftp.cancelTransfer'].includes(command)) return {}
        throw new Error('Unexpected remote command: ' + command)
    }
    const panel = Object.create(panelType.prototype)
    panel.platform = platform; panel.path = '/remote'; panel.sftp = await sessionType.open(platform.bridge, 'ssh-test')
    panel.notifications = { error: m => messages.push(m) }; panel.translate = { instant: text => text }
    panel.navigate = async () => {}; platform.showMessageBox = async options => { prompts.push(options); return { response: 0 } }
    return { platform, panel, calls, transfers, remoteCalls, messages, prompts }
}
await check('remote open failure cancels every prepared tree leaf and reports original error', async () => {
    const f = await uploadFixture(() => 'remote denied')
    const root = await f.platform.startUploadFromPaths(['/local/folder', '/local/plain.txt'], true)
    await f.panel.uploadOneFolder(root)
    assert.deepEqual(f.messages, ['remote denied']); assert.ok(f.transfers.every(t => t.isCancelled()))
    assert.equal(f.calls.filter(c => c[0] === 'transfer.cancel').length, 2)
    assert.equal(f.remoteCalls.filter(c => c[0] === 'sftp.uploadOpen').length, 1)
})
await check('later child failure keeps completed upload and cancels untouched nested leaves', async () => {
    const f = await uploadFixture(r => r.path.endsWith('plain.txt') ? 'second remote denied' : null)
    const root = await f.platform.startUploadFromPaths(['/local/folder', '/local/plain.txt', '/local/folder'], true)
    await f.panel.uploadOneFolder(root)
    assert.deepEqual(f.messages, ['second remote denied'])
    assert.deepEqual(f.transfers.map(t => t.getState()), ['completed', 'cancelled', 'cancelled'])
    const cancelled = f.calls.filter(c => c[0] === 'transfer.cancel').map(c => c[1].id)
    assert.deepEqual(cancelled, f.transfers.slice(1).map(t => t.id))
    assert.equal(f.remoteCalls.filter(c => c[0] === 'sftp.uploadOpen').length, 2)
})
await check('failed overwrite retry cancels current and remaining files without cancelling prior success', async () => {
    const f = await uploadFixture(r => r.path.endsWith('plain.txt') ? r.overwritePolicy === 'skip' ? 'already exists' : 'overwrite denied' : null)
    const root = await f.platform.startUploadFromPaths(['/local/folder', '/local/plain.txt', '/local/folder'], true)
    await f.panel.uploadOneFolder(root)
    assert.deepEqual(f.messages, ['overwrite denied']); assert.equal(f.prompts.length, 1)
    assert.deepEqual(f.transfers.map(t => t.getState()), ['completed', 'cancelled', 'cancelled'])
    assert.deepEqual(f.remoteCalls.filter(c => c[0] === 'sftp.uploadOpen').map(c => c[1].overwritePolicy), ['skip', 'skip', 'overwrite'])
})
console.log(`Native SFTP drop: ${checks.length} passed; ${failures.length} failed; native IPC and DOM boundary simulated`)
assert.equal(failures.length, 0, failures.join('; '))
