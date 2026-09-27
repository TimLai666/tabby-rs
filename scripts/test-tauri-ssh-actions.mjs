import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import ts from 'typescript'
import { Subject } from 'rxjs'

const Platform = { Windows: 'Windows', macOS: 'macOS', Linux: 'Linux' }
class BaseTab {
    static template = ''
    static styles = []
    static animations = []
    constructor () {
        this.hasFocus = false
        this.hotkeys = { hotkey$: new Subject() }
        this.subscriptions = []
        this.input = []
        this.notices = []
        this.reconnects = 0
        this.baseInits = 0
        this.notifications = { error: text => this.notices.push(text) }
        this.translate = { instant: text => `translated:${text}` }
    }
    subscribeUntilDestroyed (source, next) { this.subscriptions.push(source.subscribe(next)) }
    ngOnInit () { this.baseInits++ }
    ngOnDestroy () { this.subscriptions.forEach(subscription => subscription.unsubscribe()) }
    sendInput (value) { this.input.push(value) }
    async reconnect () { this.reconnects++ }
}
class TabMenu {}
class TerminalMenu {}
const core = { BaseTabComponent: BaseTab, TabContextMenuItemProvider: TabMenu, Platform }
const terminal = { BaseTerminalTabComponent: BaseTab, ConnectableTerminalTabComponent: BaseTab, TerminalContextMenuItemProvider: TerminalMenu }
const fakes = {
    '@angular/core': { Component: () => target => target, Injectable: () => target => target },
    'tabby-core': core,
    'tabby-terminal': terminal,
    '../api/hostBridge': {},
    '../services/winscp.service': {},
    './services/winscp.service': {},
    './authPromptModal.component': {},
    './session': { TauriSshSession: class {} },
}
function load (relative) {
    const source = fs.readFileSync(new URL(`../${relative}`, import.meta.url), 'utf8')
    const exports = {}
    vm.runInNewContext(ts.transpileModule(source, {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true },
    }).outputText, {
        exports, setTimeout, clearTimeout,
        require: name => {
            assert.ok(name in fakes, `Unexpected dependency: ${name}`)
            return fakes[name]
        },
    })
    return exports
}
const { TauriSshTabComponent } = load('tabby-tauri/src/ssh/tab.component.ts')
fakes['./ssh/tab.component'] = { TauriSshTabComponent }
const { TauriSftpContextMenu } = load('tabby-tauri/src/sftpContextMenu.ts')
let path = 'WinSCP.exe'
const launched = []
let failure = false
const winscp = {
    getWinSCPPath: () => path,
    launchWinSCP: async session => {
        if (failure) throw new Error('private-password-must-not-appear')
        launched.push(session)
    },
}
const tab = new TauriSshTabComponent({}, {}, {}, {}, winscp)
const session = { open: false }
tab.session = session
let sftp = 0
tab.openSFTP = async () => { sftp++ }
tab.ngOnInit()
assert.equal(tab.baseInits, 1)
const keys = ['home', 'end', 'restart-ssh-session', 'open-sftp', 'launch-winscp']
for (const key of keys) tab.hotkeys.hotkey$.next(key)
assert.deepEqual([tab.input.length, tab.reconnects, sftp, launched.length], [0, 0, 0, 0])
tab.hasFocus = true
for (const key of keys) tab.hotkeys.hotkey$.next(key)
await Promise.resolve()
assert.deepEqual(tab.input, ['\x1bOH', '\x1bOF'])
assert.deepEqual([tab.reconnects, sftp, launched.length], [1, 1, 1])
assert.equal(launched[0], session)
tab.session = null
await tab.launchWinSCP()
assert.equal(launched.length, 1, 'A tab without a session must not launch')
tab.session = session
failure = true
await tab.launchWinSCP()
assert.deepEqual(tab.notices, ['translated:Could not launch WinSCP'])
failure = false

const translate = { instant: text => text }
const menu = new TauriSftpContextMenu({ platform: Platform.Windows }, winscp, translate)
assert.ok(menu instanceof TabMenu)
assert.equal((await menu.getItems(new BaseTab())).length, 0)
const items = await menu.getItems(tab)
assert.deepEqual(Array.from(items, item => item.label), ['Open SFTP panel', 'Launch WinSCP'])
items[0].click()
items[1].click()
await Promise.resolve()
assert.deepEqual([sftp, launched.length], [2, 2])
tab.session = null
assert.equal((await menu.getItems(tab)).length, 2, 'Menu construction must tolerate an uninitialized SSH session')
for (const platform of [Platform.macOS, Platform.Linux]) {
    const other = new TauriSftpContextMenu({ platform }, winscp, translate)
    assert.deepEqual(Array.from(await other.getItems(tab), item => item.label), ['Open SFTP panel'])
}
path = null
assert.deepEqual(Array.from(await menu.getItems(tab), item => item.label), ['Open SFTP panel'])
tab.ngOnDestroy()
for (const key of keys) tab.hotkeys.hotkey$.next(key)
assert.deepEqual([tab.input.length, tab.reconnects, sftp, launched.length], [2, 1, 2, 2])
console.log('Tauri SSH focus, hotkeys, menu, and safe launch errors passed')
