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
const POSIX = path.posix

const sharedPanelSource = fs.readFileSync(path.join(root, 'tabby-ssh/src/components/sftpPanel.controller.ts'), 'utf8')
const panelSource = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/sftpPanel.component.ts'), 'utf8')
const sessionSource = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/sftp.ts'), 'utf8')
const bridgeSource = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/tauriHostBridge.service.ts'), 'utf8')
const tauriPlatformSource = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/platform.service.ts'), 'utf8')
const corePlatformSource = fs.readFileSync(path.join(root, 'tabby-core/src/api/platform.ts'), 'utf8')
const coreFileTransferSource = fs.readFileSync(path.join(root, 'tabby-core/src/api/fileTransfer.ts'), 'utf8')
const coreUtilsSource = fs.readFileSync(path.join(root, 'tabby-core/src/utils.ts'), 'utf8')
const upstream = file => execFileSync('git', ['show', `14e2d60:${file}`], { cwd: root, encoding: 'utf8' })
const refPanelSource = upstream('tabby-ssh/src/components/sftpPanel.component.ts')
const refPlatformSource = upstream('tabby-electron/src/services/platform.service.ts')
const refSessionSource = upstream('tabby-ssh/src/session/sftp.ts')

function treeOf (name, source) {
    return ts.createSourceFile(`${name}.ts`, source, ts.ScriptTarget.Latest, true)
}

function memberText (source, className, memberName) {
    const tree = treeOf(className, source)
    const cls = tree.statements.find(node => ts.isClassDeclaration(node) && node.name?.text === className)
    assert.ok(cls, `class ${className} must exist`)
    const member = cls.members.find(node => {
        if (ts.isConstructorDeclaration(node)) {
            return memberName === 'constructor'
        }
        return (ts.isMethodDeclaration(node) || ts.isGetAccessorDeclaration(node)) &&
            node.name?.getText(tree) === memberName
    })
    assert.ok(member, `${className}.${memberName} must exist`)
    return ts.createPrinter().printNode(ts.EmitHint.Unspecified, member, tree)
}

function classText (source, className) {
    const tree = treeOf(className, source)
    const cls = tree.statements.find(node => ts.isClassDeclaration(node) && node.name?.text === className)
    assert.ok(cls, `class ${className} must exist`)
    return ts.createPrinter().printNode(ts.EmitHint.Unspecified, cls, tree)
}

function functionText (source, name) {
    const tree = treeOf('fn', source)
    const fn = tree.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === name)
    assert.ok(fn, `function ${name} must exist`)
    return ts.createPrinter().printNode(ts.EmitHint.Unspecified, fn, tree)
}

function transpile (name, program, globals = {}) {
    const result = { exports: {} }
    const code = ts.transpileModule(program, {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS },
    }).outputText
    vm.runInNewContext(code, { module: result, exports: result.exports, ...globals }, { filename: `${name}.js` })
    return result.exports
}

const core = transpile('core', [
    'let fileTransferSequence = 0',
    coreFileTransferSource,
    classText(corePlatformSource, 'FileTransfer'),
    classText(corePlatformSource, 'FileDownload'),
    classText(corePlatformSource, 'DirectoryDownload'),
    'module.exports = { sanitizeTransferName, sanitizeTransferRelativePath, FileTransfer, FileDownload, DirectoryDownload }',
].join('\n'), { Error })

const wrapPromise = transpile('core-utils', `${functionText(coreUtilsSource, 'wrapPromise')}; module.exports = { wrapPromise }`, { Error }).wrapPromise

const russh = {
    OPEN_READ: 1,
    SFTPFileType: { File: 1, Directory: 2, Symlink: 3 },
}

