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
const installerSmoke = path.join(work, 'installer-smoke.json')
const metadata = path.join(work, 'tabby-rs-metadata.json')
const expectedRevision = '0123456789abcdef0123456789abcdef01234567'

fs.writeFileSync(bundleAudit, '{ broken')
fs.writeFileSync(licenseReport, JSON.stringify({ passed: true, sourceRevision: 'fedcba9876543210fedcba9876543210fedcba98' }))
fs.writeFileSync(installerSmoke, JSON.stringify({
    passed: true,
    platform: 'windows',
    planOnly: true,
    operations: [{ action: 'install' }],
}))
fs.writeFileSync(metadata, JSON.stringify({
    dependencyLocks: {
        'yarn.lock': '0'.repeat(64),
        'src-tauri/Cargo.lock': '1'.repeat(64),
    },
    toolchain: {},
}))

await assert.rejects(
    execFileAsync(process.execPath, [gate,
        '--bundle-audit', bundleAudit,
        '--license-report', licenseReport,
        '--installer-smoke', installerSmoke,
        '--platform', 'windows',
        '--source-revision', expectedRevision,
        '--output', output,
    ], { cwd: root }),
)

const report = JSON.parse(fs.readFileSync(output, 'utf8'))
assert.equal(report.passed, false)
assert.equal(report.sourceRevision, expectedRevision)
assert.ok(report.failures.some(error => error.startsWith('invalid bundle audit ')))
assert.ok(report.failures.includes(`license report sourceRevision must match ${expectedRevision}`))
assert.ok(report.failures.includes('installer smoke must execute install, launch, and uninstall operations'))
assert.ok(report.failures.includes('installer smoke is missing launch operation'))
assert.ok(report.failures.includes('installer smoke is missing uninstall operation'))

const passingBundleAudit = path.join(work, 'passing-bundle-audit.json')
const passingOutput = path.join(work, 'passing-release-gate.json')
fs.writeFileSync(passingBundleAudit, JSON.stringify({
    passed: true,
    sourceRevision: expectedRevision,
    platform: 'windows',
    arch: 'x86_64',
    target: 'x86_64-pc-windows-msvc',
}))
await assert.rejects(
    execFileAsync(process.execPath, [gate,
        '--bundle-audit', passingBundleAudit,
        '--license-report', licenseReport,
        '--installer-smoke', installerSmoke,
        '--platform', 'windows',
        '--source-revision', expectedRevision,
        '--output', passingOutput,
    ], { cwd: root }),
)
const mismatchReport = JSON.parse(fs.readFileSync(passingOutput, 'utf8'))
assert.ok(mismatchReport.failures.includes('release metadata dependency lock hash does not match checkout: yarn.lock'))
assert.ok(mismatchReport.failures.includes('release metadata dependency lock hash does not match checkout: src-tauri/Cargo.lock'))

console.log('Release gate contract fixtures passed')
