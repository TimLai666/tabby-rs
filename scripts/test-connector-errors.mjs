import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'
import configFactory from '../app/webpack.config.tauri.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const { ResolverFactory, CachedInputFileSystem } = createRequire(import.meta.url)('enhanced-resolve')
const resolver = ResolverFactory.createResolver({
    ...configFactory().resolve,
    fileSystem: new CachedInputFileSystem(fs, 4000),
    useSyncFileSystemCalls: true,
})
const errors = [
    [{ code: 'io', details: 'fixture refused', message: 'wrong field' }, 'fixture refused'],
    [new Error('fixture refused'), 'fixture refused'],
    [{ message: 'fixture refused' }, 'fixture refused'],
    ['fixture refused', 'fixture refused'],
    [{ details: '', message: 'wrong field' }, ''],
    [{ details: 42, message: 'fallback' }, 'fallback'],
    [null, 'null'], [undefined, 'undefined'], [42, '42'],
]

// Run the actual tab initialization. Only Angular, base UI, and the native
// session are replaced; terminal writes and asynchronous cleanup are observed.
async function check (protocol, error, message) {
    const events = []
    let releaseDestroy
    const destroyed = new Promise(resolve => { releaseDestroy = resolve })
    let destroying
    const destroyStarted = new Promise(resolve => { destroying = resolve })
    class Session {
        serviceMessage$ = {}
        async start () { throw error }
        async destroy () {
            events.push('destroy')
            destroying()
            await destroyed
            events.push('destroyed')
        }
    }
    class BaseTab {
        static styles = []
        static animations = []
        static template = ''
        translate = { instant: text => text }
        size = { columns: 80, rows: 24 }
        async initializeSession () {}
        setSession (session) { this.session = session }
        startSpinner () { events.push('startSpinner') }
        stopSpinner () { events.push('stopSpinner') }
        attachSessionHandler () {}
        write (text) { events.push(text) }
    }
    const file = path.join(root, `tabby-tauri/src/${protocol}/tab.component.ts`)
    const nativeRequire = createRequire(file)
    // Use the same dependency instance selected by the real Tauri bundle.
    const colors = nativeRequire(resolver.resolveSync({}, path.dirname(file), 'ansi-colors'))
    const wasEnabled = colors.enabled
    colors.enabled = true
    const module = { exports: {} }
    const require = name => {
        if (name === 'ansi-colors') return colors
        if (name === '@angular/core') return { Component: () => target => target }
        if (name === '@biesbjerg/ngx-translate-extract-marker') return { marker: text => text }
        if (name === 'tabby-core') return { Platform: {} }
        if (name === 'tabby-terminal') return { BaseTerminalTabComponent: BaseTab, ConnectableTerminalTabComponent: BaseTab }
        if (name === './session') return { TauriSerialSession: Session, TauriTelnetSession: Session }
        if (name === './profile') return {}
        if (name.endsWith('.pug')) return ''
        return nativeRequire(name)
    }
    try {
        const code = ts.transpileModule(fs.readFileSync(file, 'utf8'), {
            compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS,
                esModuleInterop: true, experimentalDecorators: true },
        }).outputText
        vm.runInNewContext(code, { module, exports: module.exports, require, setTimeout, clearTimeout }, { filename: file })
        const Tab = module.exports[protocol === 'serial' ? 'TauriSerialTabComponent' : 'TauriTelnetTabComponent']
        const tab = new Tab({}, {})
        let finished = false
        const initialized = tab.initializeSession().then(() => { finished = true })
        await destroyStarted
        await Promise.resolve()
        assert.equal(finished, false, `${protocol}: await session cleanup`)
        // Fixed upstream error rendering, including the badge and message colors.
        const expected = '\x1b[30m\x1b[41m X \x1b[49m\x1b[39m '
            + (message ? `\x1b[31m${message}\x1b[39m` : '') + '\r\n'
        assert.deepEqual(events, ['startSpinner', 'stopSpinner', expected, 'destroy'], protocol)
        releaseDestroy()
        await initialized
        assert.deepEqual(events, ['startSpinner', 'stopSpinner', expected, 'destroy', 'destroyed'], protocol)
    } finally {
        releaseDestroy()
        colors.enabled = wasEnabled
    }
}

for (const protocol of ['serial', 'telnet']) {
    for (const [error, message] of errors) await check(protocol, error, message)
    console.log(`${protocol}: ${errors.length} error shapes, upstream colors, spinner stop, and awaited cleanup passed`)
}
