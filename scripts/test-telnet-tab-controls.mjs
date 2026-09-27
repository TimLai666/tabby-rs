import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import { execFileSync } from 'node:child_process'
import ts from 'typescript'

const root = new URL('../', import.meta.url)
const upstream = execFileSync('git', ['show', '14e2d60:tabby-telnet/src/components/telnetTab.component.ts'], {
    cwd: root, encoding: 'utf8',
})
const current = fs.readFileSync(new URL('tabby-tauri/src/telnet/tab.component.ts', root), 'utf8')

function load (source, name) {
    class Base {
        static template = ''
        static styles = []
        static animations = []
        async canClose () { return true }
    }
    const module = { exports: {} }
    const code = ts.transpileModule(source, { compilerOptions: {
        target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS,
        esModuleInterop: true, experimentalDecorators: true,
    } }).outputText
    vm.runInNewContext(code, { module, exports: module.exports, require: dependency => {
        if (dependency === '@angular/core') return { Component: () => target => target }
        if (dependency === '@biesbjerg/ngx-translate-extract-marker') return { marker: text => text }
        if (dependency === 'tabby-terminal') return { BaseTerminalTabComponent: Base, ConnectableTerminalTabComponent: Base }
        if (dependency === 'tabby-core' || dependency === 'ansi-colors' || dependency.endsWith('/session')) return {}
        if (dependency.endsWith('.pug')) return ''
        throw new Error(`Unexpected dependency: ${dependency}`)
    } })
    return module.exports[name]
}

const expectedPrompt = {
    type: 'warning', message: 'translated:Disconnect from 測試.example?',
    buttons: ['translated:Disconnect', 'translated:Do not close'], defaultId: 0, cancelId: 1,
}
async function check (Tab) {
    const tab = new Tab({}, {})
    tab.profile = { options: { host: '測試.example', port: 2323 } }
    tab.translate = { instant: (text, params) => `translated:${text.replace('{host}', params?.host)}` }
    const prompts = []
    let response = 0
    tab.platform = { showMessageBox: async options => {
        prompts.push(JSON.parse(JSON.stringify(options)))
        return { response }
    } }
    const session = { open: true, destroy: () => assert.fail('canClose must not destroy a session') }
    tab.destroy = () => assert.fail('canClose must not destroy the tab')
    for (const inactive of [null, undefined, { open: false }]) {
        tab.session = inactive
        assert.equal(await tab.canClose(), true)
        assert.equal(prompts.length, 0, 'inactive sessions close without prompting')
    }
    tab.session = session
    for (response of [0, 1, -1, 2, undefined]) {
        assert.equal(await tab.canClose(), response === 0, 'only explicit confirmation permits closing')
        assert.deepEqual(prompts.at(-1), expectedPrompt)
        assert.equal(tab.session, session)
        assert.equal(session.open, true)
    }
    const failure = new Error('native dialog unavailable')
    tab.platform.showMessageBox = async () => { throw failure }
    await assert.rejects(tab.canClose(), error => error === failure)
    for (const answer of [0, 1]) {
        let respond
        let settled = false
        tab.platform.showMessageBox = () => new Promise(resolve => { respond = resolve })
        const pending = tab.canClose().then(result => { settled = true; return result })
        await Promise.resolve()
        assert.equal(settled, false, 'closing must wait for the user response')
        assert.equal(session.open, true)
        respond({ response: answer })
        assert.equal(await pending, answer === 0)
    }
    return prompts
}

const original = await check(load(upstream, 'TelnetTabComponent'))
console.log('Original Telnet close confirmation: reference cases passed')
const actual = await check(load(current, 'TauriTelnetTabComponent'))
assert.deepEqual(actual, original)
console.log('Tauri Telnet close confirmation: inactive, confirm, cancel, unknown response, error, and pending cases match upstream')

// Run the real AppService close methods without its unrelated Angular services.
const appSource = ts.createSourceFile('app.service.ts', fs.readFileSync(new URL('tabby-core/src/services/app.service.ts', root), 'utf8'),
    ts.ScriptTarget.Latest, true)
const appClass = appSource.statements.find(node => ts.isClassDeclaration(node) && node.name.text === 'AppService')
const closeMethods = appClass.members.filter(node => ts.isMethodDeclaration(node) && ['closeAllTabs', 'closeWindow'].includes(node.name.getText(appSource)))
assert.equal(closeMethods.length, 2)
const printer = ts.createPrinter()
const controllerModule = { exports: {} }
const controllerCode = `export class CloseController { ${closeMethods.map(node => printer.printNode(ts.EmitHint.Unspecified, node, appSource)).join('\n')} }`
vm.runInNewContext(ts.transpileModule(controllerCode, { compilerOptions: {
    target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS,
} }).outputText, { exports: controllerModule.exports })
const CurrentTab = load(current, 'TauriTelnetTabComponent')
for (const response of [0, 1]) {
    const controller = new controllerModule.exports.CloseController()
    let resolveDialog
    let dialogStarted
    const started = new Promise(resolve => { dialogStarted = resolve })
    const actions = []
    const tab = new CurrentTab({}, {})
    tab.profile = { options: { host: 'fixture.example' } }
    tab.session = { open: true }
    tab.translate = { instant: text => text }
    tab.platform = { showMessageBox: () => {
        dialogStarted()
        return new Promise(resolve => { resolveDialog = resolve })
    } }
    tab.destroy = () => actions.push('destroy-telnet')
    controller.tabs = [{ canClose: async () => true, destroy: () => actions.push('destroy-other') }, tab]
    controller.tabRecovery = { enabled: true, saveTabs: async () => actions.push('save') }
    controller.hostWindow = { close: () => actions.push('close-window') }
    const closing = controller.closeWindow()
    await started
    assert.deepEqual(actions, ['save'], 'all tabs and the window stay open during confirmation')
    resolveDialog({ response })
    await closing
    assert.deepEqual(actions, response === 0 ? ['save', 'destroy-other', 'destroy-telnet', 'close-window'] : ['save'])
    assert.equal(controller.tabRecovery.enabled, response !== 0)
}
console.log('AppService window close: cancellation preserves every tab; confirmation closes tabs before the window')
