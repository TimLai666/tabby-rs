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
    console.log('Caption fixture: CLI errors, safe output, production source identity, and isolated installation passed')
} finally {
    fs.rmSync(directory, { recursive: true, force: true })
}
