import assert from 'node:assert/strict'
import fs from 'node:fs'
import fsp from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'
import { Subject } from 'rxjs'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = file => fs.readFileSync(path.join(root, file), 'utf8')
const upstream = file => execFileSync('git', ['show', `14e2d60:${file}`], { cwd: root, encoding: 'utf8' })
const treeOf = (name, text) => ts.createSourceFile(`${name}.ts`, text, ts.ScriptTarget.Latest, true)

function classText (text, name) {
    const tree = treeOf(name, text)
    const node = tree.statements.find(item => ts.isClassDeclaration(item) && item.name?.text === name)
    assert.ok(node, `class ${name} must exist`)
    return ts.createPrinter().printNode(ts.EmitHint.Unspecified, node, tree)
}

function memberText (text, className, memberName) {
    const tree = treeOf(className, text)
    const cls = tree.statements.find(item => ts.isClassDeclaration(item) && item.name?.text === className)
    assert.ok(cls, `class ${className} must exist`)
    const member = cls.members.find(item => (ts.isMethodDeclaration(item) || ts.isGetAccessorDeclaration(item)) && item.name?.getText(tree) === memberName)
    assert.ok(member, `${className}.${memberName} must exist`)
    return ts.createPrinter().printNode(ts.EmitHint.Unspecified, member, tree)
}

function functionText (text, name) {
    const tree = treeOf(name, text)
    const node = tree.statements.find(item => ts.isFunctionDeclaration(item) && item.name?.text === name)
    assert.ok(node, `function ${name} must exist`)
    return ts.createPrinter().printNode(ts.EmitHint.Unspecified, node, tree)
}

function transpile (name, text, globals = {}) {
    const module = { exports: {} }
    const output = ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS } }).outputText
    vm.runInNewContext(output, { module, exports: module.exports, Error, Uint8Array, ...globals }, { filename: `${name}.js` })
    return module.exports
}

const corePlatform = source('tabby-core/src/api/platform.ts')
const coreTransfer = source('tabby-core/src/api/fileTransfer.ts')
const nativePlatformSource = source('tabby-tauri/src/services/platform.service.ts')
const nativePanelSource = source('tabby-tauri/src/ssh/sftpPanel.component.ts')
const sharedPanelSource = source('tabby-ssh/src/components/sftpPanel.controller.ts')
const transportSource = source('tabby-tauri/src/ssh/sftpPanelTransport.ts')
const sessionSource = source('tabby-tauri/src/ssh/sftp.ts')
const editorSource = source('tabby-tauri/src/ssh/sftpEditor.ts')
const core = transpile('core', [
    'let fileTransferSequence = 0', coreTransfer,
    classText(corePlatform, 'FileTransfer'), classText(corePlatform, 'FileDownload'),
    classText(corePlatform, 'DirectoryDownload'),
    'module.exports = { FileTransfer, FileDownload, DirectoryDownload }',
].join('\n'), { Subject })

const NativeSession = transpile('native-session', [
    classText(sessionSource, 'TauriSftpSession'), 'module.exports = { TauriSftpSession }',
].join('\n'), { Subject }).TauriSftpSession
const nativeTransportExports = transpile('native-transport', [
    functionText(transportSource, 'toPanelFile'), functionText(transportSource, 'toPanelError'),
    ...(treeOf('transport', transportSource).statements.some(node => ts.isFunctionDeclaration(node) && node.name?.text === 'getSftpDownloadSize') ? [functionText(transportSource, 'getSftpDownloadSize')] : []),
    classText(transportSource, 'TauriSftpPanelTransport'),
    'module.exports = { TauriSftpPanelTransport, getSftpDownloadSize: typeof getSftpDownloadSize === "undefined" ? undefined : getSftpDownloadSize }',
].join('\n'), { Date, Error })
const NativeTransport = nativeTransportExports.TauriSftpPanelTransport

