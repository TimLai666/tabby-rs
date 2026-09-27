import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import vm from 'node:vm'
import readline from 'node:readline'
import { PassThrough } from 'node:stream'
import { createRequire } from 'node:module'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import webpack from 'webpack'
import configFactory from '../app/webpack.config.tauri.mjs'

const require = createRequire(import.meta.url)
const reference = require('../app/src/shims/readline/provenance.json')
if (!process.versions.electron) {
    const child = spawnSync(require('electron'), [fileURLToPath(import.meta.url)], {
        env: { ...process.env, ELECTRON_RUN_AS_NODE: '1' }, stdio: 'inherit',
    })
    if (child.error) throw child.error
    process.exit(child.status ?? 1)
}
assert.equal(process.versions.electron, reference.electron)
assert.equal(process.versions.node, reference.node)
assert.equal(process.versions.icu, reference.icu)

// Exercise the actual browser resolution without letting Node supply readline.
const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'tabby-readline-test-'))
const originalTerm = process.env.TERM
process.env.TERM = 'xterm-256color'
try {
    const entry = path.join(temp, 'entry.cjs')
    await fs.writeFile(entry, `module.exports = { readline: require('readline'),
        Buffer: require('buffer').Buffer, PassThrough: require('stream').PassThrough, Spinner: require('cli-spinner').Spinner }`)
    const config = configFactory()
    const compiler = webpack({
        ...config, entry, devtool: false, module: { rules: [] },
        plugins: config.plugins.filter(p => p.constructor.name !== 'AngularWebpackPlugin'),
        output: { path: temp, filename: 'bundle.cjs', library: { type: 'commonjs2' }, publicPath: '' },
    })
    const stats = await new Promise((resolve, reject) => compiler.run((error, result) => error ? reject(error) : resolve(result)))
    await new Promise((resolve, reject) => compiler.close(error => error ? reject(error) : resolve()))
    assert.equal(stats.hasErrors(), false, stats.toString({ all: false, errors: true }))
    const context = { module: { exports: {} }, console, setTimeout, clearTimeout, setInterval, clearInterval,
        queueMicrotask, TextDecoder, TextEncoder, AbortController, window: {} }
    vm.runInNewContext(await fs.readFile(path.join(temp, 'bundle.cjs'), 'utf8'), context)
    const browser = context.module.exports
    const output = new browser.PassThrough()
    let bytes = ''
    output.on('data', chunk => { bytes += chunk.toString() })
    browser.readline.clearLine(output, 0)
    browser.readline.cursorTo(output, 0)
    assert.equal(bytes, '\x1b[2K\x1b[1G')
    const spinner = new browser.Spinner({ text: 'Connecting', stream: output })
    spinner.start()
    spinner.stop(true)

    async function run (implementation, chunks, columns = 12) {
        const Stream = implementation === browser.readline ? browser.PassThrough : PassThrough
        const Bytes = implementation === browser.readline ? browser.Buffer : Buffer
        const input = new Stream()
        const output = new Stream()
        output.columns = columns
        const lines = []
        let screen = ''
        output.on('data', chunk => { screen += chunk.toString() })
        const rl = implementation.createInterface({ input, output, terminal: true, prompt: '> ' })
        rl.on('line', line => { lines.push(line); rl.prompt(true) })
        rl.prompt()
        for (const chunk of chunks) {
            input.write(Bytes.from(chunk))
            await new Promise(resolve => setTimeout(resolve, 0))
        }
        output.emit('resize')
        const state = { lines, screen, line: rl.line, cursor: rl.cursor, history: [...rl.history] }
        rl.close()
        assert.equal(input.listenerCount('keypress'), 0)
        assert.equal(output.listenerCount('resize'), 0)
        input.destroy()
        output.destroy()
        return state
    }
    const scenarios = [
        ['hello\r'], ['one\r', 'two\r', '\x1b[A', '\r'],
        ['abcd', '\x1b[D\x1b[D', '\x7f', 'X', '\x1b[3~', '\r'],
        ['abc', '\x01', 'X', '\x05', 'Y', '\r'],
        ['hello world', '\x17', 'new', '\r'],
        ['abc', '\x15', '\x19', '\r'], ['abc', '\x1f', '\r'],
        ['台灣🙂', '\x1b[D', '\x7f', '\r'],
        [Buffer.from([0xe5]), Buffer.from([0x8f, 0xb0]), '\r'],
        ['a\tb\r'], ['one\r\ntwo\n'], ['a'.repeat(50), '\x01', '\x0b', '\r'],
    ]
    for (const [index, scenario] of scenarios.entries()) {
        assert.deepEqual(await run(browser.readline, scenario), await run(readline, scenario), `scenario ${index}`)
    }
    for (const platform of ['windows', 'macos']) {
        context.window.__TABBY_PLATFORM__ = platform
        const input = new browser.PassThrough()
        const output = new browser.PassThrough()
        const lines = []
        const editor = browser.readline.createInterface({ input, output, terminal: true })
        editor.on('line', line => lines.push(line))
        input.write(browser.Buffer.from('ab\x1acd\r'))
        await new Promise(resolve => setTimeout(resolve, 0))
        assert.deepEqual(lines, ['abcd'], `${platform} Ctrl+Z leaves the editor usable`)
        editor.close()
        input.destroy()
        output.destroy()
    }
    const { getStringWidth } = require('../app/src/shims/readline/runtime.cjs')
    const icu = process.binding('icu')
    for (let cp = 0; cp <= 0x10ffff; cp++) {
        const character = String.fromCodePoint(cp)
        assert.equal(getStringWidth(character, false), icu.getStringWidth(character.normalize('NFC')), `width U+${cp.toString(16)}`)
    }
    console.log(`Browser readline: Electron ${reference.electron}, spinner, ${scenarios.length} editing scenarios, Windows/macOS Ctrl+Z, and 1114112 code point widths passed`)
} finally {
    if (originalTerm === undefined) delete process.env.TERM
    else process.env.TERM = originalTerm
    await fs.rm(temp, { recursive: true, force: true })
}
