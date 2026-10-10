import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const panelSource = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/sftpPanel.component.ts'), 'utf8')
const sessionSource = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/sftp.ts'), 'utf8')
const bridgeSource = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/tauriHostBridge.service.ts'), 'utf8')
const referenceSource = execFileSync('git', ['show', '14e2d60:tabby-ssh/src/components/sftpPanel.component.ts'], { cwd: root, encoding: 'utf8' })

function memberText (source, className, memberName, treeName = className) {
    const tree = ts.createSourceFile(`${treeName}.ts`, source, ts.ScriptTarget.Latest, true)
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

function classMembersText (source, className) {
    const tree = ts.createSourceFile(`${className}.ts`, source, ts.ScriptTarget.Latest, true)
    const cls = tree.statements.find(node => ts.isClassDeclaration(node) && node.name?.text === className)
    assert.ok(cls, `class ${className} must exist`)
    return cls.members
        .map(member => ts.createPrinter().printNode(ts.EmitHint.Unspecified, member, tree))
        .join('\n')
}

function functionText (source, name) {
    const tree = ts.createSourceFile('fn.ts', source, ts.ScriptTarget.Latest, true)
    const fn = tree.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === name)
    assert.ok(fn, `function ${name} must exist`)
    return ts.createPrinter().printNode(ts.EmitHint.Unspecified, fn, tree)
}

function transpile (name, program, globals = {}) {
    const result = { exports: {} }
    const code = ts.transpileModule(program, {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS },
    }).outputText
    vm.runInNewContext(code, { module: result, ...globals }, { filename: `${name}.js` })
    return result.exports
}

function instantiate (name, body, globals = {}) {
    const exports = transpile(name, `class ${name} { ${body} }; module.exports = ${name}`, globals)
    return new exports()
}

function nativeHost (invoke) {
    const program = [
        functionText(bridgeSource, 'toRustCommand'),
        `class TauriHostBridge { ${memberText(bridgeSource, 'TauriHostBridge', 'api')} ${memberText(bridgeSource, 'TauriHostBridge', 'invoke')} }`,
        `class TauriSftpSession { ${classMembersText(sessionSource, 'TauriSftpSession')} }`,
        'module.exports = { TauriHostBridge, TauriSftpSession }',
    ].join('\n')
    const exports = transpile('host', program, { window: { __TAURI__: { core: { invoke } } }, Error })
    return { TauriHostBridge: exports.TauriHostBridge, TauriSftpSession: exports.TauriSftpSession }
}

const file = (name, fullPath, over = {}) => ({ name, fullPath, isDirectory: false, isSymlink: false, mode: 0o644, size: 3, isOperable: true, ...over })
const dir = (name, fullPath, over = {}) => ({ name, fullPath, isDirectory: true, isSymlink: false, mode: 0o755, size: 0, isOperable: true, ...over })
const link = (name, fullPath, over = {}) => ({ name, fullPath, isDirectory: false, isSymlink: true, mode: 0o120777, size: 1, isOperable: true, ...over })

const RAW_TARGET = {
    '/data/link-rel.txt': 'file.txt',
    '/data/link-abs.txt': '/data/file.txt',
    '/data/link-dir-rel': 'dir',
    '/data/link-dir-abs': '/data/dir',
    '/data/alias/sub-link': 'inner.txt',
    '/data/dangling': 'missing.txt',
}

const STATS = {
    '/data/file.txt': { isDirectory: false, isSymlink: false, mode: 0o644, size: 9 },
    '/data/dir': { isDirectory: true, isSymlink: false, mode: 0o755, size: 0 },
    '/data/alias/inner.txt': { isDirectory: false, isSymlink: false, mode: 0o640, size: 12 },
    '/data/alias/sub-link': { isDirectory: false, isSymlink: false, mode: 0o777, size: 999 },
    '/data/folder/note-link': { isDirectory: false, isSymlink: false, mode: 0o644, size: 9 },
}

const REMOTE_DIRS = {
    '/data/folder': [
        file('inner.txt', '/data/folder/inner.txt', { size: 4 }),
        link('note-link', '/data/folder/note-link', { size: 9 }),
        dir('subdir', '/data/folder/subdir'),
        file('display-only', '/data/folder/display-only', { isOperable: false, unoperableReason: 'remote filename was decoded from invalid UTF-8 and is display-only' }),
    ],
    '/data/folder/subdir': [file('deep.txt', '/data/folder/subdir/deep.txt', { size: 5 })],
    '/data/link-dir-rel': [file('nested.txt', '/data/link-dir-rel/nested.txt', { size: 6 })],
    '/data/link-dir-abs': [file('nested.txt', '/data/link-dir-abs/nested.txt', { size: 6 })],
}