const NativePlatform = transpile('native-platform', [
    classText(nativePlatformSource, 'TauriFileDownload'),
    classText(nativePlatformSource, 'TauriDirectoryDownload'),
    `class NativePlatform { ${memberText(nativePlatformSource, 'TauriPlatformService', 'startDownload')} ${memberText(nativePlatformSource, 'TauriPlatformService', 'prepareEditableFile')} ${memberText(nativePlatformSource, 'TauriPlatformService', 'startDownloadDirectory')} ${memberText(nativePlatformSource, 'TauriPlatformService', 'pickDirectory')} }`,
    'module.exports = { NativePlatform }',
].join('\n'), {
    FileDownload: core.FileDownload, DirectoryDownload: core.DirectoryDownload,
    sanitizeTransferName: name => name.replace(/[\\/]/g, '_'),
    sanitizeTransferRelativePath: name => name.split('/').filter(part => part && part !== '.' && part !== '..').join('/'),
}).NativePlatform

const NativePanel = transpile('native-panel', [
    `class SFTPPanelController { ${memberText(sharedPanelSource, 'SFTPPanelController', 'downloadFolder')} }`,
    `class TauriSftpPanelComponent extends SFTPPanelController { notifications = { error: error => this.showError(error) }; ${memberText(nativePanelSource, 'TauriSftpPanelComponent', 'open')} ${memberText(nativePanelSource, 'TauriSftpPanelComponent', 'download')} ${memberText(nativePanelSource, 'TauriSftpPanelComponent', 'downloadDirectory')} ${memberText(nativePanelSource, 'TauriSftpPanelComponent', 'downloadFolderRecursive')} ${memberText(nativePanelSource, 'TauriSftpPanelComponent', 'calculateFolderSizeAndUpdate')} ${memberText(nativePanelSource, 'TauriSftpPanelComponent', 'downloadItem')} ${memberText(nativePanelSource, 'TauriSftpPanelComponent', 'showError')} }`,
    'module.exports = { NativePanel: TauriSftpPanelComponent }',
].join('\n'), { Error, posixPath: path.posix, getSftpDownloadSize: nativeTransportExports.getSftpDownloadSize }).NativePanel
const NativeEditor = transpile('native-editor', [
    `class TauriSftpEditor { ${memberText(editorSource, 'TauriSftpEditor', 'edit')} }`,
    'module.exports = { TauriSftpEditor }',
].join('\n'), { Error, getSftpDownloadSize: nativeTransportExports.getSftpDownloadSize, setTimeout: () => 1, clearTimeout () {} }).TauriSftpEditor

const REMOTE_BYTES = Buffer.from([0, 255, 17, 128, 1])
const remoteEntries = {
    '/remote/unknown.bin': { name: 'unknown.bin', fullPath: '/remote/unknown.bin', isDirectory: false, isSymlink: false, mode: 0o100640, size: null, modified: null, isOperable: true },
    '/remote/empty.bin': { name: 'empty.bin', fullPath: '/remote/empty.bin', isDirectory: false, isSymlink: false, mode: 0o100600, size: 0, modified: null, isOperable: true },
    '/remote/known.bin': { name: 'known.bin', fullPath: '/remote/known.bin', isDirectory: false, isSymlink: false, mode: 0o100644, size: REMOTE_BYTES.length, modified: null, isOperable: true },
    '/remote/alias': { name: 'alias', fullPath: '/remote/alias', isDirectory: false, isSymlink: true, mode: 0o120777, size: 5, modified: null, isOperable: true },
    '/remote/folder': { name: 'folder', fullPath: '/remote/folder', isDirectory: true, isSymlink: false, mode: 0o40755, size: 0, modified: null, isOperable: true },
    '/remote/folder/unknown.bin': { name: 'unknown.bin', fullPath: '/remote/folder/unknown.bin', isDirectory: false, isSymlink: false, mode: 0o100640, size: null, modified: null, isOperable: true },
}