const reference = {
    ...transpile('ref-session', [
        classText(refSessionSource, 'SFTPFileHandle'),
        classText(refSessionSource, 'SFTPSession'),
        'module.exports = { SFTPFileHandle, SFTPSession }',
    ].join('\n'), { Subject, russh, posixPath: POSIX, LogService: class LogService {}, Error }),
    ...transpile('ref-platform', [
        functionText(refPlatformSource, 'resolveInsideBase'),
        classText(refPlatformSource, 'ElectronFileDownload'),
        classText(refPlatformSource, 'ElectronDirectoryDownload'),
        `class ReferencePlatform { ${memberText(refPlatformSource, 'ElectronPlatformService', 'startDownload')} ${memberText(refPlatformSource, 'ElectronPlatformService', 'startDownloadDirectory')} }`,
        'module.exports = { resolveInsideBase, ElectronFileDownload, ElectronDirectoryDownload, ReferencePlatform }',
    ].join('\n'), {
        path, fs: fsp, fsSync: fs, wrapPromise,
        FileDownload: core.FileDownload, DirectoryDownload: core.DirectoryDownload, Error,
    }),
}

reference.ReferencePanel = transpile('ref-panel', [
    `class ReferencePanel { ${[
        memberText(refPanelSource, 'SFTPPanelComponent', 'downloadItem'),
        memberText(refPanelSource, 'SFTPPanelComponent', 'download'),
        memberText(refPanelSource, 'SFTPPanelComponent', 'downloadFolder'),
        memberText(refPanelSource, 'SFTPPanelComponent', 'downloadFolderRecursive'),
        memberText(refPanelSource, 'SFTPPanelComponent', 'calculateFolderSizeAndUpdate'),
    ].join('\n')} }`,
    'module.exports = { ReferencePanel }',
].join('\n'), { path: POSIX, Error }).ReferencePanel

const native = transpile('native-host', [
    functionText(bridgeSource, 'toRustCommand'),
    `class TauriHostBridge { ${memberText(bridgeSource, 'TauriHostBridge', 'api')} ${memberText(bridgeSource, 'TauriHostBridge', 'invoke')} }`,
    classText(sessionSource, 'TauriSftpSession'),
    'module.exports = { TauriHostBridge, TauriSftpSession }',
].join('\n'), { window: { __TAURI__: { core: { invoke: (command, args) => nativeBridge(command, args) } } }, Error, Subject })

let nativeBridge = () => { throw new Error('native bridge was not configured') }

native.NativePlatform = transpile('native-platform', [
    classText(tauriPlatformSource, 'TauriFileDownload'),
    classText(tauriPlatformSource, 'TauriDirectoryDownload'),
    `class NativePlatform { ${[
        memberText(tauriPlatformSource, 'TauriPlatformService', 'startDownload'),
        memberText(tauriPlatformSource, 'TauriPlatformService', 'startDownloadDirectory'),
        memberText(tauriPlatformSource, 'TauriPlatformService', 'pickDirectory'),
    ].join('\n')} }`,
    'module.exports = { NativePlatform }',
].join('\n'), {
    sanitizeTransferName: core.sanitizeTransferName,
    sanitizeTransferRelativePath: core.sanitizeTransferRelativePath,
    FileDownload: core.FileDownload, DirectoryDownload: core.DirectoryDownload, Error,
}).NativePlatform

native.NativePanel = transpile('native-panel', [
    `class FolderController { ${memberText(sharedPanelSource, 'SFTPPanelController', 'downloadFolder')} }`,
    `class NativePanel extends FolderController { notifications = { error: error => this.showError(new Error(error)) }; ${[
        memberText(panelSource, 'TauriSftpPanelComponent', 'download'),
        memberText(panelSource, 'TauriSftpPanelComponent', 'downloadDirectory'),
        memberText(panelSource, 'TauriSftpPanelComponent', 'downloadFolderRecursive'),
        memberText(panelSource, 'TauriSftpPanelComponent', 'calculateFolderSizeAndUpdate'),
        memberText(panelSource, 'TauriSftpPanelComponent', 'showError'),
    ].join('\n')} }`,
    'module.exports = { NativePanel }',
].join('\n'), { Error, posixPath: POSIX }).NativePanel

