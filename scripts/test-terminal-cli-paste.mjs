import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'

const root = process.cwd()
const target = process.argv[2] ?? path.join(root, 'tabby-terminal/src/cli.ts')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const angular = {
    Injectable: () => value => value,
    Component: () => value => value,
    ViewChild: () => () => {},
}
class BaseTabComponent {}
class BaseTerminalTabComponent extends BaseTabComponent {}

function load (file, dependencies) {
    const module = { exports: {} }
    const source = fs.readFileSync(file, 'utf8')
    const output = ts.transpileModule(source, { compilerOptions: {
        target: ts.ScriptTarget.ES2020,
        module: ts.ModuleKind.CommonJS,
        experimentalDecorators: true,
        esModuleInterop: true,
    } }).outputText
    vm.runInNewContext(output, {
        exports: module.exports, module, console,
        require: name => {
            if (name in dependencies) return dependencies[name]
            throw new Error('Unprovided dependency: ' + name)
        },
    }, { filename: file })
    return module.exports
}

const { SplitTabComponent } = load(path.join(root, 'tabby-core/src/components/splitTab.component.ts'), {
    rxjs: require('rxjs'),
    '@angular/core': angular,
    './baseTab.component': { BaseTabComponent },
    '../api/tabRecovery': { TabRecoveryProvider: class {} },
    '../api/tabProgress': {},
    '../services/tabs.service': {},
    '../services/hotkeys.service': {},
    '../services/tabRecovery.service': {},
})
const { TerminalCLIHandler } = load(target, {
    '@angular/core': angular,
    'shell-quote': require('shell-quote'),
    'tabby-core': { CLIHandler: class {}, SplitTabComponent },
    './api/baseTerminalTab.component': { BaseTerminalTabComponent },
})

const terminal = () => Object.assign(new BaseTerminalTabComponent(), {
    session: {}, writes: [], sendInput (text) { this.writes.push(text) },
})
const split = focusedTab => Object.assign(Object.create(SplitTabComponent.prototype), { focusedTab })
const text = '貼上驗證|OK'
const cases = []

async function check (name, setup, expectedWrites, expectedFront, {
    response = 0, escape = false, dialogError = false, operation = 'paste', afterPrompt,
} = {}) {
    const focused = terminal()
    const other = terminal()
    const app = { activeTab: setup(focused, other) }
    let front = 0
    const prompts = []
    const handler = new TerminalCLIHandler(app, { bringToFront: () => front++ }, {
        showMessageBox: async options => {
            prompts.push(options)
            afterPrompt?.(app, focused, other)
            if (dialogError) throw new Error('dialog unavailable')
            return { response }
        },
    }, { instant: (value, data) => data ? value.replace('{text}', data.text) : value })
    try {
        const event = { argv: { _: [operation], text, escape } }
        if (dialogError) {
            await assert.rejects(handler.handle(event), /dialog unavailable/)
        } else {
            assert.equal(await handler.handle(event), operation === 'paste')
        }
        const escaped = require('shell-quote').quote([text])
        assert.deepEqual(focused.writes, expectedWrites === 'escaped' ? [escaped] : expectedWrites ? [text] : [])
        assert.deepEqual(other.writes, [], 'another pane receives no input')
        assert.equal(front, expectedFront)
        assert.equal(prompts.length, operation === 'paste' ? 1 : 0)
        if (prompts.length) {
            assert.equal(prompts[0].message, `Paste "${escape ? escaped : text}"?`)
            assert.deepEqual(Array.from(prompts[0].buttons), ['Paste', 'Cancel'])
            assert.equal(prompts[0].cancelId, 1)
        }
        cases.push({ name, passed: true })
    } catch (error) {
        cases.push({ name, passed: false, error: error.message })
    }
}

await check('direct terminal', focused => focused, true, 1)
await check('focused pane in split container', focused => split(focused), true, 1)
await check('cancel leaves pane untouched', focused => split(focused), false, 0, { response: 1 })
await check('escaped payload reaches focused pane', focused => split(focused), 'escaped', 1, { escape: true })
await check('no active tab', () => null, false, 0)
await check('no focused pane', () => split(null), false, 0)
await check('nonterminal focused tab', () => split(new BaseTabComponent()), false, 0)
await check('missing session', focused => { focused.session = null; return split(focused) }, false, 0)
await check('dialog failure', focused => split(focused), false, 0, { dialogError: true })
await check('non-paste command', focused => split(focused), false, 0, { operation: 'other' })
await check('focus resolved after confirmation', (focused, other) => split(other), true, 1, {
    afterPrompt: (app, focused) => { app.activeTab.focusedTab = focused },
})

console.log(JSON.stringify({
    scope: 'Actual CLI handler and SplitTabComponent.getFocusedTab; modal, PTY, and terminal boundaries are controlled substitutes.',
    passed: cases.filter(test => test.passed).length,
    failed: cases.filter(test => !test.passed).length,
    cases,
}, null, 2))
process.exitCode = cases.every(test => test.passed) ? 0 : 1