function harness (base, options = {}) {
    const calls = { opened: [], prepared: [], errors: [], bridge: [], events: [] }
    const active = new Map()
    const sftp = new Map()
    let id = 0
    const entries = { ...remoteEntries,
        '/remote/alias': { ...remoteEntries['/remote/alias'], size: options.aliasSize === undefined ? 5 : options.aliasSize },
        '/remote/regular-wrong': { ...remoteEntries['/remote/known.bin'], name: 'regular-wrong', fullPath: '/remote/regular-wrong', size: options.regularSize ?? 2 },
    }
    const bridge = { async invoke (command, request = {}) {
        calls.bridge.push({ command, request })
        if (command === 'fileEdit.ready' || command === 'desktop.openPath' || command === 'fileEdit.stop') calls.events.push({ command, request })
        switch (command) {
            case 'sftp.open': return { id: 'session' }
            case 'sftp.close': sftp.clear(); return null
            case 'sftp.list': return request.path === '/remote/folder' ? [entries['/remote/folder/unknown.bin']] : [entries['/remote/unknown.bin'], entries['/remote/alias'], entries['/remote/folder'], entries['/remote/empty.bin'], entries['/remote/known.bin'], entries['/remote/regular-wrong']]
            case 'sftp.stat': return request.follow && request.path === '/target' ? { ...entries['/remote/unknown.bin'], name: 'target', fullPath: '/target', size: null } : entries[request.path]
            case 'sftp.readlink': return '../target'
            case 'dialog.save': return path.join(base, request.fileName)
            case 'dialog.open': return [path.join(base, 'folder')]
            case 'sftp.downloadOpen': {
                const transferId = `sftp-${++id}`
                sftp.set(transferId, { path: request.path, offset: 0 })
                return { id: transferId, direction: 'download', name: request.path, size: null, transferred: 0, state: 'running' }
            }
            case 'sftp.read': {
                const transfer = sftp.get(request.transferId)
                assert.ok(transfer, `SFTP stream ${request.transferId} should be active`)
                if (options.failRead || (options.failAfterBytes !== undefined && transfer.offset >= options.failAfterBytes)) throw new Error('fixture read failure')
                if (options.closeDuringRead && options.closeAfterBytes !== undefined && transfer.offset >= options.closeAfterBytes) {
                    await options.closeDuringRead()
                    throw new Error('fixture session closed during read')
                }
                const bytes = options.targetBytes ?? (transfer.path === '/remote/empty.bin' ? Buffer.alloc(0) : REMOTE_BYTES)
                const chunk = bytes.subarray(transfer.offset, transfer.offset + Math.min(request.maxBytes, 2))
                transfer.offset += chunk.length
                return Array.from(chunk)
            }
            case 'sftp.closeTransfer': sftp.delete(request.transferId); return { id: request.transferId, direction: 'download', name: '', size: null, transferred: 0, state: 'completed' }
            case 'sftp.cancelTransfer': sftp.delete(request.transferId); return null
            case 'transfer.openDownload': {
                const destination = request.destination || path.join(base, request.relativePath)
                fs.mkdirSync(path.dirname(destination), { recursive: true })
                const transferId = `file-${++id}`
                const stage = `${destination}.stage-${transferId}`
                fs.writeFileSync(stage, Buffer.alloc(0))
                active.set(transferId, { destination, stage, size: request.size ?? null, bytes: 0 })
                calls.opened.push({ ...request, transferId, destination })
                return { id: transferId, direction: 'download', name: request.name, size: request.size ?? null, transferred: 0, state: 'pending' }
            }
            case 'transfer.write': {
                const transfer = active.get(request.id)
                assert.ok(transfer, `local transfer ${request.id} should be active`)
                const bytes = Buffer.from(request.data)
                if (transfer.size !== null && transfer.bytes + bytes.length > transfer.size) throw new Error('transfer exceeds advertised size')
                fs.appendFileSync(transfer.stage, bytes)
                transfer.bytes += bytes.length
                return null
            }
            case 'transfer.close': {
                const transfer = active.get(request.id)
                assert.ok(transfer, `local transfer ${request.id} should be active`)
                active.delete(request.id)
                if (transfer.size !== null && transfer.bytes !== transfer.size) {
                    fs.rmSync(transfer.stage, { force: true })
                    throw new Error('download size does not match the advertised size')
                }
                fs.renameSync(transfer.stage, transfer.destination)
                return null
            }
            case 'transfer.cancel': {
                const transfer = active.get(request.id)
                active.delete(request.id)
                if (transfer) fs.rmSync(transfer.stage, { force: true })
                return null
            }
            case 'transfer.createDirectory': fs.mkdirSync(path.join(base, request.relativePath), { recursive: true }); return null
            case 'fileEdit.prepare': {
                calls.prepared.push(request)
                const transferId = `edit-${++id}`
                active.set(transferId, { destination: path.join(base, 'editable'), stage: path.join(base, `editable.stage-${transferId}`), size: request.size ?? null, bytes: 0 })
                fs.writeFileSync(active.get(transferId).stage, Buffer.alloc(0))
                return { id: 'workspace', path: path.join(base, 'editable'), transfer: { id: transferId, name: request.name, size: request.size ?? null } }
            }
            case 'fileEdit.ready':
            case 'desktop.openPath':
            case 'fileEdit.stop': return null
            default: throw new Error(`unexpected bridge command ${command}`)
        }
    } }
    const platform = new NativePlatform()
    platform.bridge = bridge
    platform.fileTransferStarted = { next () {} }
    const panel = new NativePanel()
    panel.platform = platform
    panel.notifications = { error: error => calls.errors.push(String(error?.message ?? error)) }
    return { bridge, platform, panel, calls, active, sftp }
}