const REL_TARGET = Buffer.from(Array.from({ length: 64 }, (_, index) => (index * 27 + 191) % 256))
const ABS_TARGET = Buffer.from([0, 1, 2, 251, 252, 3, 4, 5, 6, 7, 255])

const remote = {
    files: new Map([
        ['/folder/a-before.txt', Buffer.from('ALPHA')],
        ['/folder/z-after.txt', Buffer.from('OMEGA')],
        ['/folder/sub/b-before.txt', Buffer.from('BEE')],
        ['/folder/sub/cc.txt', Buffer.from('CC')],
        ['/short/head.txt', Buffer.from('HEAD')],
        ['/short/tail.txt', Buffer.from('TAIL')],
        ['/targets/rel-target.bin', REL_TARGET],
        ['/targets/abs-target.bin', ABS_TARGET],
        ['/plain/one.txt', Buffer.from('ONE')],
        ['/plain/two.bin', Buffer.from([0, 255, 1, 254, 2, 253])],
        ['/plain/inner/deep.txt', Buffer.from('DEEP')],
        ['/display/keep.txt', Buffer.from('KEEP')],
        ['/display/display-only.bin', Buffer.from('X')],
    ]),
    modes: new Map([
        ['/folder/a-before.txt', 0o644],
        ['/folder/z-after.txt', 0o644],
        ['/folder/sub/b-before.txt', 0o600],
        ['/folder/sub/cc.txt', 0o644],
        ['/short/head.txt', 0o644],
        ['/short/tail.txt', 0o644],
        ['/targets/rel-target.bin', 0o644],
        ['/targets/abs-target.bin', 0o644],
        ['/plain/one.txt', 0o644],
        ['/plain/two.bin', 0o644],
        ['/plain/inner/deep.txt', 0o644],
        ['/display/keep.txt', 0o644],
        ['/display/display-only.bin', 0o644],
    ]),
    links: new Map([
        ['/folder/rel-link', '../targets/rel-target.bin'],
        ['/short/abs-link', '/targets/abs-target.bin'],
    ]),
}

const DISPLAY_ONLY = 'remote filename was decoded from invalid UTF-8 and is display-only'

remote.dirs = new Map([
    ['/folder', [
        { name: 'a-before.txt', isDirectory: false, isSymlink: false, mode: 0o644 },
        { name: 'rel-link', isDirectory: false, isSymlink: true, mode: 0o120777, linkTarget: '../targets/rel-target.bin' },
        { name: 'z-after.txt', isDirectory: false, isSymlink: false, mode: 0o644 },
        { name: 'sub', isDirectory: true, isSymlink: false, mode: 0o755 },
    ]],
    ['/folder/sub', [
        { name: 'b-before.txt', isDirectory: false, isSymlink: false, mode: 0o600 },
        { name: 'cc.txt', isDirectory: false, isSymlink: false, mode: 0o644 },
    ]],
    ['/short', [
        { name: 'head.txt', isDirectory: false, isSymlink: false, mode: 0o644 },
        { name: 'abs-link', isDirectory: false, isSymlink: true, mode: 0o120777, linkTarget: '/targets/abs-target.bin' },
        { name: 'tail.txt', isDirectory: false, isSymlink: false, mode: 0o644 },
    ]],
    ['/plain', [
        { name: 'one.txt', isDirectory: false, isSymlink: false, mode: 0o644 },
        { name: 'two.bin', isDirectory: false, isSymlink: false, mode: 0o644 },
        { name: 'inner', isDirectory: true, isSymlink: false, mode: 0o755 },
    ]],
    ['/plain/inner', [
        { name: 'deep.txt', isDirectory: false, isSymlink: false, mode: 0o644 },
    ]],
    ['/display', [
        { name: 'keep.txt', isDirectory: false, isSymlink: false, mode: 0o644 },
        { name: 'display-only.bin', isDirectory: false, isSymlink: false, mode: 0o644, isOperable: false, unoperableReason: DISPLAY_ONLY },
    ]],
])

