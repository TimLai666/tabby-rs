#!/usr/bin/env node

import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import vm from 'node:vm'
import webpack from 'webpack'
import rendererConfig from '../app/webpack.config.tauri.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const bellPath = path.join(root, 'tabby-terminal/src/bell.ogg')
const originalBell = execFileSync('git', [
    'show', '14e2d60b9b6dee84a53c37f05eefeb803787de04:tabby-terminal/src/bell.ogg',
], { cwd: root })
assert.deepEqual(fs.readFileSync(bellPath), originalBell, 'retain the fixed upstream bell bytes')

const temporaryDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-bell-'))
try {
    const config = rendererConfig()
    const entry = path.join(temporaryDirectory, 'entry.cjs')
    const componentPath = path.join(root, 'tabby-terminal/src/api/baseTerminalTab.component.ts')
    const component = fs.readFileSync(componentPath, 'utf8')
    const request = component.match(/this\.bellPlayer\.src = require\('([^']+)'\)/)?.[1]
    assert.ok(request, 'exercise the shared terminal audio import')
    fs.writeFileSync(entry, `module.exports = require(${JSON.stringify(request)})\n`)
    const rules = config.module.rules.filter(rule => [bellPath, 'bell.wav'].some(file => rule.test?.test(file)))
    assert.ok(rules.length, 'use the actual renderer audio rules')
    const compiler = webpack({
        mode: 'production',
        target: 'web',
        entry,
        output: {
            path: temporaryDirectory,
            filename: 'bell.cjs',
            library: { type: 'commonjs2' },
            publicPath: './',
        },
        // The fixture issuer is in tmp; model the real shared component's
        // relative target, then apply the native renderer's actual aliases.
        resolve: { alias: { [request + '$']: path.resolve(path.dirname(componentPath), request), ...config.resolve.alias } },
        module: { rules },
    })
    try {
        await new Promise((resolve, reject) => compiler.run((error, stats) => {
            if (error) { reject(error); return }
            if (stats.hasErrors()) { reject(new Error(stats.toString('errors-only'))); return }
            resolve()
        }))
    } finally {
        await new Promise((resolve, reject) => compiler.close(error => error ? reject(error) : resolve()))
    }
    const bundle = fs.readFileSync(path.join(temporaryDirectory, 'bell.cjs'), 'utf8')
    for (const capability of ['probably', 'maybe', '']) {
        const target = { exports: {} }
        vm.runInNewContext(bundle, {
            module: target,
            exports: target.exports,
            document: {
                createElement: tag => {
                    assert.equal(tag, 'audio')
                    return { canPlayType: type => type === 'audio/ogg; codecs="vorbis"' ? capability : '' }
                },
            },
        })
        const bellURL = target.exports
        assert.ok(!/^(?:data|blob|https?):/.test(bellURL), 'native CSP requires a same-origin bell resource')
        const emittedBell = path.resolve(temporaryDirectory, bellURL)
        assert.ok(emittedBell.startsWith(temporaryDirectory + path.sep), 'resource stays inside the output directory')
        const bytes = fs.readFileSync(emittedBell)
        if (capability) {
            assert.deepEqual(bytes, originalBell, 'Ogg-capable WebViews retain the exact upstream audio')
        } else {
            assert.match(bellURL, /\.wav$/, 'WebViews without Ogg support need the PCM bell')
            assert.equal(bytes.toString('ascii', 0, 4), 'RIFF')
            assert.equal(bytes.toString('ascii', 8, 12), 'WAVE')
            assert.equal(bytes.toString('ascii', 12, 16), 'fmt ')
            assert.equal(bytes.readUInt32LE(16), 16)
            assert.equal(bytes.readUInt16LE(20), 1, 'uncompressed PCM')
            assert.equal(bytes.readUInt16LE(22), 1, 'mono')
            assert.equal(bytes.readUInt32LE(24), 48000)
            assert.equal(bytes.readUInt16LE(34), 16)
            assert.equal(bytes.toString('ascii', 36, 40), 'data')
            assert.equal(bytes.readUInt32LE(40), 15936 * 2)
            assert.equal(bytes.length, 44 + 15936 * 2)
            assert.equal(createHash('sha256').update(bytes.subarray(44)).digest('hex'),
                'f4893d606c1cde6f519a72abc0296b57a0036024f8b733d864a1ec48c6a2276c',
                'PCM samples match the fixed upstream Ogg decoded by Electron 38.8.6')
        }
    }
    const csp = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8')).app.security.csp
    assert.match(csp, /(?:^|;\s*)default-src 'self'(?:;|$)/)
    assert.doesNotMatch(csp, /(?:^|;\s*)media-src[^;]*(?:data:|blob:|\*)/, 'do not relax the native content policy')
    console.log('Terminal bell resource passed: upstream Ogg, matching PCM fallback, same-origin emission, unchanged media policy')
} finally {
    fs.rmSync(temporaryDirectory, { recursive: true, force: true })
}