async function nativeHarness (base, options) {
    const env = harness(base, options)
    const session = await NativeSession.open(env.bridge, 'ssh')
    env.panel.sftp = new NativeTransport(session)
    env.session = session
    openSessions.push(session)
    return env
}

function nativeEditor (env) {
    const editor = new NativeEditor()
    editor.bridge = env.bridge
    editor.platform = env.platform
    editor.notifications = { error: error => env.calls.errors.push(String(error?.message ?? error)) }
    const prepare = env.platform.prepareEditableFile.bind(env.platform)
    env.platform.prepareEditableFile = async (...args) => {
        const workspace = await prepare(...args)
        env.calls.editTransfer = workspace.transfer
        return workspace
    }
    return editor
}

async function fixedUpstreamEdit (base, listedSize, targetBytes) {
    const refPlatform = upstream('tabby-electron/src/services/platform.service.ts')
    const refEditor = upstream('tabby-electron/src/sftpContextMenu.ts')
    const ReferenceDownload = transpile('reference-download', `${classText(refPlatform, 'ElectronFileDownload')}; module.exports = ElectronFileDownload`, { FileDownload: core.FileDownload, fs: fsp, path, Error })
    const ReferenceEditor = transpile('reference-editor', `class EditSFTPContextMenu { ${memberText(refEditor, 'EditSFTPContextMenu', 'edit')} }; module.exports = EditSFTPContextMenu`, {
        path, fs, Error, Subject, tmp: { dir: async () => ({ path: base }) },
        resolveInsideBase: (dir, name) => path.resolve(dir, name), setTimeout: () => 1, clearTimeout () {},
    })
    const observed = { size: undefined, mode: undefined, name: undefined, path: undefined }
    const editor = new ReferenceEditor()
    editor.platform = {
        async startDownload (name, mode, size, filePath) {
            Object.assign(observed, { name, mode, size, destination: filePath })
            const transfer = new ReferenceDownload(filePath, mode, size, { powerSaveBlocker: { start: () => 1, stop () {} } })
            observed.transfer = transfer
            await transfer.open()
            return transfer
        },
        openPath: filePath => { observed.openPath = filePath },
    }
    let fileClosed
    editor.sftp = { closed$: new Subject(), download (remotePath, transfer) {
        observed.path = remotePath
        const closeFile = transfer.file.close.bind(transfer.file)
        transfer.file.close = () => { fileClosed = closeFile(); return fileClosed }
        return (async () => { await transfer.write(targetBytes); transfer.close() })()
    } }
    await editor.edit({ name: 'alias', fullPath: '/remote/alias', mode: 0o120777, size: listedSize }, editor.sftp)
    await fileClosed
    return { observed, actualBytes: fs.readFileSync(path.join(base, 'alias')) }
}

const failures = []
const openSessions = []
let cases = 0
async function check (name, run) {
    cases++
    const base = fs.mkdtempSync(path.join(os.tmpdir(), 'sftp-unknown-size-'))
    try { await run(base); console.log(`${name}: PASS`) } catch (error) { failures.push({ name, error }); console.error(`${name}: FAIL: ${error.message}`) } finally {
        await Promise.allSettled(openSessions.splice(0).map(session => session.close()))
        fs.rmSync(base, { recursive: true, force: true })
    }
}

