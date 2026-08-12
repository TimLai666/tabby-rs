import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'

const execFileAsync = promisify(execFile)
const root = path.resolve(new URL('..', import.meta.url).pathname)
const gate = path.join(root, 'scripts', 'check-release-gate.mjs')
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-release-gate-test-'))
const output = path.join(work, 'release-gate.json')
const bundleAudit = path.join(work, 'bundle-audit.json')
const licenseReport = path.join(work, 'license-report.json')

fs.writeFileSync(bundleAudit, '{ broken')
fs.writeFileSync(licenseReport, '{ broken')

await assert.rejects(
    execFileAsync(process.execPath, [gate,
        '--bundle-audit', bundleAudit,
        '--license-report', licenseReport,
        '--output', output,
    ], { cwd: root }),
)

const report = JSON.parse(fs.readFileSync(output, 'utf8'))
assert.equal(report.passed, false)
assert.ok(report.failures.some(error => error.startsWith('invalid bundle audit ')))
assert.ok(report.failures.some(error => error.startsWith('invalid license report ')))

console.log('Release gate contract fixtures passed')
