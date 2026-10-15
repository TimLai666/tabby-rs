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

    const tooltipLabels = ['Maximize', 'Restore']
    const tooltipKernings = ['normal', 'none']
    const requestedTooltipFont = '12px "Segoe UI"'
    const sentinelSets = [
        {
            name: 'primary',
            values: {
                Maximize: {
                    normal: { width: 41.25, left: 1.5, right: 42.75 },
                    none: { width: 43.5, left: 2.25, right: 45.75 },
                },
                Restore: {
                    normal: { width: 47.125, left: 1.875, right: 49 },
                    none: { width: 49.375, left: 2.625, right: 52 },
                },
            },
        },
        {
            name: 'secondary',
            values: {
                Maximize: {
                    normal: { width: 61.75, left: 3.5, right: 65.25 },
                    none: { width: 63.875, left: 4.125, right: 68 },
                },
                Restore: {
                    normal: { width: 69.5, left: 3.75, right: 73.25 },
                    none: { width: 71.625, left: 4.5, right: 76.125 },
                },
            },
        },
    ]
    const createTooltipCanvasBoundary = ({ values, rejectNoneKerning, fontReadback }) => {
        const state = { fontSets: [], kerningSets: [], measureCalls: [] }
        let font = ''
        let fontKerning = 'normal'
        const context2d = {}
        Object.defineProperties(context2d, {
            font: {
                get: () => font,
                set: next => {
                    state.fontSets.push(next)
                    font = next === requestedTooltipFont ? fontReadback : next
                },
            },
            fontKerning: {
                get: () => fontKerning,
                set: next => {
                    state.kerningSets.push(next)
                    if (!(rejectNoneKerning && next === 'none')) fontKerning = next
                },
            },
        })
        context2d.measureText = text => {
            const byKerning = values[text]
            if (!byKerning) throw new Error(`Unexpected tooltip measureText string: ${JSON.stringify(text)}`)
            const metrics = byKerning[fontKerning]
            if (!metrics) throw new Error(`Unexpected fontKerning during tooltip measureText: ${JSON.stringify(fontKerning)}`)
            state.measureCalls.push({ text, font, fontKerning })
            return {
                width: metrics.width,
                actualBoundingBoxLeft: metrics.left,
                actualBoundingBoxRight: metrics.right,
            }
        }
        const canvas = {
            getContext: kind => {
                if (kind !== '2d') throw new Error(`Unexpected canvas context kind: ${JSON.stringify(kind)}`)
                return context2d
            },
        }
        const document = {
            createElement: tag => {
                if (tag !== 'canvas') throw new Error(`Tooltip metrics may only create canvas elements, got: ${JSON.stringify(tag)}`)
                return canvas
            },
        }
        return { document, state }
    }

    const bootReference = ({ locale, sentinels, rejectNoneKerning, fontReadback }) => {
        const events = []
        const switches = []
        const writes = []
        const readinessSwitches = { value: null }
        const tooltipCanvas = createTooltipCanvasBoundary({ values: sentinels, rejectNoneKerning, fontReadback })
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
                this.webContents = {
                    executeJavaScript: script => {
                        if (typeof script === 'string' && script.includes('requestAnimationFrame')) {
                            events.push('executeJavaScript:requestAnimationFrame-boundary')
                            return Promise.resolve()
                        }
                        events.push('executeJavaScript:canvas-tooltip-metrics')
                        try {
                            const diagnosticContext = vm.createContext({ document: tooltipCanvas.document })
                            const result = vm.runInContext(script, diagnosticContext, {
                                filename: 'windows_frame_native/tooltip-metrics.js',
                                timeout: 1000,
                            })
                            return Promise.resolve(result)
                        } catch (error) {
                            return Promise.reject(error)
                        }
                    },
                }
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
        return { events, switches, writes, readinessSwitches, completion: sandbox.__completion, canvas: tooltipCanvas.state }
    }

    const bootCases = [
        { name: 'en-US primary sentinels', locale: 'en-US', fontReadback: 'normal 12px "Segoe UI"', sentinels: sentinelSets[0].values, rejectNoneKerning: false },
        { name: 'ja-JP secondary sentinels', locale: 'ja-JP', fontReadback: '400 12px "Segoe UI"', sentinels: sentinelSets[1].values, rejectNoneKerning: false },
        { name: 'en-US primary sentinels with rejected fontKerning none', locale: 'en-US', fontReadback: 'normal 12px "Segoe UI"', sentinels: sentinelSets[0].values, rejectNoneKerning: true },
    ]

    const assertTooltipTextMetrics = (manifest, canvasState, bootCase) => {
        const { name, sentinels, rejectNoneKerning } = bootCase
        assert.ok(
            manifest.tooltipTextMetrics !== undefined,
            `readiness manifest must include tooltipTextMetrics (${name}); manifest keys: ${Object.keys(manifest).join(', ')}`,
        )
        const metrics = manifest.tooltipTextMetrics
        assert.equal(metrics.kind, 'canvas-tooltip-text-metrics-diagnostic', `tooltipTextMetrics.kind must identify the canvas diagnostic (${name})`)
        assert.equal(metrics.diagnosticOnly, true, `tooltipTextMetrics.diagnosticOnly must stay true (${name})`)
        assert.equal(metrics.nativeTooltipMetricsVerified, false, `this fixture never verifies native Windows typesetting (${name})`)
        assert.equal(metrics.requestedFont, requestedTooltipFont, `tooltipTextMetrics.requestedFont must stay ${requestedTooltipFont} (${name})`)
        assert.equal(typeof metrics.font, 'string', `tooltipTextMetrics.font must be the Canvas font readback (${name})`)
        assert.ok(metrics.font.length > 0, `tooltipTextMetrics.font must not be empty (${name})`)
        assert.equal(metrics.measurements.length, 4, `tooltipTextMetrics.measurements must hold one entry per label and kerning (${name}): ${JSON.stringify(metrics.measurements)}`)

        assert.ok(
            canvasState.kerningSets.includes('normal'),
            `the diagnostic must set Canvas fontKerning normal (${name}): ${JSON.stringify(canvasState.kerningSets)}`,
        )
        assert.ok(
            canvasState.kerningSets.includes('none'),
            `the diagnostic must attempt Canvas fontKerning none (${name}): ${JSON.stringify(canvasState.kerningSets)}`,
        )

        assert.ok(canvasState.fontSets.includes(requestedTooltipFont), 'the requested font must be assigned: ' + name)
        assert.equal(metrics.font, bootCase.fontReadback, 'font must preserve the Canvas property readback: ' + name)
        assert.equal(canvasState.measureCalls.length, 4, 'measureText must run for all four requests: ' + name)
        for (const call of canvasState.measureCalls) {
            assert.equal(call.font, metrics.font, 'each measurement must use the reported Canvas font: ' + name)
        }

        const entries = new Map()
        for (const entry of metrics.measurements) {
            assert.ok(tooltipLabels.includes(entry.label), `measurement labels must be Maximize or Restore (${name}): ${JSON.stringify(entry)}`)
            assert.ok(tooltipKernings.includes(entry.requestedKerning), `measurement requestedKerning must be normal or none (${name}): ${JSON.stringify(entry)}`)
            entries.set(`${entry.label}/${entry.requestedKerning}`, entry)
        }
        assert.equal(entries.size, 4, `measurements must cover Maximize/Restore x normal/none exactly once (${name}): ${JSON.stringify([...entries.keys()])}`)

        for (const label of tooltipLabels) {
            for (const requestedKerning of tooltipKernings) {
                const entry = entries.get(`${label}/${requestedKerning}`)
                const effectiveKerning = rejectNoneKerning && requestedKerning === 'none' ? 'normal' : requestedKerning
                const expected = sentinels[label][effectiveKerning]
                assert.equal(
                    entry.actualKerning,
                    effectiveKerning,
                    `actualKerning must record the Canvas fontKerning readback for ${label}/${requestedKerning} (${name})`,
                )
                assert.equal(entry.width, expected.width, `width must equal the fake Canvas sentinel for ${label}/${effectiveKerning} (${name})`)
                assert.equal(entry.actualBoundingBoxLeft, expected.left, `actualBoundingBoxLeft must equal the fake Canvas sentinel for ${label}/${effectiveKerning} (${name})`)
                assert.equal(entry.actualBoundingBoxRight, expected.right, `actualBoundingBoxRight must equal the fake Canvas sentinel for ${label}/${effectiveKerning} (${name})`)
            }
        }
    }

    for (const bootCase of bootCases) {
        const observed = bootReference(bootCase)
        assert.ok(observed.completion, `electron.cjs must reach app.whenReady() for locale ${bootCase.locale}`)
        await observed.completion
        assert.ok(
            observed.events.includes('executeJavaScript:requestAnimationFrame-boundary'),
            `the requestAnimationFrame wait must stay a framework boundary returning undefined (${bootCase.name})`,
        )
        assert.ok(
            observed.readinessSwitches.value.includes('lang=EN'),
            `app.commandLine.appendSwitch('lang','EN') must already be set when app.whenReady() runs (${bootCase.locale}): ${JSON.stringify(observed.readinessSwitches.value)}`,
        )
        assert.ok(
            observed.events.indexOf('appendSwitch:lang=EN') < observed.events.indexOf('whenReady:begin'),
            `appendSwitch('lang','EN') must run before app.whenReady() (${bootCase.locale})`,
        )
        assert.equal(observed.writes.length, 1, `readiness manifest must be written exactly once (${bootCase.locale})`)
        const manifest = JSON.parse(observed.writes[0].data)
        assert.equal(manifest.mode, 'thin')
        assert.equal(
            manifest.locale,
            bootCase.locale,
            `readiness manifest locale must be app.getLocale() (${bootCase.locale}), not a hardcoded English value`,
        )
        assertTooltipTextMetrics(manifest, observed.canvas, bootCase)
    }

    console.log('Caption fixture: CLI errors, safe output, source identity, isolated installation, reference boot lang/locale, and canvas tooltip text metrics diagnostics passed')
} finally {
    fs.rmSync(directory, { recursive: true, force: true })
}
