import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'

const execFileAsync = promisify(execFile)
const root = fileURLToPath(new URL('..', import.meta.url))
const script = path.join(root, 'scripts', 'aggregate-release-gate.mjs')
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-aggregate-gate-test-'))
const revision = '0123456789abcdef0123456789abcdef01234567'
const targets = ['x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu']

for (const target of targets) {
    const directory = path.join(work, target)
    fs.mkdirSync(directory, { recursive: true })
    fs.writeFileSync(path.join(directory, 'release-gate.json'), JSON.stringify({ schemaVersion: 1, passed: true, sourceRevision: revision, failures: [] }))
    fs.writeFileSync(path.join(directory, 'tabby-rs-metadata.json'), JSON.stringify({
        revision,
        channel: 'stable',
        version: '0.1.0',
        target,
        platform: target.includes('windows') ? 'windows' : 'linux',
        arch: 'x86_64',
    }))
}

const output = path.join(work, 'aggregate.json')
await execFileAsync(process.execPath, [script, work,
    '--source-revision', revision,
    '--channel', 'stable',
    '--expected-targets', JSON.stringify(targets),
    '--output', output,
], { cwd: root })
const report = JSON.parse(fs.readFileSync(output, 'utf8'))
assert.equal(report.passed, true)
assert.equal(report.version, '0.1.0')
assert.deepEqual(report.targets.map(target => target.target).sort(), [...targets].sort())

const invalidWork = path.join(work, 'invalid')
fs.mkdirSync(invalidWork, { recursive: true })
fs.writeFileSync(path.join(invalidWork, 'release-gate.json'), JSON.stringify({ passed: true, sourceRevision: revision, failures: [] }))
fs.writeFileSync(path.join(invalidWork, 'tabby-rs-metadata.json'), JSON.stringify({
    channel: 'stable',
    target: targets[0],
    platform: 'windows',
    arch: 'x86_64',
}))
const invalidOutput = path.join(work, 'invalid-aggregate.json')
await assert.rejects(
    execFileAsync(process.execPath, [script, path.join(work, 'invalid'),
        '--source-revision', revision,
        '--channel', 'stable',
        '--expected-targets', JSON.stringify([targets[0]]),
        '--output', invalidOutput,
    ], { cwd: root }),
)
const invalidReport = JSON.parse(fs.readFileSync(invalidOutput, 'utf8'))
assert.equal(invalidReport.passed, false)
assert.ok(invalidReport.failures.includes('release-gate.json: release gate schema version is invalid'))
assert.ok(invalidReport.failures.includes('release-gate.json: release version is missing'))
assert.ok(invalidReport.failures.includes('release-gate.json: release metadata revision is missing'))

const emptyOutput = path.join(work, 'empty-aggregate.json')
await assert.rejects(
    execFileAsync(process.execPath, [script, path.join(work, 'empty-input'),
        '--expected-targets', '[]',
        '--output', emptyOutput,
    ], { cwd: root }),
)
const emptyReport = JSON.parse(fs.readFileSync(emptyOutput, 'utf8'))
assert.equal(emptyReport.passed, false)
assert.ok(emptyReport.failures.includes('expected targets must be a non-empty array of strings'))

console.log('Aggregate release gate fixtures passed')