function resolveRemote (p) {
    let current = p
    for (let index = 0; index < 40; index++) {
        if (!remote.links.has(current)) {
            return current
        }
        const target = remote.links.get(current)
        current = target.startsWith('/') ? target : POSIX.join(POSIX.dirname(current), target)
    }
    throw new Error(`remote link chain too deep: ${p}`)
}

function remoteContent (p) {
    return remote.files.get(resolveRemote(p)) ?? Buffer.alloc(0)
}

function remoteEntry (dir, name) {
    const fullPath = POSIX.join(dir, name)
    const source = (remote.dirs.get(dir) ?? []).find(item => item.name === name)
    if (!source) {
        return null
    }
    const isOperable = source.isOperable !== false
    return {
        name,
        fullPath,
        isDirectory: source.isDirectory,
        isSymlink: source.isSymlink,
        mode: source.mode,
        size: source.isSymlink ? Buffer.byteLength(source.linkTarget) : (remote.files.get(fullPath) ?? Buffer.alloc(0)).length,
        modified: null,
        isOperable,
        unoperableReason: isOperable ? null : (source.unoperableReason ?? DISPLAY_ONLY),
    }
}

function remoteList (dir) {
    return (remote.dirs.get(dir) ?? []).map(source => remoteEntry(dir, source.name))
}

function remoteStat (p, follow) {
    if (remote.links.has(p)) {
        if (!follow) {
            return {
                name: POSIX.basename(p), fullPath: p, isDirectory: false, isSymlink: true,
                mode: 0o120777, size: Buffer.byteLength(remote.links.get(p)), modified: null,
                isOperable: true, unoperableReason: null,
            }
        }
        return remoteStatResolved(resolveRemote(p))
    }
    return remoteStatResolved(p)
}

function remoteStatResolved (p) {
    if (remote.files.has(p)) {
        return {
            name: POSIX.basename(p), fullPath: p, isDirectory: false, isSymlink: false,
            mode: remote.modes.get(p) ?? 0o644, size: remote.files.get(p).length, modified: null,
            isOperable: true, unoperableReason: null,
        }
    }
    if (remote.dirs.has(p)) {
        return {
            name: POSIX.basename(p), fullPath: p, isDirectory: true, isSymlink: false,
            mode: 0o755, size: 0, modified: null, isOperable: true, unoperableReason: null,
        }
    }
    throw new Error(`no such remote path: ${p}`)
}

function fakeRusshSftp () {
    const closed = new Subject()
    return {
        closed$: closed,
        async readDirectory (p) {
            return remoteList(p).map(entry => ({
                name: entry.name,
                metadata: {
                    type: entry.isSymlink ? russh.SFTPFileType.Symlink : entry.isDirectory ? russh.SFTPFileType.Directory : russh.SFTPFileType.File,
                    permissions: entry.mode,
                    size: entry.size,
                    mtime: 0,
                },
            }))
        },
        async readlink (p) {
            if (!remote.links.has(p)) {
                throw new Error(`not a symlink: ${p}`)
            }
            return remote.links.get(p)
        },
        async stat (p) {
            const metadata = remoteStat(p, true)
            return {
                type: metadata.isDirectory ? russh.SFTPFileType.Directory : russh.SFTPFileType.File,
                permissions: metadata.mode,
                size: metadata.size,
                mtime: 0,
            }
        },
        async open (p) {
            const content = remoteContent(p)
            let position = 0
            return {
                async read (maxBytes) {
                    const chunk = content.subarray(position, position + maxBytes)
                    position += chunk.length
                    return chunk
                },
                async writeAll () {},
                async shutdown () {},
            }
        },
        async removeDirectory () {},
        async createDirectory () {},
        async rename () {},
        async removeFile () {},
        async chmod () {},
    }
}