await check('fixed 14e2d60 known-size reference still streams a binary file to the actual filesystem', async base => {
    const refPlatform = upstream('tabby-electron/src/services/platform.service.ts')
    const refPanel = upstream('tabby-ssh/src/components/sftpPanel.component.ts')
    const refDownload = classText(refPlatform, 'ElectronFileDownload')
    const ReferenceDownload = transpile('reference-download', `${refDownload}; module.exports = ElectronFileDownload`, { FileDownload: core.FileDownload, fs: fsp, path, Error })
    const ReferencePanel = transpile('reference-panel', `class SFTPPanelComponent { ${memberText(refPanel, 'SFTPPanelComponent', 'download')} }; module.exports = SFTPPanelComponent`, { path, Error })
    const observed = {}
    const panel = new ReferencePanel()
    panel.platform = { async startDownload (name, mode, size) {
        observed.size = size
        const transfer = new ReferenceDownload(path.join(base, name), mode, size, { powerSaveBlocker: { start: () => 1, stop () {} } })
        await transfer.open()
        return transfer
    } }
    let stream
    let fileClosed
    panel.sftp = { download (_remotePath, transfer) {
        const originalClose = transfer.file.close.bind(transfer.file)
        transfer.file.close = () => { fileClosed = originalClose(); return fileClosed }
        stream = (async () => { await transfer.write(REMOTE_BYTES); transfer.close() })()
        return stream
    } }
    await panel.download('/remote/unknown.bin', 0o600, 0)
    await stream
    await fileClosed
    assert.equal(observed.size, 0, 'the fixed original shared-panel path advertised the backend fallback zero')
    assert.deepEqual(fs.readFileSync(path.join(base, 'unknown.bin')), REMOTE_BYTES)
})

for (const [listedSize, targetBytes] of [[0, REMOTE_BYTES], [2, REMOTE_BYTES], [9, REMOTE_BYTES], [7, Buffer.alloc(0)]]) {
    await check(`fixed 14e2d60 edit accepts alias size ${listedSize} for ${targetBytes.length} target bytes`, async base => {
        const { observed, actualBytes } = await fixedUpstreamEdit(base, listedSize, targetBytes)
        assert.equal(observed.name, 'alias')
        assert.equal(observed.mode, 0o120777)
        assert.equal(observed.size, listedSize)
        assert.equal(observed.path, '/remote/alias')
        assert.equal(observed.transfer.getSize(), listedSize)
        assert.equal(observed.transfer.getTotalSize(), listedSize)
        assert.equal(observed.transfer.getCompletedBytes(), targetBytes.length)
        assert.deepEqual(actualBytes, targetBytes)
        assert.equal(observed.openPath, observed.destination)
    })
}

await check('native direct unknown-size file carries null through the panel adapter as unknown and closes at EOF', async base => {
    const { panel, calls, active, sftp } = await nativeHarness(base)
    const [entry] = await panel.sftp.readdir('/remote')
    assert.equal(entry.size, 0, 'unknown wire size keeps the original shared-panel display value')
    assert.equal(entry.transferSize, undefined, 'unknown wire size remains absent for transfer validation')
    await panel.download(entry)
    assert.deepEqual(fs.readFileSync(path.join(base, 'unknown.bin')), REMOTE_BYTES)
    assert.ok(calls.opened[0].size == null, 'unknown advertised transfer size must remain absent at IPC')
    assert.equal(active.size, 0)
    assert.equal(sftp.size, 0)
    assert.deepEqual(calls.errors, [])
})

await check('native followed file link downloads unknown-size target bytes with the followed target mode', async base => {
    const { panel, calls } = await nativeHarness(base)
    const target = await panel.sftp.stat('/target', true)
    assert.equal(target.size, 0, 'unknown target keeps the original UI display size')
    assert.equal(target.transferSize, undefined, 'unknown target retains absent transfer size')
    const [, alias] = await panel.sftp.readdir('/remote')
    await panel.open(alias)
    assert.deepEqual(fs.readFileSync(path.join(base, 'alias')), REMOTE_BYTES)
    assert.equal(calls.opened[0].mode, 0o100640)
    assert.ok(calls.opened[0].size == null, 'followed target unknown size must remain absent at IPC')
    assert.deepEqual(calls.errors, [])
})

