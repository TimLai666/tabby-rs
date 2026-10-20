#!/usr/bin/env node

import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
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
    fs.writeFileSync(entry, `module.exports = require(${JSON.stringify(bellPath)})\n`)
    const rules = config.module.rules.filter(rule => rule.test?.test(bellPath))
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
    const { createRequire } = await import('node:module')
    const bellURL = createRequire(import.meta.url)(path.join(temporaryDirectory, 'bell.cjs'))
    assert.ok(!/^(?:data|blob|https?):/.test(bellURL), 'native CSP requires a same-origin bell resource')
    const emittedBell = path.resolve(temporaryDirectory, bellURL)
    assert.ok(emittedBell.startsWith(temporaryDirectory + path.sep), 'resource stays inside the output directory')
    assert.deepEqual(fs.readFileSync(emittedBell), originalBell, 'bundled bell retains the exact upstream audio')
    const csp = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8')).app.security.csp
    assert.match(csp, /(?:^|;\s*)default-src 'self'(?:;|$)/)
    assert.doesNotMatch(csp, /(?:^|;\s*)media-src[^;]*(?:data:|blob:|\*)/, 'do not relax the native content policy')
    console.log('Terminal bell resource passed: fixed upstream bytes, same-origin emission, unchanged media policy')
} finally {
    fs.rmSync(temporaryDirectory, { recursive: true, force: true })
}
