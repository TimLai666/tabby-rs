import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'

const execFileAsync = promisify(execFile)
const root = path.resolve(new URL('..', import.meta.url).pathname)
const script = path.join(root, 'scripts', 'aggregate-release-gate.mjs')
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-aggregate-gate-test-'))
const revision = '0123456789abcdef0123456789abcdef01234567'
const targets = ['x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu']

for (const target of targets) {
    const directory = path.join(work, target)
    fs.mkdirSync(directory, { recursive: true })
    fs.writeFileSync(path.join(directory, 'release-gate.json'), JSON.stringify({ passed: true, sourceRevision: revision, failures: [] }))
    fs.writeFileSync(path.join(directory, 'tabby-rs-metadata.json'), JSON.stringify({
        revision,
        channel: 'stable',
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
assert.deepEqual(report.targets.map(target => target.target).sort(), [...targets].sort())

console.log('Aggregate release gate fixtures passed')