for (const advertisedSize of [0, 2, 9]) {
    await check(`native symbolic-link edit with listed size ${advertisedSize} keeps alias progress and copies target bytes to EOF`, async base => {
        const env = await nativeHarness(base, { aliasSize: advertisedSize })
        const [ , alias ] = await env.panel.sftp.readdir('/remote')
        const statCalls = env.calls.bridge.filter(call => call.command === 'sftp.stat' || call.command === 'sftp.readlink').length
        const editor = nativeEditor(env)
        await editor.edit(alias, env.panel.sftp)
        const transfer = env.calls.editTransfer
        assert.equal(env.calls.prepared[0].name, 'alias')
        assert.equal(env.calls.prepared[0].mode, 0o120777)
        assert.equal(env.calls.prepared[0].size, undefined, 'link target size is unknown to the native transfer guard')
        assert.equal(env.calls.bridge.find(call => call.command === 'sftp.downloadOpen').request.path, alias.fullPath)
        assert.deepEqual(fs.readFileSync(path.join(base, 'editable')), REMOTE_BYTES)
        assert.equal(transfer.getSize(), advertisedSize, 'UI total retains the alias listed size')
        assert.equal(transfer.getTotalSize(), advertisedSize)
        assert.equal(transfer.getCompletedBytes(), REMOTE_BYTES.length)
        assert.deepEqual(env.calls.events.filter(event => event.command === 'desktop.openPath').map(event => event.request.path), [path.join(base, 'editable')])
        assert.equal(env.calls.bridge.filter(call => call.command === 'sftp.stat' || call.command === 'sftp.readlink').length, statCalls, 'edit does not add stat or readlink requests')
        assert.equal(env.active.size, 0)
        assert.equal(env.sftp.size, 0)
        assert.deepEqual(env.calls.errors, [])
    })
}

await check('native nonzero listed symbolic link to an empty target completes successfully', async base => {
    const env = await nativeHarness(base, { aliasSize: 7, targetBytes: Buffer.alloc(0) })
    const [, alias] = await env.panel.sftp.readdir('/remote')
    await nativeEditor(env).edit(alias, env.panel.sftp)
    assert.equal(env.calls.prepared[0].size, undefined)
    assert.deepEqual(fs.readFileSync(path.join(base, 'editable')), Buffer.alloc(0))
    assert.equal(env.calls.editTransfer.getSize(), 7)
    assert.equal(env.calls.editTransfer.getCompletedBytes(), 0)
    assert.equal(env.calls.prepared[0].size, undefined)
    assert.equal(env.calls.events.filter(event => event.command === 'desktop.openPath').length, 1)
    assert.equal(env.active.size, 0)
    assert.equal(env.sftp.size, 0)
})

await check('native unknown-size symbolic link shows zero UI total and sends unknown IPC size', async base => {
    const env = await nativeHarness(base, { aliasSize: null })
    const [, alias] = await env.panel.sftp.readdir('/remote')
    assert.equal(alias.size, 0)
    assert.equal(alias.transferSize, undefined)
    await nativeEditor(env).edit(alias, env.panel.sftp)
    assert.equal(env.calls.prepared[0].size, undefined)
    assert.equal(env.calls.editTransfer.getSize(), 0)
    assert.equal(env.calls.editTransfer.getTotalSize(), 0)
    assert.equal(env.calls.editTransfer.getCompletedBytes(), REMOTE_BYTES.length)
    assert.deepEqual(fs.readFileSync(path.join(base, 'editable')), REMOTE_BYTES)
})

await check('native ordinary file with incorrect positive size rejects and never opens editor', async base => {
    const env = await nativeHarness(base, { regularSize: REMOTE_BYTES.length + 2 })
    const entries = await env.panel.sftp.readdir('/remote')
    const ordinary = entries.find(entry => entry.name === 'regular-wrong')
    await nativeEditor(env).edit(ordinary, env.panel.sftp)
    assert.equal(env.calls.prepared[0].size, REMOTE_BYTES.length + 2)
    assert.equal(env.calls.events.some(event => event.command === 'fileEdit.ready' || event.command === 'desktop.openPath'), false)
    assert.equal(env.calls.events.some(event => event.command === 'fileEdit.stop'), true)
    assert.ok(env.calls.errors.some(error => /download size does not match/.test(error)))
    assert.equal(env.active.size, 0)
    assert.equal(env.sftp.size, 0)
    assert.deepEqual(fs.readdirSync(base).filter(name => name.includes('.stage-')), [])
})

