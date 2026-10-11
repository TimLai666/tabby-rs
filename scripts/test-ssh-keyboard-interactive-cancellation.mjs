import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import vm from 'node:vm'
import ts from 'typescript'

const baselineCommit = '14e2d60b9b6dee84a53c37f05eefeb803787de04'
const source = execFileSync('git', ['show', `${baselineCommit}:tabby-ssh/src/session/ssh.ts`], { encoding: 'utf8' })
const parsed = ts.createSourceFile('ssh.ts', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
let method
function visit (node) {
    if (ts.isMethodDeclaration(node) && node.name.getText(parsed) === '_handleAuth') method = node
    ts.forEachChild(node, visit)
}
visit(parsed)
assert.ok(method, 'the fixed baseline _handleAuth method must exist')
const methodJs = ts.transpileModule(`class Baseline { ${method.getText(parsed)} }`, {
    compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS },
}).outputText
const baselineModule = { exports: {} }
let baselinePromptClass
function visitPrompt (node) { if (ts.isClassDeclaration(node) && node.name?.text === 'KeyboardInteractivePrompt') baselinePromptClass = node; ts.forEachChild(node, visitPrompt) }
visitPrompt(parsed)
assert.ok(baselinePromptClass, 'the fixed baseline shared prompt model must exist')
const promptJs = ts.transpileModule(baselinePromptClass.getText(parsed), { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS } }).outputText
const promptModule = { exports: {} }
vm.runInNewContext(`${promptJs}\nexports.KeyboardInteractivePrompt = KeyboardInteractivePrompt`, { exports: promptModule.exports })
const KeyboardInteractivePrompt = promptModule.exports.KeyboardInteractivePrompt
let promptOutcome = 'cancel'
class SSHClient {}
class AuthenticatedSSHClient {}
vm.runInNewContext(`${methodJs}\nexports.Baseline = Baseline`, {
    exports: baselineModule.exports, KeyboardInteractivePrompt,
    russh: { SSHClient, AuthenticatedSSHClient },
    sshAuthTypeForMethod: method => method.type === 'keyboard-interactive' ? 'keyboard-interactive' : 'password',
})
const Baseline = baselineModule.exports.Baseline

async function run (rounds, storedPassword = 'saved', remainingMethods = []) {
    rounds = [...rounds]
    const calls = []
    const client = Object.assign(Object.create(Baseline.prototype), {
        ssh: Object.assign(Object.create(SSHClient.prototype), {
            async authenticateNone () { return { remainingMethods } },
            async startKeyboardInteractiveAuthentication () {
                calls.push('start')
                const next = rounds.shift()
                if (next instanceof Error) throw next
                return { state: 'prompt', name: 'fixture', instructions: '', prompts: () => (next ?? { prompts: [{ prompt: 'Password: ', echo: false }] }).prompts }
            },
            async continueKeyboardInteractiveAuthentication (answers) {
                calls.push(['continue', answers])
                const next = rounds.shift()
                if (next instanceof Error) throw next
                return next ? { state: 'prompt', name: 'fixture', instructions: '', prompts: () => next.prompts } : new AuthenticatedSSHClient()
            },
            async authenticateWithPassword (_username, password) { calls.push(['password', password]); return new AuthenticatedSSHClient() },
        }),
        authUsername: 'alice', allAuthMethods: [
            { type: 'keyboard-interactive', savedPassword: null },
            { type: 'saved-password', password: storedPassword },
        ],
        previouslyDisconnected: false,
        profile: { options: { host: 'fixture.test' } }, ngbModal: { open () { throw new Error('unexpected modal') } },
        translate: { instant: value => value },
        passwordStorage: { async loadPassword () { return null } },
        emitKeyboardInteractivePrompt (prompt) { calls.push(['prompt', prompt]); if (promptOutcome === 'resolve') prompt.respond(); else prompt.reject() }, emitServiceMessage () {},
    })
    return { result: await client._handleAuth(), calls }
}