function resolveInsideBase (base, relative) {
    const resolvedBase = path.resolve(base)
    const target = path.resolve(resolvedBase, relative)
    const rel = path.relative(resolvedBase, target)
    if (rel !== '' && (rel === '..' || rel.startsWith('..' + path.sep) || path.isAbsolute(rel))) {
        throw new Error(`Refusing access outside the target directory: ${relative}`)
    }
    return target
}

function referenceHarness (base) {
    const calls = { errors: [] }
    const logger = { debug () {}, info () {}, warn () {}, error () {} }
    const session = new reference.SFTPSession(fakeRusshSftp(), { get: () => ({ create: () => logger }) })
    const platform = new reference.ReferencePlatform()
    platform.electron = {
        powerSaveBlocker: { start: () => 1, stop: () => {} },
        dialog: {
            showSaveDialog: async () => ({ filePath: path.join(base, 'single-ref', 'file') }),
            showOpenDialog: async () => ({ canceled: true, filePaths: [] }),
        },
    }
    platform.hostWindow = { getWindow: () => null }
    platform.zone = { run: fn => fn() }
    platform.translate = { instant: (text, params) => params?.name === undefined ? text : text.replace('{name}', params.name) }
    platform.fileTransferStarted = { next: () => {} }
    platform.pickDirectory = async () => base
    const panel = new reference.ReferencePanel()
    panel.platform = platform
    panel.notifications = { error: message => calls.errors.push(String(message)) }
    panel.sftp = session
    return { panel, calls, base }
}

function nativeInvoke ({ base, calls }) {
    const sessions = new Map()
    const sftpTransfers = new Map()
    let nextId = 1
    return async (command, payload) => {
        const request = payload?.request ?? {}
        calls.bridgeCalls.push(command)
        switch (command) {
            case 'dialog_open':
                return [base]
            case 'dialog_save':
                return path.join(base, 'single', String(request.fileName ?? 'download'))
            case 'sftp_open':
                return { id: 'sftp-1' }
            case 'sftp_close':
                return null
            case 'sftp_list':
                return remoteList(request.path)
            case 'sftp_stat':
                return remoteStat(request.path, !!request.follow)
            case 'sftp_readlink':
                return remote.links.get(request.path) ?? (() => { throw new Error(`not a symlink: ${request.path}`) })()
            case 'sftp_download_open': {
                const id = `sftp-transfer-${nextId++}`
                sftpTransfers.set(id, { path: request.path, position: 0 })
                return { id, direction: 'download', name: request.path, size: null, transferred: 0, state: 'running' }
            }
            case 'sftp_read': {
                const transfer = sftpTransfers.get(request.transferId)
                assert.ok(transfer, `sftp transfer ${request.transferId} must exist`)
                const content = remoteContent(transfer.path)
                const chunk = content.subarray(transfer.position, transfer.position + request.maxBytes)
                transfer.position += chunk.length
                return Array.from(chunk)
            }
            case 'sftp_close_transfer': {
                const transfer = sftpTransfers.get(request.transferId)
                sftpTransfers.delete(request.transferId)
                return { id: request.transferId, direction: 'download', name: '', size: null, transferred: transfer?.position ?? 0, state: 'completed' }
            }
            case 'sftp_cancel_transfer':
                sftpTransfers.delete(request.transferId)
                return {}
            case 'transfer_create_directory': {
                const destination = resolveInsideBase(base, request.relativePath)
                fs.mkdirSync(destination, { recursive: true })
                calls.createdDirectories.push(request.relativePath)
                return null
            }
            case 'transfer_open_download': {
                const destination = request.baseDirectory && request.relativePath
                    ? resolveInsideBase(base, request.relativePath)
                    : request.destination
                assert.ok(destination, 'download destination must be supplied')
                fs.mkdirSync(path.dirname(destination), { recursive: true })
                const id = `transfer-${nextId++}`
                const stage = `${destination}.tabby-stage-${id}`
                fs.writeFileSync(stage, Buffer.alloc(0))
                sessions.set(id, { destination, stage, size: request.size ?? null, mode: request.mode, transferred: 0 })
                calls.opened.push({ id, name: request.name, mode: request.mode, size: request.size ?? null, relativePath: request.relativePath ?? null })
                return { id, direction: 'download', name: request.name, size: request.size ?? null, transferred: 0, state: 'pending' }
            }
            case 'transfer_write': {
                const session = sessions.get(request.id)
                assert.ok(session, `transfer ${request.id} must exist`)
                const bytes = Buffer.from(request.data)
                if (session.size !== null && session.transferred + bytes.length > session.size) {
                    throw new Error('transfer exceeds advertised size')
                }
                fs.appendFileSync(session.stage, bytes)
                session.transferred += bytes.length
                return null
            }
            case 'transfer_close': {
                const session = sessions.get(request.id)
                assert.ok(session, `transfer ${request.id} must exist`)
                sessions.delete(request.id)
                if (session.size !== null && session.transferred !== session.size) {
                    fs.rmSync(session.stage, { force: true })
                    throw new Error('download size does not match the advertised size')
                }
                fs.renameSync(session.stage, session.destination)
                return null
            }
            case 'transfer_cancel': {
                const session = sessions.get(request.id)
                sessions.delete(request.id)
                if (session) {
                    fs.rmSync(session.stage, { force: true })
                }
                return null
            }
            default:
                throw new Error(`unexpected bridge command: ${command}`)
        }
    }
}