await check('native ordinary file advertised as zero still rejects nonempty bytes and never opens editor', async base => {
    const env = await nativeHarness(base, { regularSize: 0 })
    const entries = await env.panel.sftp.readdir('/remote')
    const ordinary = entries.find(entry => entry.name === 'regular-wrong')
    await nativeEditor(env).edit(ordinary, env.panel.sftp)
    assert.equal(env.calls.prepared[0].size, 0)
    assert.equal(env.calls.events.some(event => event.command === 'fileEdit.ready' || event.command === 'desktop.openPath'), false)
    assert.equal(env.calls.events.some(event => event.command === 'fileEdit.stop'), true)
    assert.ok(env.calls.errors.some(error => /transfer exceeds advertised size/.test(error)))
    assert.equal(env.active.size, 0)
    assert.equal(env.sftp.size, 0)
    assert.deepEqual(fs.readdirSync(base).filter(name => name.includes('.stage-')), [])
})

await check('native symbolic-link read failure cancels local and remote handles and never opens editor', async base => {
    const env = await nativeHarness(base, { failAfterBytes: 2 })
    const [, alias] = await env.panel.sftp.readdir('/remote')
    await nativeEditor(env).edit(alias, env.panel.sftp)
    assert.equal(env.calls.events.some(event => event.command === 'fileEdit.ready' || event.command === 'desktop.openPath'), false)
    assert.equal(env.calls.events.some(event => event.command === 'fileEdit.stop'), true)
    assert.ok(env.calls.errors.some(error => /fixture read failure/.test(error)))
    assert.equal(env.calls.bridge.filter(call => call.command === 'transfer.write').length, 1, 'failure follows one successfully written chunk')
    assert.equal(env.active.size, 0)
    assert.equal(env.sftp.size, 0)
    assert.equal(env.calls.bridge.some(call => call.command === 'sftp.cancelTransfer'), true)
    assert.equal(env.calls.bridge.some(call => call.command === 'transfer.cancel'), true)
    assert.deepEqual(fs.readdirSync(base).filter(name => name.includes('.stage-')), [])
})

await check('native session close during pending symbolic-link read cancels work and never opens editor', async base => {
    let closeSession
    const env = await nativeHarness(base, { closeAfterBytes: 2, closeDuringRead: async () => { await closeSession() } })
    closeSession = () => env.session.close()
    const [, alias] = await env.panel.sftp.readdir('/remote')
    await nativeEditor(env).edit(alias, env.panel.sftp)
    assert.equal(env.calls.events.some(event => event.command === 'fileEdit.ready' || event.command === 'desktop.openPath'), false)
    assert.equal(env.calls.events.some(event => event.command === 'fileEdit.stop'), true)
    assert.equal(env.calls.bridge.filter(call => call.command === 'transfer.write').length, 1, 'session closes after one successful local write')
    assert.equal(env.active.size, 0)
    assert.equal(env.sftp.size, 0)
    assert.equal(env.calls.bridge.some(call => call.command === 'transfer.cancel'), true)
    assert.deepEqual(fs.readdirSync(base).filter(name => name.includes('.stage-')), [])
    assert.deepEqual(env.calls.errors, [], 'closing the owning session suppresses a spurious error notification')
})

await check('native folder download streams unknown-size child to the owned local directory', async base => {
    const { panel, calls, active } = await nativeHarness(base)
    const [, , folder] = await panel.sftp.readdir('/remote')
    const [child] = await panel.sftp.readdir(folder.fullPath)
    assert.equal(child.size, 0, 'unknown child keeps the original UI display size')
    assert.equal(child.transferSize, undefined, 'unknown child retains absent transfer size')
    await panel.download(folder)
    assert.deepEqual(fs.readFileSync(path.join(base, 'folder', 'unknown.bin')), REMOTE_BYTES)
    assert.ok(calls.opened[0].size == null, 'folder child unknown size must remain absent at IPC')
    assert.equal(active.size, 0)
    assert.deepEqual(calls.errors, [])
})

