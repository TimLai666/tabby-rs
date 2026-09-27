import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import ts from 'typescript'

function load (file, dependencies = {}) {
    const loaded = { exports: {} }
    vm.runInNewContext(ts.transpileModule(fs.readFileSync(new URL(file, import.meta.url), 'utf8'), {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true },
    }).outputText, {
        exports: loaded.exports,
        require: name => { assert.ok(name in dependencies, `Unexpected runtime dependency: ${name}`); return dependencies[name] },
        URL, setTimeout, clearTimeout,
    })
    return loaded.exports
}
const { KeyboardInteractivePrompt } = load('../tabby-ssh/src/api/keyboardInteractivePrompt.ts')
const prompts = [{ prompt: 'PASSWORD: ', echo: false }, { prompt: 'Password visible', echo: true }, { prompt: 'Code: ' }]
const a = new KeyboardInteractivePrompt('name', 'instructions', prompts)
const b = new KeyboardInteractivePrompt('name', '', prompts)
assert.deepEqual(Array.from(a.responses), ['', '', ''])
assert.equal(a.isAPasswordPrompt(0), true)
assert.equal(a.isAPasswordPrompt(1), false)
assert.equal(a.isAPasswordPrompt(2), false)
a.responses.splice(0, 3, 'synthetic-password', 'visible-answer', '123456')
a.respond()
assert.deepEqual(Array.from(await a.promise), ['synthetic-password', 'visible-answer', '123456'])
assert.deepEqual(Array.from(b.responses), ['', '', ''])
const rejected = assert.rejects(b.promise, /Keyboard-interactive auth rejected/)
b.reject()
await rejected
console.log('Shared keyboard-interactive model: isolated answers, masking, response and rejection passed')

const decorator = () => target => target
let metadata
let startImplementation
const { TauriSshTabComponent } = load('../tabby-tauri/src/ssh/tab.component.ts', {
    '@angular/core': { Component: value => { metadata = value; return decorator() } },
    'tabby-terminal': { BaseTerminalTabComponent: { template: '<terminal />' }, ConnectableTerminalTabComponent: class {
        async disconnect () {} ngOnDestroy () {} onSessionDestroyed () {}
        async initializeSession () {} setSession (session) { this.session = session } attachSessionHandler () {}
    } },
    '../../../tabby-ssh/src/api/keyboardInteractivePrompt': { KeyboardInteractivePrompt },
    './session': { TauriSshSession: class {
        start () { return startImplementation(this) } async destroy () {}
    } },
})
const calls = []
const tab = new TauriSshTabComponent({}, {
    invoke: async (command, request) => { calls.push({ command, request }) },
}, {}, { open: () => { throw new Error('Interactive SSH must use the inline panel') } }, {})
tab.logger = { warn () {} }
tab.profile = { id: 'target', options: { host: 'target.test', port: 22, user: '$USER', password: 'target-only' } }
const session = {}
tab.session = session
const event = {
    requestId: 'ki-1', name: 'Login', instructions: 'Server notice',
    prompts: [{ text: 'Password: ', echo: false }, { text: 'Verification code: ', echo: true }],
    keyboardInteractive: { host: 'hop.test', port: 2222, username: 'resolved-hop' },
}
const shown = tab.showAuthPrompt(event, session)
assert.ok(tab.activeKIPrompt instanceof KeyboardInteractivePrompt)
const originalPrompt = tab.activeKIPrompt
const duplicate = tab.showAuthPrompt(event, session)
assert.equal(tab.activeKIPrompt, originalPrompt, 'Duplicate prompt delivery must not cancel the native waiter')
await duplicate
assert.deepEqual(JSON.parse(JSON.stringify(tab.activeKIProfile.options)), {
    host: 'hop.test', port: 2222, user: 'resolved-hop',
})
assert.deepEqual(Array.from(tab.activeKIPrompt.responses), ['', ''])
tab.activeKIPrompt.responses.splice(0, 2, 'hop-answer', 'otp-answer')
tab.activeKIPrompt.respond()
await shown
assert.deepEqual(JSON.parse(JSON.stringify(calls)), [{ command: 'ssh.authResponse', request: { requestId: 'ki-1', responses: ['hop-answer', 'otp-answer'] } }])
assert.equal(tab.activeKIPrompt, null)
assert.equal(tab.profile.options.user, '$USER')
assert.equal(tab.profile.options.password, 'target-only')
const cancelled = tab.showAuthPrompt({ ...event, requestId: 'ki-2' }, session)
const oldPrompt = tab.activeKIPrompt
await tab.disconnect()
await cancelled
oldPrompt.respond()
assert.equal(tab.activeKIPrompt, null)
assert.equal(calls.length, 2)
assert.deepEqual(Array.from(calls[1].request.responses), [])
await tab.showAuthPrompt({ ...event, requestId: 'stale' }, {})
assert.equal(tab.activeKIPrompt, null)
assert.equal(calls.length, 2)
assert.match(metadata.template, /keyboard-interactive-auth-panel/)
assert.match(metadata.template, /\[profile\]="activeKIProfile"/)
console.log('Tauri inline keyboard-interactive panel: resolved hop identity, response, cancellation and stale-session isolation passed')