async function nativeHarness () {
    const base = fs.mkdtempSync(path.join(TMP_ROOT, 'native-'))
    const calls = { errors: [], opened: [], createdDirectories: [], bridgeCalls: [] }
    nativeBridge = nativeInvoke({ base, calls })
    const bridge = new native.TauriHostBridge()
    const platform = new native.NativePlatform()
    platform.bridge = bridge
    platform.fileTransferStarted = { next: () => {} }
    const panel = new native.NativePanel()
    panel.platform = platform
    panel.notifications = { error: message => calls.errors.push(String(message)) }
    panel.sftp = await native.TauriSftpSession.open(bridge, 'ssh-1')
    return { panel, platform, bridge, calls, base }
}

const TMP_ROOT = fs.mkdtempSync(path.join(os.tmpdir(), 'sftp-directory-links-'))

const folder = { name: 'folder', fullPath: '/folder', isDirectory: true, isSymlink: false, mode: 0o755, size: 0, isOperable: true }
const short = { name: 'short', fullPath: '/short', isDirectory: true, isSymlink: false, mode: 0o755, size: 0, isOperable: true }
const plain = { name: 'plain', fullPath: '/plain', isDirectory: true, isSymlink: false, mode: 0o755, size: 0, isOperable: true }
const display = { name: 'display', fullPath: '/display', isDirectory: true, isSymlink: false, mode: 0o755, size: 0, isOperable: true }

function read (base, ...segments) {
    return fs.readFileSync(path.join(base, ...segments))
}

function modeOf (target) {
    return fs.statSync(target).mode & 0o7777
}

function controlMode (dir, name, mode) {
    const control = path.join(dir, name)
    fs.closeSync(fs.openSync(control, 'w', mode))
    return control
}

const failures = []
let checks = 0

async function check (name, run) {
    checks++
    try {
        await run()
        console.log(`${name}: PASS`)
    } catch (error) {
        failures.push(name)
        console.error(`${name}: FAIL: ${error.message}`)
    }
}

const LINK_DOWNLOAD_CASES = new Set([
    'native folder download writes the relative file link payload between plain siblings',
    'native folder download writes the absolute file link payload between plain siblings',
])