const nonempty = [{ prompts: [{ prompt: 'Password: ', echo: false }] }]
{
    promptOutcome = 'cancel'
    const { result, calls } = await run(nonempty)
    assert.ok(result instanceof AuthenticatedSSHClient)
    assert.deepEqual(calls.filter(call => Array.isArray(call) && call[0] === 'continue'), [])
    assert.deepEqual(calls.at(-1), ['password', 'saved'])
}
{
    promptOutcome = 'cancel'
    const { calls } = await run([{ prompts: [] }, ...nonempty])
    assert.deepEqual(JSON.parse(JSON.stringify(calls.find(call => Array.isArray(call) && call[0] === 'continue'))), ['continue', []], 'empty challenge auto-continues with []')
}
{
    promptOutcome = 'cancel'
    const { result, calls } = await run(nonempty, 'saved', ['keyboard-interactive', 'password'])
    assert.ok(result instanceof AuthenticatedSSHClient)
    assert.deepEqual(calls.at(-1), ['password', 'saved'])
}
for (const error of [new Error('start failed'), new Error('continue failed')]) {
    promptOutcome = error.message.startsWith('continue') ? 'resolve' : 'cancel'
    const rounds = error.message.startsWith('start') ? [error] : [{ prompts: [{ prompt: 'Code: ', echo: true }] }, error]
    await assert.rejects(run(rounds), error)
}
{
    promptOutcome = 'resolve'
    const { calls } = await run([])
    const sent = calls.find(call => Array.isArray(call) && call[0] === 'continue')
    assert.deepEqual(JSON.parse(JSON.stringify(sent)), ['continue', ['']], 'one empty string is a real submitted blank answer')
}

function loadTypeScript (file, modules) {
    const text = fs.readFileSync(new URL(file, import.meta.url), 'utf8')
    const js = ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true } }).outputText
    const loaded = { exports: {} }
    vm.runInNewContext(js, { exports: loaded.exports, require: name => { assert.ok(name in modules, name); return modules[name] }, URL, setTimeout, clearTimeout, console })
    return loaded.exports
}
const currentPrompt = loadTypeScript('../tabby-ssh/src/api/keyboardInteractivePrompt.ts', {})
const decorator = () => target => target
const { TauriSshTabComponent } = loadTypeScript('../tabby-tauri/src/ssh/tab.component.ts', {
    '@angular/core': { Component: () => decorator() },
    'tabby-core': { Platform: { Web: 'web' } },
    'tabby-terminal': { BaseTerminalTabComponent: { template: '', styles: [] }, ConnectableTerminalTabComponent: class {
        async disconnect () {} ngOnDestroy () {} onSessionDestroyed () {} async destroy () {} initializeSession () {}
        attachSessionHandler () {} setSession (value) { this.session = value }
    } },
    '../../../tabby-ssh/src/api/keyboardInteractivePrompt': currentPrompt,
    './session': { TauriSshSession: class {} }, './portForwardingModal.component': { TauriSshPortForwardingModalComponent: class {} },
    './toolbar.component.pug': '', './tab.component.scss': '',
})
const currentCalls = []
const tab = new TauriSshTabComponent({}, { invoke: async (command, request) => currentCalls.push({ command, request }) }, {}, { open: () => { throw new Error('the inline prompt must be used') } }, {})
tab.logger = { warn () {} }
tab.profile = { id: 'profile', options: { host: 'target.test', port: 22, user: 'alice', password: 'target-secret' } }
const currentSession = {}
tab.session = currentSession
const event = { requestId: 'dismiss', name: 'Fixture', instructions: '', prompts: [{ text: 'Code: ', echo: true }], keyboardInteractive: { host: 'target.test', port: 22, username: 'alice' } }
const dismissed = tab.showAuthPrompt(event, currentSession)
tab.activeKIPrompt.reject()
await dismissed
assert.deepEqual(JSON.parse(JSON.stringify(currentCalls.at(-1))), { command: 'ssh.authResponse', request: { requestId: 'dismiss', responses: [] } }, 'current renderer prompt rejection sends [] without abort')
const blank = tab.showAuthPrompt({ ...event, requestId: 'blank' }, currentSession)
assert.deepEqual(Array.from(tab.activeKIPrompt.responses), [''])
tab.activeKIPrompt.respond()
await blank
assert.deepEqual(JSON.parse(JSON.stringify(currentCalls.at(-1).request)), { requestId: 'blank', responses: [''] }, 'current tab preserves one submitted blank string')
const teardown = tab.showAuthPrompt({ ...event, requestId: 'teardown' }, currentSession)
await tab.disconnect()
await teardown
assert.deepEqual(JSON.parse(JSON.stringify(currentCalls.at(-1).request)), { requestId: 'teardown', responses: [], abort: true }, 'disconnect aborts the pending prompt')

console.log('Baseline _handleAuth, baseline shared prompt, current renderer rejection/blank/teardown, and transport error controls passed')
