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
        this.translate = { instant: (text, params) => params ? text.replace('{host}', params.host) : `translated:${text}` }
    }
    subscribeUntilDestroyed (source, next) { this.subscriptions.push(source.subscribe(next)) }
    ngOnInit () { this.baseInits++ }
    ngOnDestroy () { this.subscriptions.forEach(subscription => subscription.unsubscribe()) }
    sendInput (value) { this.input.push(value) }
    async reconnect () { this.reconnects++ }
    async canClose () { return true }
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
    '../../../tabby-ssh/src/api/keyboardInteractivePrompt': {},
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

// canClose tests
let promptCalls = 0
let lastPromptOptions = null
let promptResult = { response: 0 }
let promptError = null
tab.platform = {
    showMessageBox: async (options) => {
        promptCalls++
        lastPromptOptions = options
        if (promptError) throw promptError
        return promptResult
    },
}
tab.config = { store: { ssh: { warnOnClose: false } } }
tab.profile = { options: { host: 'test-host' } }

// 1. null session: no prompt, returns true
tab.session = null
assert.equal(await tab.canClose(), true, 'canClose: null session returns true')
assert.equal(promptCalls, 0, 'canClose: null session must not call showMessageBox')

// 2. disconnected session (open: false): no prompt, returns true
tab.session = { open: false }
promptCalls = 0
assert.equal(await tab.canClose(), true, 'canClose: disconnected session returns true')
assert.equal(promptCalls, 0, 'canClose: disconnected session must not call showMessageBox')

// 3. config disabled (warnOnClose false): no prompt, returns true
tab.session = { open: true }
tab.profile.options.warnOnClose = null
tab.config.store.ssh.warnOnClose = false
promptCalls = 0
assert.equal(await tab.canClose(), true, 'canClose: config disabled returns true')
assert.equal(promptCalls, 0, 'canClose: config disabled must not call showMessageBox')

// 4. null profile inherits global true
tab.config.store.ssh.warnOnClose = true
promptResult = { response: 1 }
promptCalls = 0
assert.equal(await tab.canClose(), false, 'canClose: null profile inherits global true')
assert.equal(promptCalls, 1, 'canClose: must call showMessageBox once')
assert.equal(lastPromptOptions.type, 'warning', 'canClose: dialog type must be warning')
assert.equal(lastPromptOptions.message, 'Disconnect from test-host?', 'canClose: message must have translated host')
assert.equal(lastPromptOptions.buttons.length, 2, 'canClose: must have two buttons')
assert.equal(lastPromptOptions.defaultId, 0, 'canClose: defaultId must be 0')
assert.equal(lastPromptOptions.cancelId, 1, 'canClose: cancelId must be 1')

// 5. null profile inherits global false
tab.config.store.ssh.warnOnClose = false
promptCalls = 0
assert.equal(await tab.canClose(), true, 'canClose: null profile inherits global false')
assert.equal(promptCalls, 0, 'canClose: global false must not call showMessageBox')

// 6. undefined profile inherits global true
tab.profile.options.warnOnClose = undefined
tab.config.store.ssh.warnOnClose = true
promptResult = { response: 1 }
promptCalls = 0
assert.equal(await tab.canClose(), false, 'canClose: undefined profile inherits global true')
assert.equal(promptCalls, 1, 'canClose: must call showMessageBox once')

// 7. explicit true overrides global false
tab.profile.options.warnOnClose = true
tab.config.store.ssh.warnOnClose = false
promptResult = { response: 1 }
promptCalls = 0
assert.equal(await tab.canClose(), false, 'canClose: explicit true overrides global false')
assert.equal(promptCalls, 1, 'canClose: must call showMessageBox once')

// 8. explicit false overrides global true
tab.profile.options.warnOnClose = false
tab.config.store.ssh.warnOnClose = true
promptCalls = 0
assert.equal(await tab.canClose(), true, 'canClose: explicit false overrides global true')
assert.equal(promptCalls, 0, 'canClose: explicit false must not call showMessageBox')

// 9. confirm (response 0) allows close
tab.profile.options.warnOnClose = true
tab.config.store.ssh.warnOnClose = true
promptResult = { response: 0 }
promptCalls = 0
assert.equal(await tab.canClose(), true, 'canClose: confirm (response 0) allows close')
assert.equal(promptCalls, 1, 'canClose: must call showMessageBox once')

// 10. cancel (response 1) blocks close
promptResult = { response: 1 }
promptCalls = 0
assert.equal(await tab.canClose(), false, 'canClose: cancel (response 1) blocks close')
assert.equal(promptCalls, 1, 'canClose: must call showMessageBox once')

// 11. unknown response blocks close
promptResult = { response: 2 }
promptCalls = 0
assert.equal(await tab.canClose(), false, 'canClose: unknown response blocks close')
assert.equal(promptCalls, 1, 'canClose: must call showMessageBox once')

// 12. rejected dialog must not silently allow close
promptError = new Error('dialog rejected')
await assert.rejects(tab.canClose(), /dialog rejected/, 'canClose: rejected dialog throws')

console.log('Tauri SSH focus, hotkeys, menu, safe launch errors, and canClose passed')
