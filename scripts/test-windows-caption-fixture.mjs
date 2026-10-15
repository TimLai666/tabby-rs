#!/usr/bin/env node

import assert from 'node:assert/strict'
import { execFileSync, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const script = path.join(root, 'scripts/build-windows-caption-fixture.mjs')
const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-caption-fixture-contract-'))
const run = args => spawnSync(process.execPath, [script, ...args], { cwd: os.tmpdir(), encoding: 'utf8', timeout: 30000 })

try {
    const help = run(['--help'])
    assert.equal(help.status, 0, help.stderr)
    for (const fragment of ['--output', 'paint(snapshot)', 'read()', 'Exit codes:']) assert.ok(help.stdout.includes(fragment))
    for (const args of [[], ['--output'], ['--output', '--help'], ['--unknown', 'x'], ['--output', 'x', '--output', 'y']]) {
        const result = run(args)
        assert.equal(result.status, 2, `${JSON.stringify(args)}: ${result.stderr}`)
        assert.match(result.stderr, /Usage:/)
    }
    const output = path.join(directory, 'caption with spaces.js')
    const built = run(['--output', output])
    assert.equal(built.status, 0, built.stderr)
    assert.equal(built.stdout.trim(), output)
    const payload = fs.readFileSync(output, 'utf8')
    assert.ok(Buffer.byteLength(payload) > 1000 && Buffer.byteLength(payload) < 65536)
    execFileSync(process.execPath, ['--check', output], { stdio: 'pipe' })
    const window = {}
    const context = vm.createContext({ window })
    vm.runInContext(payload, context, { timeout: 1000 })
    assert.deepEqual(Object.keys(window).sort(), ['__tabbyCaptionFixture'])
    const fixture = window.__tabbyCaptionFixture
    const painter = fs.readFileSync(path.join(root, 'tabby-tauri/src/components/windowsCaption.ts'))
    assert.equal(fixture.sourceSha256, createHash('sha256').update(painter).digest('hex'))
    assert.equal(typeof fixture.paint, 'function')
    assert.equal(typeof fixture.read, 'function')
    assert.equal(fixture.read(), null, 'Installing the fixture must not paint or require a document')
    assert.equal(typeof context.exports, 'undefined', 'CommonJS compiler exports must stay inside the payload')
    const before = fs.readFileSync(output)
    const repeated = run(['--output', output])
    assert.equal(repeated.status, 1, repeated.stderr)
    assert.deepEqual(fs.readFileSync(output), before, 'Existing output must survive a duplicate build')
    const missingParent = run(['--output', path.join(directory, 'absent', 'painter.js')])
    assert.equal(missingParent.status, 1, missingParent.stderr)
    assert.ok(!fs.existsSync(path.join(directory, 'absent')), 'A typo must not create output parent directories')

    const referencePath = path.join(root, 'src-tauri/examples/windows_frame_native/electron.cjs')
    const referenceSource = fs.readFileSync(referencePath, 'utf8')

    const bootReference = locale => {
        const events = []
        const switches = []
        const writes = []
        const readinessSwitches = { value: null }
        const app = {
            commandLine: {
                appendSwitch: (key, value) => {
                    events.push(`appendSwitch:${key}=${value}`)
                    switches.push(`${key}=${value}`)
                },
            },
            setPath: (name, value) => events.push(`setPath:${name}=${value}`),
            getLocale: () => locale,
            quit: () => events.push('quit'),
            exit: code => events.push(`exit:${code}`),
            on: name => events.push(`on:${name}`),
            whenReady: () => {
                events.push('whenReady:begin')
                readinessSwitches.value = [...switches]
                return {
                    then: callback => {
                        const settled = Promise.resolve().then(() => callback())
                        sandbox.__completion = settled
                        return settled
                    },
                }
            },
        }
        class BrowserWindow {
            constructor (options) {
                this.options = options
                this.webContents = { executeJavaScript: () => Promise.resolve() }
                this.handle = Buffer.alloc(8)
            }
            setMenu () {}
            setTitleBarOverlay () {}
            loadURL () { return Promise.resolve() }
            setAlwaysOnTop () {}
            show () {}
            focus () {}
            getNativeWindowHandle () { return this.handle }
            getBounds () { return { x: 80, y: 60, width: 800, height: 580 } }
            getContentBounds () { return { x: 80, y: 60, width: 800, height: 580 } }
            destroy () {}
        }
        const simulatedFs = {
            mkdirSync: () => {},
            writeFileSync: (target, data, options) => writes.push({ target, data, options }),
            existsSync: () => false,
        }
        const sandbox = {
            __dirname: path.dirname(referencePath),
            process: {
                platform: 'win32',
                pid: 4444,
                argv: ['electron', referencePath, 'data-dir', 'ready.json', 'close-flag', 'thin'],
                versions: { electron: '38.8.6', chrome: '140.0.0.0' },
            },
            console: { log: () => {}, error: () => {} },
            setInterval: () => { events.push('setInterval'); return { unref: () => {} } },
            clearInterval: () => {},
            setTimeout: () => ({ unref: () => {} }),
            require: id => {
                if (id === 'node:fs') return simulatedFs
                if (id === 'node:path') return path
                if (id === 'node:module') {
                    return {
                        createRequire: () => specifier => {
                            if (specifier === 'glasstron') return { BrowserWindow }
                            throw new Error(`Unexpected reference require: ${specifier}`)
                        },
                    }
                }
                if (id === 'electron') return { app, nativeTheme: {} }
                throw new Error(`Unexpected reference require: ${id}`)
            },
        }
        vm.runInContext(referenceSource, vm.createContext(sandbox), { filename: 'windows_frame_native/electron.cjs', timeout: 5000 })
        return { events, switches, writes, readinessSwitches, completion: sandbox.__completion }
    }

    for (const locale of ['en-US', 'ja-JP']) {
        const observed = bootReference(locale)
        assert.ok(observed.completion, `electron.cjs must reach app.whenReady() for locale ${locale}`)
        await observed.completion
        assert.ok(
            observed.readinessSwitches.value.includes('lang=EN'),
            `app.commandLine.appendSwitch('lang','EN') must already be set when app.whenReady() runs (${locale}): ${JSON.stringify(observed.readinessSwitches.value)}`,
        )
        assert.ok(
            observed.events.indexOf('appendSwitch:lang=EN') < observed.events.indexOf('whenReady:begin'),
            `appendSwitch('lang','EN') must run before app.whenReady() (${locale})`,
        )
        assert.equal(observed.writes.length, 1, `readiness manifest must be written exactly once (${locale})`)
        const manifest = JSON.parse(observed.writes[0].data)
        assert.equal(manifest.mode, 'thin')
        assert.equal(
            manifest.locale,
            locale,
            `readiness manifest locale must be app.getLocale() (${locale}), not a hardcoded English value`,
        )
    }

    console.log('Caption fixture: CLI errors, safe output, source identity, isolated installation, and reference boot lang/locale passed')
} finally {
    fs.rmSync(directory, { recursive: true, force: true })
}