const REL_FILE_LINK = link('link-rel.txt', '/data/link-rel.txt')
const ABS_FILE_LINK = link('link-abs.txt', '/data/link-abs.txt')
const REL_DIR_LINK = { ...link('link-dir-rel', '/data/link-dir-rel'), size: 3 }
const ABS_DIR_LINK = { ...link('link-dir-abs', '/data/link-dir-abs'), size: 3 }
const ALIAS_PARENT = link('sub-link', '/data/alias/sub-link', { size: 7 })
const DANGLING = link('dangling', '/data/dangling', { size: 8 })
const PLAIN_FILE = file('file.txt', '/data/file.txt', { size: 9 })
const PLAIN_DIR = dir('dir', '/data/dir')
const FOLDER = dir('folder', '/data/folder')

function lookupRaw (calls, p) {
    const target = RAW_TARGET[p]
    if (target === undefined) {
        throw new Error(`no such remote link: ${p}`)
    }
    calls.readlinks.push(p)
    return target
}

function lookupStat (calls, p) {
    const stats = STATS[p]
    if (stats === undefined) {
        throw new Error(`no such remote file: ${p}`)
    }
    calls.stats.push({ path: p })
    return { ...stats }
}

function sftpStub (calls) {
    return {
        readlink: async p => { calls.order.push(`readlink:${p}`); return lookupRaw(calls, p) },
        stat: async p => { calls.order.push(`stat:${p}`); return lookupStat(calls, p) },
        readdir: async p => { calls.order.push(`readdir:${p}`); calls.readdirs.push(p); return REMOTE_DIRS[p] ?? [] },
        download: async p => { calls.order.push(`download:${p}`); calls.remoteDownloads.push(p) },
    }
}

function makeTransfer (calls, kind = 'file') {
    return {
        write: async () => {},
        close: async () => {},
        closeAsync: async () => {},
        cancel: () => { calls.cancels.push(kind) },
        setStatus: () => {},
        setTotalSize: () => {},
        setCompleted: () => {},
        isCancelled: () => false,
        createDirectory: async n => { calls.createdDirectories.push(n) },
        createFile: async (name, mode, size) => {
            calls.createdFiles.push({ name, mode, size })
            return { write: async () => {}, closeAsync: async () => {} }
        },
    }
}

function newCalls () {
    return {
        order: [], navigations: [], readlinks: [], stats: [], followedStats: [], readdirs: [], remoteDownloads: [],
        downloads: [], directoryDownloads: [], createdDirectories: [], createdFiles: [], cancels: [], errors: [],
    }
}

function referencePanel (pathValue, overrides = {}) {
    const calls = newCalls()
    const body = [
        memberText(referenceSource, 'SFTPPanelComponent', 'open'),
        memberText(referenceSource, 'SFTPPanelComponent', 'downloadItem'),
        memberText(referenceSource, 'SFTPPanelComponent', 'download'),
        memberText(referenceSource, 'SFTPPanelComponent', 'downloadFolder'),
        memberText(referenceSource, 'SFTPPanelComponent', 'downloadFolderRecursive'),
        memberText(referenceSource, 'SFTPPanelComponent', 'calculateFolderSizeAndUpdate'),
    ].join('\n')
    const panel = instantiate('ReferencePanel', body, { path: path.posix, Error })
    panel.path = pathValue
    panel.navigate = async p => { calls.navigations.push(p) }
    panel.notifications = { error: message => { calls.errors.push(String(message)) } }
    panel.sftp = sftpStub(calls)
    panel.platform = {
        startDownload: async (name, mode, size) => {
            calls.downloads.push({ name, mode, size })
            return overrides.allowFileDownload === false ? null : makeTransfer(calls)
        },
        startDownloadDirectory: async (name, mode) => {
            calls.directoryDownloads.push({ name, mode })
            return overrides.allowDirectoryDownload === false ? null : makeTransfer(calls, 'directory')
        },
    }
    return { panel, calls }
}