await check('string-path download without a size uses unknown-size streaming', async base => {
    const { panel, calls, active } = await nativeHarness(base)
    await panel.download('/remote/unknown.bin')
    assert.deepEqual(calls.errors, [])
    assert.deepEqual(fs.readFileSync(path.join(base, 'unknown.bin')), REMOTE_BYTES)
    assert.ok(calls.opened[0].size == null, 'path with no advertised size must remain unknown at IPC')
    assert.equal(active.size, 0)
    assert.deepEqual(calls.errors, [])
})

await check('local editor preparation preserves unknown size through the native request and FileDownload', async base => {
    const { bridge, platform, panel, calls, active, sftp } = await nativeHarness(base)
    const [item] = await panel.sftp.readdir('/remote')
    const editor = new NativeEditor()
    editor.bridge = bridge
    editor.platform = platform
    editor.notifications = { error: error => calls.errors.push(String(error?.message ?? error)) }
    await editor.edit(item, panel.sftp)
    assert.equal(calls.prepared[0].size, undefined)
    assert.deepEqual(fs.readFileSync(path.join(base, 'editable')), REMOTE_BYTES)
    assert.equal(active.size, 0)
    assert.equal(sftp.size, 0)
    assert.deepEqual(calls.errors, [])
})

await check('known zero remains exact and permits an empty download', async base => {
    const { platform, calls } = harness(base)
    const native = await nativeHarness(path.join(base, 'adapter'))
    const empty = await native.panel.sftp.stat('/remote/empty.bin', true)
    assert.equal(empty.size, 0)
    assert.equal(empty.transferSize, 0, 'known zero must differ from unknown transfer size')
    const transfer = await platform.startDownload('empty.bin', 0o100600, 0)
    assert.equal(calls.opened[0].size, 0)
    assert.equal(transfer.getSize(), 0)
    await transfer.closeAsync()
    assert.deepEqual(fs.readFileSync(path.join(base, 'empty.bin')), Buffer.alloc(0))
    const wrongZero = await platform.startDownload('empty.bin', 0o100600, 0)
    await assert.rejects(wrongZero.write(REMOTE_BYTES), /transfer exceeds advertised size/)
    wrongZero.cancel()
    assert.deepEqual(fs.readFileSync(path.join(base, 'empty.bin')), Buffer.alloc(0), 'known zero must preserve the existing destination on overflow')
    assert.deepEqual(fs.readdirSync(base).filter(name => name.includes('.stage-')), [])
})

await check('known positive size remains exact and permits matching binary bytes', async base => {
    const { platform, calls } = harness(base)
    const transfer = await platform.startDownload('known.bin', 0o100644, REMOTE_BYTES.length)
    assert.equal(calls.opened[0].size, REMOTE_BYTES.length)
    assert.equal(transfer.getSize(), REMOTE_BYTES.length)
    await transfer.write(REMOTE_BYTES)
    await transfer.closeAsync()
    assert.deepEqual(fs.readFileSync(path.join(base, 'known.bin')), REMOTE_BYTES)
})

await check('wrong known size still rejects overflow and incomplete close without replacing old destination', async base => {
    const { platform } = harness(base)
    const destination = path.join(base, 'protected.bin')
    fs.writeFileSync(destination, Buffer.from('KEEP'))
    const overflow = await platform.startDownload('protected.bin', 0o100600, 2)
    await assert.rejects(overflow.write(REMOTE_BYTES), /transfer exceeds advertised size/)
    overflow.cancel()
    const incomplete = await platform.startDownload('protected.bin', 0o100600, 6)
    await incomplete.write(REMOTE_BYTES)
    await assert.rejects(incomplete.closeAsync(), /download size does not match the advertised size/)
    assert.deepEqual(fs.readFileSync(destination), Buffer.from('KEEP'))
    assert.deepEqual(fs.readdirSync(base).filter(name => name.includes('.stage-')), [])
})

console.log(`SFTP unknown-size renderer: ${cases - failures.length} passed; ${failures.length} failed; native IPC/picker/filesystem bridge is simulated`)
assert.equal(failures.length, 0, failures.map(item => `${item.name}: ${item.error.message}`).join('; '))