let timedOutPrompt
tab.write = () => {}
startImplementation = async session => {
    timedOutPrompt = tab.showAuthPrompt({ ...event, requestId: 'expired' }, session)
    throw new Error('SSH operation timed out')
}
await tab.initializeSession()
assert.equal(tab.activeKIPrompt, null, 'A failed initial connection must clear its pending authentication panel')
await timedOutPrompt
assert.deepEqual(Array.from(calls.at(-1).request.responses), [])
console.log('Initial SSH connection failure clears the interactive panel and cancels its response')

const { KeyboardInteractiveAuthComponent } = load('../tabby-ssh/src/components/keyboardInteractiveAuthPanel.component.ts', {
    '@angular/core': {
        Component: decorator, Input: () => () => {}, Output: () => () => {}, ViewChild: () => () => {},
        ChangeDetectionStrategy: { OnPush: 0 }, EventEmitter: class { emit () {} },
    },
    '@angular/common': {}, '@angular/forms': {}, 'tabby-core': {},
})
const saves = []
const panel = new KeyboardInteractiveAuthComponent({
    loadPassword: async profile => { assert.equal(profile.options.user, 'resolved-hop'); return 'saved-hop' },
    savePassword: async (profile, value) => saves.push({ profile, value }),
}, {}, { markForCheck () {} })
panel.profile = { options: { host: 'hop.test', port: 2222, user: 'resolved-hop' } }
panel.prompt = new KeyboardInteractivePrompt('Two steps', '', prompts)
panel.input = { nativeElement: { focus () {} } }
await panel.ngOnInit()
assert.deepEqual(Array.from(panel.prompt.responses), ['saved-hop', '', ''])
assert.equal(panel.shouldEcho(), false)
panel.remember = true
panel.next()
assert.equal(panel.step, 1)
assert.equal(panel.shouldEcho(), true)
assert.equal(saves.length, 1)
assert.equal(saves[0].profile, panel.profile)
assert.equal(saves[0].value, 'saved-hop')
panel.previous()
assert.equal(panel.step, 0)
panel.remember = false
panel.next()
panel.prompt.responses[1] = 'echo-answer'
panel.next()
panel.prompt.responses[2] = 'otp'
panel.next()
assert.deepEqual(Array.from(await panel.prompt.promise), ['saved-hop', 'echo-answer', 'otp'])
assert.equal(saves.length, 1, 'Only explicit consent on a password field can save a password')
let finishLoad
const delayed = new KeyboardInteractiveAuthComponent({ loadPassword: () => new Promise(resolve => { finishLoad = resolve }) }, {}, { markForCheck () {} })
delayed.profile = panel.profile
delayed.prompt = new KeyboardInteractivePrompt('', '', prompts)
const loading = delayed.ngOnInit()
delayed.prompt.responses[0] = 'already-typed'
finishLoad('stored')
await loading
assert.equal(delayed.prompt.responses[0], 'already-typed')
console.log('Shared panel: resolved-account prefill, echo, previous/next/finish, explicit password saving and late-load input preservation passed')

const notices = []
const unavailable = new KeyboardInteractiveAuthComponent({
    loadPassword: async () => { throw new Error('private storage details') },
    savePassword: async () => { throw new Error('private storage details') },
}, {}, { markForCheck () {} }, { error: text => notices.push(text) }, { instant: text => text })
unavailable.profile = panel.profile
unavailable.prompt = new KeyboardInteractivePrompt('', '', [prompts[0]])
await assert.doesNotReject(unavailable.ngOnInit())
unavailable.prompt.responses[0] = 'manual-password'
unavailable.remember = true
unavailable.next()
assert.deepEqual(Array.from(await unavailable.prompt.promise), ['manual-password'])
await Promise.resolve()
assert.deepEqual(notices, ['Could not save password'])
console.log('Unavailable secret storage preserves manual login and reports save failure without private details')
