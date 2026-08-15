import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { execFileSync, spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-parity-report-test-'))
const features = path.join(work, 'features.yaml')
const platforms = path.join(work, 'platforms.yaml')
const output = path.join(work, 'parity-report.json')
const htmlOutput = path.join(work, 'parity-report.html')

fs.writeFileSync(features, `baseline:\n  repository: 'example/<script>'\n  commit: abc123\n  version: 1.0.0\nfeatures:\n  - id: shell\n    title: Local shell\n    issues: [7]\n    platforms: [linux]\n    tests:\n      automated: [fixture-test]\n    status: pending\n`)
fs.writeFileSync(platforms, `platforms:\n  - id: linux\n    runner: ubuntu\n    target: x86_64-unknown-linux-gnu\n    requiredChecks: [local-shell]\n    status: pending\n`)

const pending = spawnSync(process.execPath, [
    path.join(root, 'scripts/compare-parity.mjs'),
    '--features', features,
    '--platforms', platforms,
    '--output', output,
], { cwd: root, encoding: 'utf8' })
assert.equal(pending.status, 1)
assert.match(pending.stdout, /"passed": false/)
const pendingReport = JSON.parse(fs.readFileSync(output, 'utf8'))
assert.deepEqual(pendingReport.features.pending, ['shell'])
assert.deepEqual(pendingReport.platforms.pending, ['linux'])
assert.equal(pendingReport.manifests.features.path, 'parity/features.yaml')
assert.equal(pendingReport.manifests.platforms.path, 'parity/platform-matrix.yaml')
assert.equal(pendingReport.manifests.features.ids[0], 'shell')
assert.equal(pendingReport.manifests.platforms.ids[0], 'linux')

const reportOnly = spawnSync(process.execPath, [
    path.join(root, 'scripts/compare-parity.mjs'),
    '--features', features,
    '--platforms', platforms,
    '--report-only',
    '--output', output,
    '--html-output', htmlOutput,
], { cwd: root, encoding: 'utf8' })
assert.equal(reportOnly.status, 0)
assert.match(reportOnly.stdout, /"passed": false/)
const pendingHtml = fs.readFileSync(htmlOutput, 'utf8')
assert.match(pendingHtml, /<title>Tabby RS parity report<\/title>/)
assert.match(pendingHtml, /feature shell is pending/)
assert.match(pendingHtml, /example\/&lt;script&gt;/)

fs.writeFileSync(features, `baseline:\n  repository: example/tabby\n  commit: abc123\n  version: 1.0.0\nfeatures:\n  - id: shell\n    title: Local shell\n    issues: [7]\n    platforms: [linux]\n    tests:\n      automated: fixture-test\n    status: passed\n    evidence: [fixture-test]\n`)
fs.writeFileSync(platforms, `platforms:\n  - id: linux\n    runner: ubuntu\n    target: x86_64-unknown-linux-gnu\n    requiredChecks: [local-shell]\n    status: accepted-difference\n`)

const malformed = spawnSync(process.execPath, [
    path.join(root, 'scripts/compare-parity.mjs'),
    '--features', features,
    '--platforms', platforms,
], { cwd: root, encoding: 'utf8' })
assert.equal(malformed.status, 1)
assert.match(malformed.stdout, /feature shell has invalid automated or manual test list/)
assert.match(malformed.stdout, /platform linux has no evidence for accepted-difference/)
assert.match(malformed.stdout, /platform linux accepted-difference has no reason/)

fs.writeFileSync(features, `baseline:\n  repository: example/tabby\n  commit: abc123\n  version: 1.0.0\nfeatures:\n  - id: shell\n    title: Local shell\n    issues: [7]\n    platforms: [linux]\n    tests:\n      automated: [fixture-test]\n    status: passed\n    evidence: [fixture-test]\n`)
fs.writeFileSync(platforms, `platforms:\n  - id: linux\n    runner: ubuntu\n    target: x86_64-unknown-linux-gnu\n    requiredChecks: [local-shell]\n    status: passed\n    evidence: [fixture-test]\n`)

const passed = execFileSync(process.execPath, [
    path.join(root, 'scripts/compare-parity.mjs'),
    '--features', features,
    '--platforms', platforms,
    '--html-output', htmlOutput,
], { cwd: root, encoding: 'utf8' })
assert.match(passed, /"passed": true/)
const passedHtml = fs.readFileSync(htmlOutput, 'utf8')
assert.match(passedHtml, /<p>No parity failures\.<\/p>/)

console.log('Parity report fixtures passed')