function nativePanel (overrides = {}) {
    const calls = newCalls()
    const body = [
        memberText(panelSource, 'TauriSftpPanelComponent', 'open'),
        memberText(panelSource, 'TauriSftpPanelComponent', 'download'),
        memberText(panelSource, 'TauriSftpPanelComponent', 'downloadDirectory'),
    ].join('\n')
    const panel = instantiate('NativePanel', body, { Error, posixPath: path.posix })
    panel.path = '/data'
    panel.showError = error => { calls.errors.push(error instanceof Error ? error.message : String(error)) }
    panel.navigate = async p => { calls.navigations.push(p) }
    const sftp = sftpStub(calls)
    panel.sftp = { ...sftp, stat: async (p, follow) => { calls.followedStats.push({ path: p, follow }); return sftp.stat(p) } }
    panel.platform = {
        startDownload: async (name, mode, size) => {
            calls.downloads.push({ name, mode, size })
            return overrides.allowFileDownload === false ? null : makeTransfer(calls)
        },
        startDownloadDirectory: async (name, mode) => {
            calls.directoryDownloads.push({ name, mode })
            return overrides.allowDirectoryDownload === false ? null : makeTransfer(calls, 'directory')
        },
    }
    return { panel, calls }
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

const LINK_RESOLUTION_CASES = new Set([
    'native cancelled file selection leaves no remote download',
    'native cancelled directory selection leaves no remote listing',
    'native rejects a dangling link without transfer or navigation',
    'native rejects a link whose readlink would fail without transfer or navigation',
    'native open preserves the alias download for a relative file link',
    'native open preserves the alias download for an absolute file link',
    'native open navigates the alias for a directory link',
    'native explicit download preserves the alias name/path with followed size/mode for a file link',
    'native explicit download lists the alias folder for a directory link',
    'native resolves the target against the panel path before stat, not by following the alias',
    'native folder download does not silently skip a file link',
])

await check('reference open preserves the alias download for a relative file link', async () => {
    const { panel, calls } = referencePanel('/data')
    await panel.open(REL_FILE_LINK)
    assert.deepEqual(calls.order, ['readlink:/data/link-rel.txt', 'stat:/data/file.txt', 'download:/data/link-rel.txt'])
    assert.deepEqual(calls.downloads, [{ name: 'link-rel.txt', mode: 0o644, size: 9 }])
    assert.deepEqual(calls.navigations, [])
})

await check('reference open preserves the alias download for an absolute file link', async () => {
    const { panel, calls } = referencePanel('/data')
    await panel.open(ABS_FILE_LINK)
    assert.deepEqual(calls.order, ['readlink:/data/link-abs.txt', 'stat:/data/file.txt', 'download:/data/link-abs.txt'])
    assert.deepEqual(calls.downloads, [{ name: 'link-abs.txt', mode: 0o644, size: 9 }])
})

await check('reference downloadItem preserves the alias name/path with followed size/mode for a file link', async () => {
    for (const item of [REL_FILE_LINK, ABS_FILE_LINK]) {
        const { panel, calls } = referencePanel('/data')
        await panel.downloadItem(item)
        assert.deepEqual(calls.order, [`readlink:${item.fullPath}`, 'stat:/data/file.txt', `download:${item.fullPath}`])
        assert.deepEqual(calls.downloads, [{ name: item.name, mode: 0o644, size: 9 }])
    }
})

await check('reference open navigates the alias for a directory link', async () => {
    for (const item of [REL_DIR_LINK, ABS_DIR_LINK]) {
        const { panel, calls } = referencePanel('/data')
        await panel.open(item)
        assert.deepEqual(calls.order, [`readlink:${item.fullPath}`, 'stat:/data/dir'])
        assert.deepEqual(calls.navigations, [item.fullPath])
        assert.deepEqual(calls.remoteDownloads, [])
    }
})

await check('reference downloadItem lists the alias folder for a directory link, nested plain files download', async () => {
    for (const item of [REL_DIR_LINK, ABS_DIR_LINK]) {
        const { panel, calls } = referencePanel('/data')
        await panel.downloadItem(item)
        assert.ok(calls.readdirs.length > 0 && calls.readdirs.every(p => p === item.fullPath))
        assert.deepEqual(calls.remoteDownloads, [`${item.fullPath}/nested.txt`])
    }
})

await check('reference resolves the target against the panel path before stat, not by following the alias', async () => {
    const { panel, calls } = referencePanel('/data/alias')
    await panel.open(ALIAS_PARENT)
    assert.deepEqual(calls.order, ['readlink:/data/alias/sub-link', 'stat:/data/alias/inner.txt', 'download:/data/alias/sub-link'])
    assert.deepEqual(calls.downloads, [{ name: 'sub-link', mode: 0o640, size: 12 }])
})

await check('reference folder download includes a file link rather than skipping it', async () => {
    const { panel, calls } = referencePanel('/data')
    await panel.downloadItem(FOLDER)
    assert.ok(calls.remoteDownloads.includes('/data/folder/note-link'), `file link must be downloaded, got ${JSON.stringify(calls.remoteDownloads)}`)
    assert.ok(calls.remoteDownloads.includes('/data/folder/inner.txt'))
    assert.ok(calls.remoteDownloads.includes('/data/folder/subdir/deep.txt'))
})

await check('reference rejects a dangling link without transfer or navigation', async () => {
    const { panel, calls } = referencePanel('/data')
    await assert.rejects(panel.open(DANGLING), /no such remote file:/)
    assert.deepEqual(calls.navigations, [])
    assert.deepEqual(calls.remoteDownloads, [])
    assert.deepEqual(calls.downloads, [])
})

await check('reference rejects a readlink failure without transfer or navigation', async () => {
    const { panel, calls } = referencePanel('/data')
    await assert.rejects(panel.open(link('unknown', '/data/unknown')), /no such remote link:/)
    assert.deepEqual(calls.navigations, [])
    assert.deepEqual(calls.remoteDownloads, [])
    assert.deepEqual(calls.downloads, [])
})

await check('reference cancelled file selection leaves no remote download', async () => {
    for (const item of [PLAIN_FILE, REL_FILE_LINK, ABS_FILE_LINK]) {
        const { panel, calls } = referencePanel('/data', { allowFileDownload: false })
        await panel.open(item)
        assert.deepEqual(calls.downloads, [{ name: item.name, mode: 0o644, size: 9 }])
        assert.deepEqual(calls.remoteDownloads, [])
        assert.deepEqual(calls.readdirs, [])
    }
})

await check('reference cancelled directory selection leaves no remote listing', async () => {
    for (const item of [FOLDER, REL_DIR_LINK, ABS_DIR_LINK]) {
        const { panel, calls } = referencePanel('/data', { allowDirectoryDownload: false })
        await panel.downloadItem(item)
        assert.deepEqual(calls.directoryDownloads, [{ name: item.name, mode: 0 }])
        assert.deepEqual(calls.readdirs, [])
        assert.deepEqual(calls.remoteDownloads, [])
    }
})

await check('reference plain file downloads with its own name/mode/size', async () => {
    const { panel, calls } = referencePanel('/data')
    await panel.open(PLAIN_FILE)
    assert.deepEqual(calls.downloads, [{ name: 'file.txt', mode: 0o644, size: 9 }])
    assert.deepEqual(calls.remoteDownloads, ['/data/file.txt'])
})

await check('reference plain directory open navigates its path', async () => {
    const { panel, calls } = referencePanel('/data')
    await panel.open(PLAIN_DIR)
    assert.deepEqual(calls.navigations, ['/data/dir'])
    assert.deepEqual(calls.readlinks, [])
})

await check('native rejects a dangling link without transfer or navigation', async () => {
    const { panel, calls } = nativePanel()
    await panel.open(DANGLING)
    assert.deepEqual(calls.order, ['readlink:/data/dangling', 'stat:/data/missing.txt'])
    assert.deepEqual(calls.errors, ['no such remote file: /data/missing.txt'])
    assert.deepEqual(calls.navigations, [])
    assert.deepEqual(calls.remoteDownloads, [])
    assert.deepEqual(calls.downloads, [])
})

await check('native rejects a link whose readlink would fail without transfer or navigation', async () => {
    const { panel, calls } = nativePanel()
    await panel.download(link('unknown', '/data/unknown'))
    assert.deepEqual(calls.order, ['readlink:/data/unknown'])
    assert.deepEqual(calls.errors, ['no such remote link: /data/unknown'])
    assert.deepEqual(calls.remoteDownloads, [])
    assert.deepEqual(calls.downloads, [])
})

await check('native keeps display-only remote names out of any operation', async () => {
    const { panel, calls } = nativePanel()
    await panel.open({ ...PLAIN_FILE, name: 'display-only', fullPath: '/data/display-only', isOperable: false, unoperableReason: 'display-only fixture' })
    assert.deepEqual(calls.errors, ['display-only fixture'])
    assert.deepEqual(calls.navigations, [])
    assert.deepEqual(calls.downloads, [])
})

await check('native cancelled file selection leaves no remote download', async () => {
    for (const item of [PLAIN_FILE, REL_FILE_LINK, ABS_FILE_LINK]) {
        const { panel, calls } = nativePanel({ allowFileDownload: false })
        await panel.open(item)
        assert.deepEqual(calls.downloads, [{ name: item.name, mode: 0o644, size: 9 }])
        assert.deepEqual(calls.remoteDownloads, [])
        assert.deepEqual(calls.readdirs, [])
    }
})

await check('native cancelled directory selection leaves no remote listing', async () => {
    for (const item of [FOLDER, REL_DIR_LINK, ABS_DIR_LINK]) {
        const { panel, calls } = nativePanel({ allowDirectoryDownload: false })
        await panel.download(item)
        assert.deepEqual(calls.directoryDownloads, [{ name: item.name, mode: 0 }])
        assert.deepEqual(calls.readdirs, [])
        assert.deepEqual(calls.remoteDownloads, [])
    }
})

await check('native plain file downloads with its own name/mode/size', async () => {
    const { panel, calls } = nativePanel()
    await panel.open(PLAIN_FILE)
    assert.deepEqual(calls.downloads, [{ name: 'file.txt', mode: 0o644, size: 9 }])
    assert.deepEqual(calls.remoteDownloads, ['/data/file.txt'])
})

await check('native plain directory open navigates its path', async () => {
    const { panel, calls } = nativePanel()
    await panel.open(PLAIN_DIR)
    assert.deepEqual(calls.navigations, ['/data/dir'])
    assert.deepEqual(calls.readlinks, [])
})

await check('native folder download retains plain nested files', async () => {
    const { panel, calls } = nativePanel()
    await panel.download(FOLDER)
    assert.ok(calls.remoteDownloads.includes('/data/folder/inner.txt'), `plain files must be downloaded, got ${JSON.stringify(calls.remoteDownloads)}`)
    assert.ok(calls.remoteDownloads.includes('/data/folder/subdir/deep.txt'))
})

await check('native folder download keeps display-only names out of transfers', async () => {
    const { panel, calls } = nativePanel()
    await panel.download(FOLDER)
    assert.ok(!calls.remoteDownloads.some(p => p.includes('display-only')), `display-only entries must be skipped, got ${JSON.stringify(calls.remoteDownloads)}`)
})

await check('TauriSftpSession.readlink maps to sftp_readlink with id/path and returns the raw target', async () => {
    const ipc = []
    const { TauriHostBridge, TauriSftpSession } = nativeHost(async (command, args) => {
        ipc.push({ command, args: args === undefined ? undefined : JSON.parse(JSON.stringify(args)) })
        if (command === 'sftp_open') {
            return { id: 'sftp-1' }
        }
        if (command === 'sftp_readlink') {
            return '../extra/raw target.bin'
        }
        throw new Error(`unexpected command ${command}`)
    })
    const bridge = new TauriHostBridge()
    const session = await TauriSftpSession.open(bridge, 'ssh-1')
    const target = await session.readlink('/data/link')
    assert.equal(target, '../extra/raw target.bin')
    assert.deepEqual(ipc[0], { command: 'sftp_open', args: { request: { id: 'ssh-1' } } })
    assert.deepEqual(ipc[1], { command: 'sftp_readlink', args: { request: { id: 'sftp-1', path: '/data/link' } } })
    assert.equal(ipc.length, 2)
})

await check('the actual toRustCommand keeps dots and camelCase on sftp.downloadOpen', async () => {
    const seen = []
    const { TauriHostBridge } = nativeHost(async (command, args) => {
        seen.push({ command, args: args === undefined ? undefined : JSON.parse(JSON.stringify(args)) })
        return { id: 't1' }
    })
    const bridge = new TauriHostBridge()
    await bridge.invoke('sftp.downloadOpen', { id: 'sftp-1', path: '/data/file.txt' })
    assert.deepEqual(seen, [{ command: 'sftp_download_open', args: { request: { id: 'sftp-1', path: '/data/file.txt' } } }])
})

await check('a closed TauriSftpSession readlink rejects without a further IPC request', async () => {
    const ipc = []
    const { TauriHostBridge, TauriSftpSession } = nativeHost(async (command, args) => {
        ipc.push({ command, args })
        if (command === 'sftp_open') {
            return { id: 'sftp-1' }
        }
        if (command === 'sftp_close') {
            return undefined
        }
        throw new Error(`unexpected command ${command}`)
    })
    const bridge = new TauriHostBridge()
    const session = await TauriSftpSession.open(bridge, 'ssh-1')
    await session.close()
    const before = ipc.length
    await assert.rejects(session.readlink('/data/link'), /SFTP session is closed/)
    await assert.rejects(session.stat('/data/link'), /SFTP session is closed/)
    await assert.rejects(session.readdir('/data'), /SFTP session is closed/)
    assert.equal(ipc.length, before)
})

await check('TauriSftpSession.download streams exact binary bytes and closes the transfer', async () => {
    const payload = [104, 105, 0, 255, 254, 100]
    const ipc = []
    const writes = []
    const flow = []
    const transfer = {
        write: async bytes => { writes.push(Array.from(bytes)) },
        closeAsync: async () => { flow.push('closeAsync') },
        cancel: () => { flow.push('cancel') },
    }
    const { TauriHostBridge, TauriSftpSession } = nativeHost(async (command, args) => {
        ipc.push({ command, args })
        if (command === 'sftp_open') {
            return { id: 'sftp-1' }
        }
        if (command === 'sftp_download_open') {
            return { id: 'transfer-1' }
        }
        if (command === 'sftp_read') {
            return ipc.filter(call => call.command === 'sftp_read').length <= 1 ? payload : []
        }
        if (command === 'sftp_close_transfer') {
            return { bytesTransferred: payload.length }
        }
        throw new Error(`unexpected command ${command}`)
    })
    const bridge = new TauriHostBridge()
    const session = await TauriSftpSession.open(bridge, 'ssh-1')
    const result = await session.download('/data/file.txt', transfer)
    assert.deepEqual(writes, [payload])
    assert.deepEqual(flow, ['closeAsync'])
    assert.equal(result.bytesTransferred, payload.length)
    assert.ok(ipc.some(call => call.command === 'sftp_read'))
    assert.ok(ipc.some(call => call.command === 'sftp_close_transfer'))
})

await check('TauriSftpSession.download failure cancels the transfer and the remote transfer', async () => {
    const ipc = []
    const flow = []
    const transfer = {
        write: async () => { flow.push('write') },
        closeAsync: async () => { flow.push('closeAsync') },
        cancel: () => { flow.push('cancel') },
    }
    const { TauriHostBridge, TauriSftpSession } = nativeHost(async (command, args) => {
        ipc.push({ command, args })
        if (command === 'sftp_open') {
            return { id: 'sftp-1' }
        }
        if (command === 'sftp_download_open') {
            return { id: 'transfer-1' }
        }
        if (command === 'sftp_read') {
            throw new Error('remote read failed')
        }
        if (command === 'sftp_cancel_transfer') {
            return {}
        }
        throw new Error(`unexpected command ${command}`)
    })
    const bridge = new TauriHostBridge()
    const session = await TauriSftpSession.open(bridge, 'ssh-1')
    await assert.rejects(session.download('/data/file.txt', transfer), /remote read failed/)
    assert.deepEqual(flow, ['cancel'])
    assert.ok(ipc.some(call => call.command === 'sftp_cancel_transfer'))
})

await check('native open preserves the alias download for a relative file link', async () => {
    const { panel, calls } = nativePanel()
    await panel.open(REL_FILE_LINK)
    assert.deepEqual(calls.order, ['readlink:/data/link-rel.txt', 'stat:/data/file.txt', 'download:/data/link-rel.txt'])
    assert.deepEqual(calls.downloads, [{ name: 'link-rel.txt', mode: 0o644, size: 9 }])
})

await check('native open preserves the alias download for an absolute file link', async () => {
    const { panel, calls } = nativePanel()
    await panel.open(ABS_FILE_LINK)
    assert.deepEqual(calls.order, ['readlink:/data/link-abs.txt', 'stat:/data/file.txt', 'download:/data/link-abs.txt'])
    assert.deepEqual(calls.downloads, [{ name: 'link-abs.txt', mode: 0o644, size: 9 }])
})

await check('native open navigates the alias for a directory link', async () => {
    for (const item of [REL_DIR_LINK, ABS_DIR_LINK]) {
        const { panel, calls } = nativePanel()
        await panel.open(item)
        assert.deepEqual(calls.order, [`readlink:${item.fullPath}`, 'stat:/data/dir'])
        assert.deepEqual(calls.followedStats, [{ path: '/data/dir', follow: true }])
        assert.deepEqual(calls.navigations, [item.fullPath])
        assert.deepEqual(calls.remoteDownloads, [])
    }
})

await check('native explicit download preserves the alias name/path with followed size/mode for a file link', async () => {
    for (const item of [REL_FILE_LINK, ABS_FILE_LINK]) {
        const { panel, calls } = nativePanel()
        await panel.download(item)
        assert.deepEqual(calls.order, [`readlink:${item.fullPath}`, 'stat:/data/file.txt', `download:${item.fullPath}`])
        assert.deepEqual(calls.followedStats, [{ path: '/data/file.txt', follow: true }])
        assert.deepEqual(calls.downloads, [{ name: item.name, mode: 0o644, size: 9 }])
    }
})

await check('native explicit download lists the alias folder for a directory link', async () => {
    for (const item of [REL_DIR_LINK, ABS_DIR_LINK]) {
        const { panel, calls } = nativePanel()
        await panel.download(item)
        assert.deepEqual(calls.followedStats, [{ path: '/data/dir', follow: true }])
        assert.deepEqual(calls.readdirs, [item.fullPath])
        assert.deepEqual(calls.remoteDownloads, [`${item.fullPath}/nested.txt`])
    }
})

await check('native resolves the target against the panel path before stat, not by following the alias', async () => {
    const { panel, calls } = nativePanel()
    panel.path = '/data/alias'
    await panel.open(ALIAS_PARENT)
    assert.deepEqual(calls.order, ['readlink:/data/alias/sub-link', 'stat:/data/alias/inner.txt', 'download:/data/alias/sub-link'])
    assert.deepEqual(calls.followedStats, [{ path: '/data/alias/inner.txt', follow: true }])
    assert.deepEqual(calls.downloads, [{ name: 'sub-link', mode: 0o640, size: 12 }])
})

await check('native folder download does not silently skip a file link', async () => {
    const { panel, calls } = nativePanel()
    await panel.download(FOLDER)
    assert.ok(calls.remoteDownloads.includes('/data/folder/note-link'), `file link must be downloaded, got ${JSON.stringify(calls.remoteDownloads)}`)
})

for (const [label, item, target] of [
    ['file', REL_FILE_LINK, '/data/file.txt'],
    ['directory', REL_DIR_LINK, '/data/dir'],
]) {
    await check(`native ${label} link resolution uses the listed item parent after path editing or failed navigation`, async () => {
        for (const editedPath of ['/unsubmitted', '/missing']) {
            for (const method of ['open', 'download']) {
                const { panel, calls } = nativePanel()
                panel.path = editedPath
                await panel[method](item)
                assert.deepEqual(calls.errors, [], `${method} must ignore an unrelated editable path`)
                assert.deepEqual(calls.followedStats, [{ path: target, follow: true }])
                if (label === 'file') {
                    assert.deepEqual(calls.downloads, [{ name: item.name, mode: 0o644, size: 9 }])
                    assert.deepEqual(calls.remoteDownloads, [item.fullPath])
                } else if (method === 'open') {
                    assert.deepEqual(calls.navigations, [item.fullPath])
                } else {
                    assert.deepEqual(calls.readdirs, [item.fullPath])
                    assert.deepEqual(calls.remoteDownloads, [`${item.fullPath}/nested.txt`])
                }
            }
        }
    })
}

const preservedFailures = failures.filter(name => !LINK_RESOLUTION_CASES.has(name))
assert.equal(preservedFailures.length, 0, `reference/preserved SFTP link cases failed: ${preservedFailures.join('; ')}`)

console.log(`SFTP link regression: ${checks - failures.length} passed; ${failures.length} failed.`)
process.exitCode = failures.length > 0 ? 1 : 0