try {
    await check('reference folder download writes the relative file link payload between plain siblings', async () => {
        const base = fs.mkdtempSync(path.join(TMP_ROOT, 'ref-rel-'))
        const { panel, calls } = referenceHarness(base)
        await panel.downloadItem(folder)
        assert.deepEqual(calls.errors, [])
        assert.deepEqual(read(base, 'folder', 'a-before.txt'), Buffer.from('ALPHA'))
        assert.deepEqual(read(base, 'folder', 'rel-link'), REL_TARGET)
        assert.deepEqual(read(base, 'folder', 'z-after.txt'), Buffer.from('OMEGA'))
        assert.deepEqual(read(base, 'folder', 'sub', 'b-before.txt'), Buffer.from('BEE'))
        assert.deepEqual(read(base, 'folder', 'sub', 'cc.txt'), Buffer.from('CC'))
        const control = controlMode(base, 'control-rel', 0o120777)
        assert.equal(modeOf(path.join(base, 'folder', 'rel-link')), modeOf(control))
    })

    await check('reference folder download writes the absolute file link payload between plain siblings', async () => {
        const base = fs.mkdtempSync(path.join(TMP_ROOT, 'ref-abs-'))
        const { panel, calls } = referenceHarness(base)
        await panel.downloadItem(short)
        assert.deepEqual(calls.errors, [])
        assert.deepEqual(read(base, 'short', 'head.txt'), Buffer.from('HEAD'))
        assert.deepEqual(read(base, 'short', 'abs-link'), ABS_TARGET)
        assert.deepEqual(read(base, 'short', 'tail.txt'), Buffer.from('TAIL'))
        const control = controlMode(base, 'control-abs', 0o120777)
        assert.equal(modeOf(path.join(base, 'short', 'abs-link')), modeOf(control))
    })

    await check('reference folder download keeps plain files and nested directories exact', async () => {
        const base = fs.mkdtempSync(path.join(TMP_ROOT, 'ref-plain-'))
        const { panel, calls } = referenceHarness(base)
        await panel.downloadItem(plain)
        assert.deepEqual(calls.errors, [])
        assert.deepEqual(read(base, 'plain', 'one.txt'), Buffer.from('ONE'))
        assert.deepEqual(read(base, 'plain', 'two.bin'), Buffer.from([0, 255, 1, 254, 2, 253]))
        assert.deepEqual(read(base, 'plain', 'inner', 'deep.txt'), Buffer.from('DEEP'))
        assert.equal(modeOf(path.join(base, 'plain', 'one.txt')), modeOf(controlMode(base, 'control-plain', 0o644)))
    })

    await check('native folder download keeps plain files and nested directories exact', async () => {
        const { panel, calls, base } = await nativeHarness()
        await panel.download(plain)
        assert.deepEqual(calls.errors, [])
        assert.deepEqual(read(base, 'plain', 'one.txt'), Buffer.from('ONE'))
        assert.deepEqual(read(base, 'plain', 'two.bin'), Buffer.from([0, 255, 1, 254, 2, 253]))
        assert.deepEqual(read(base, 'plain', 'inner', 'deep.txt'), Buffer.from('DEEP'))
        assert.deepEqual(calls.createdDirectories, ['plain/inner'])
        assert.deepEqual(calls.opened.map(item => item.relativePath).sort(), ['plain/inner/deep.txt', 'plain/one.txt', 'plain/two.bin'])
    })

    await check('native folder download keeps display-only names out of transfers', async () => {
        const { panel, calls, base } = await nativeHarness()
        await panel.download(display)
        assert.deepEqual(calls.errors, [])
        assert.deepEqual(read(base, 'display', 'keep.txt'), Buffer.from('KEEP'))
        assert.deepEqual(calls.opened.map(item => item.relativePath), ['display/keep.txt'])
    })

    await check('native top-level plain file download keeps its name and exact bytes', async () => {
        const { panel, calls, base } = await nativeHarness()
        const entry = remoteEntry('/plain', 'one.txt')
        await panel.download(entry)
        assert.deepEqual(calls.errors, [])
        assert.deepEqual(read(base, 'single', 'one.txt'), Buffer.from('ONE'))
        const opened = calls.opened.find(item => item.name === 'one.txt')
        assert.ok(opened, `one.txt must be opened, got ${JSON.stringify(calls.opened)}`)
        assert.equal(opened.size, 3)
        assert.equal(opened.mode, 0o644)
    })

    await check('simulated native IPC enforces the advertised size during renderer writes', async () => {
        const { platform, base } = await nativeHarness()
        const transfer = await platform.startDownload('guard.bin', 0o644, 5)
        await assert.rejects(transfer.write(new Uint8Array(10)), /transfer exceeds advertised size/)
        transfer.cancel()
        assert.ok(!fs.existsSync(path.join(base, 'single', 'guard.bin')), 'destination must not be persisted')
    })

    await check('simulated native IPC rejects incomplete renderer completion', async () => {
        const { platform, base } = await nativeHarness()
        const transfer = await platform.startDownload('short.bin', 0o644, 5)
        await transfer.write(new Uint8Array(3))
        await assert.rejects(transfer.closeAsync(), /download size does not match the advertised size/)
        assert.ok(!fs.existsSync(path.join(base, 'single', 'short.bin')), 'destination must not be persisted')
    })

    await check('native folder download writes the relative file link payload between plain siblings', async () => {
        const { panel, calls, base } = await nativeHarness()
        await panel.download(folder)
        if (calls.errors.length) {
            throw new Error(`native folder download reported: ${calls.errors.join(' | ')}`)
        }
        assert.deepEqual(read(base, 'folder', 'a-before.txt'), Buffer.from('ALPHA'))
        assert.deepEqual(read(base, 'folder', 'rel-link'), REL_TARGET)
        assert.deepEqual(read(base, 'folder', 'z-after.txt'), Buffer.from('OMEGA'))
        assert.deepEqual(read(base, 'folder', 'sub', 'b-before.txt'), Buffer.from('BEE'))
        assert.deepEqual(read(base, 'folder', 'sub', 'cc.txt'), Buffer.from('CC'))
        const opened = calls.opened.find(item => item.relativePath === 'folder/rel-link')
        assert.ok(opened, `rel-link must be opened, got ${JSON.stringify(calls.opened)}`)
        assert.equal(opened.size, REL_TARGET.length)
        assert.equal(opened.mode, 0o120777)
    })

    await check('native folder download writes the absolute file link payload between plain siblings', async () => {
        const { panel, calls, base } = await nativeHarness()
        await panel.download(short)
        if (calls.errors.length) {
            throw new Error(`native folder download reported: ${calls.errors.join(' | ')}`)
        }
        assert.deepEqual(read(base, 'short', 'head.txt'), Buffer.from('HEAD'))
        assert.deepEqual(read(base, 'short', 'abs-link'), ABS_TARGET)
        assert.deepEqual(read(base, 'short', 'tail.txt'), Buffer.from('TAIL'))
        const opened = calls.opened.find(item => item.relativePath === 'short/abs-link')
        assert.ok(opened, `abs-link must be opened, got ${JSON.stringify(calls.opened)}`)
        assert.equal(opened.size, ABS_TARGET.length)
        assert.equal(opened.mode, 0o120777)
    })
} finally {
    fs.rmSync(TMP_ROOT, { recursive: true, force: true })
}

const preservedFailures = failures.filter(name => !LINK_DOWNLOAD_CASES.has(name))
assert.equal(preservedFailures.length, 0, `reference/preserved SFTP directory link cases failed: ${preservedFailures.join('; ')}`)

console.log(`SFTP directory link regression: ${checks - failures.length} passed; ${failures.length} failed.`)
process.exitCode = failures.length > 0 ? 1 : 0
