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
const dependencyAudit = path.join(work, 'dependency-audit.json')
const licenseReport = path.join(work, 'license-report.json')
const installerSmoke = path.join(work, 'installer-smoke.json')
const parityReport = path.join(work, 'parity-report.json')
const parityEvidence = path.join(work, 'parity-automated-evidence.json')
const passingParityReport = path.join(work, 'passing-parity-report.json')
const malformedPassingParityReport = path.join(work, 'malformed-passing-parity-report.json')
const metadata = path.join(work, 'tabby-rs-metadata.json')
const expectedRevision = '0123456789abcdef0123456789abcdef01234567'

fs.writeFileSync(bundleAudit, '{ broken')
fs.writeFileSync(dependencyAudit, JSON.stringify({
    schemaVersion: 1,
    policy: 'tauri-release',
    manifests: ['package.json', 'app/package.json'],
    passed: true,
    findings: [],
}))
fs.writeFileSync(licenseReport, JSON.stringify({ passed: true, sourceRevision: 'fedcba9876543210fedcba9876543210fedcba98' }))
fs.writeFileSync(installerSmoke, JSON.stringify({
    passed: true,
    platform: 'windows',
    planOnly: true,
    operations: [{ action: 'install' }],
}))
fs.writeFileSync(parityReport, JSON.stringify({
    schemaVersion: 1,
    passed: false,
    features: { total: 1, statuses: { pending: 1 }, pending: ['fixture.feature'] },
    platforms: { total: 1, statuses: { pending: 1 }, pending: ['fixture-platform'] },
    failures: ['feature fixture.feature is pending', 'platform fixture-platform is pending'],
}))
fs.writeFileSync(parityEvidence, JSON.stringify({
    schemaVersion: 1,
    kind: 'tabby-rs-parity-automated-evidence',
    sourceRevision: expectedRevision,
    platform: 'windows',
    arch: 'x86_64',
    target: null,
    expectedChecks: ['fixture-check'],
    checks: [{ name: 'fixture-check', passed: true }],
    passed: true,
}))
fs.writeFileSync(passingParityReport, JSON.stringify({
    schemaVersion: 1,
    passed: true,
    features: { total: 1, statuses: { passed: 1 }, pending: [] },
    platforms: { total: 1, statuses: { passed: 1 }, pending: [] },
    failures: [],
}))
fs.writeFileSync(malformedPassingParityReport, JSON.stringify({
    schemaVersion: 1,
    passed: true,
    features: { total: 1, statuses: { passed: 1 }, pending: ['fixture.feature'] },
    platforms: { total: 1, statuses: { passed: 1 }, pending: [] },
    failures: [],
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
        '--dependency-audit', dependencyAudit,
        '--license-report', licenseReport,
        '--installer-smoke', installerSmoke,
        '--parity-report', parityReport,
        '--parity-evidence', parityEvidence,
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
assert.ok(report.failures.includes('parity report did not pass'))
assert.ok(report.failures.includes('parity automated evidence expected checks do not match parity manifest'))
assert.ok(report.failures.includes('parity automated evidence has no platform required checks'))

const missingUserDataEvidenceSmoke = path.join(work, 'missing-user-data-evidence-smoke.json')
const missingUserDataEvidenceOutput = path.join(work, 'missing-user-data-evidence-release-gate.json')
fs.writeFileSync(missingUserDataEvidenceSmoke, JSON.stringify({
    passed: true,
    platform: 'windows',
    planOnly: false,
    operations: [
        { action: 'install' },
        { action: 'launch', identity: { userDataPreserved: false } },
        { action: 'uninstall' },
    ],
}))
await assert.rejects(
    execFileAsync(process.execPath, [gate,
        '--bundle-audit', bundleAudit,
        '--dependency-audit', dependencyAudit,
        '--license-report', licenseReport,
        '--installer-smoke', missingUserDataEvidenceSmoke,
        '--parity-report', parityReport,
        '--parity-evidence', parityEvidence,
        '--platform', 'windows',
        '--source-revision', expectedRevision,
        '--output', missingUserDataEvidenceOutput,
    ], { cwd: root }),
)
const missingUserDataEvidenceReport = JSON.parse(fs.readFileSync(missingUserDataEvidenceOutput, 'utf8'))
assert.ok(missingUserDataEvidenceReport.failures.includes('installer smoke launch did not verify user data preservation'))

const passingBundleAudit = path.join(work, 'passing-bundle-audit.json')
const failingDependencyAudit = path.join(work, 'failing-dependency-audit.json')
const dependencyFailureOutput = path.join(work, 'dependency-failure-release-gate.json')
fs.writeFileSync(passingBundleAudit, JSON.stringify({
    passed: true,
    sourceRevision: expectedRevision,
    platform: 'windows',
    arch: 'x86_64',
    target: 'x86_64-pc-windows-msvc',
}))
fs.writeFileSync(failingDependencyAudit, JSON.stringify({
    schemaVersion: 1,
    policy: 'tauri-release',
    manifests: ['package.json', 'app/package.json'],
    passed: false,
    findings: [{ rule: 'electron-runtime-dependency' }],
}))
await assert.rejects(
    execFileAsync(process.execPath, [gate,
        '--bundle-audit', passingBundleAudit,
        '--dependency-audit', failingDependencyAudit,
        '--license-report', licenseReport,
        '--installer-smoke', installerSmoke,
        '--parity-report', parityReport,
        '--platform', 'windows',
        '--source-revision', expectedRevision,
        '--output', dependencyFailureOutput,
    ], { cwd: root }),
)
const dependencyFailureReport = JSON.parse(fs.readFileSync(dependencyFailureOutput, 'utf8'))
assert.ok(dependencyFailureReport.failures.includes('Tauri dependency audit did not pass'))

const malformedDependencyAudit = path.join(work, 'malformed-dependency-audit.json')
const malformedDependencyOutput = path.join(work, 'malformed-dependency-release-gate.json')
fs.writeFileSync(malformedDependencyAudit, JSON.stringify({ passed: true, findings: [] }))
await assert.rejects(
    execFileAsync(process.execPath, [gate,
        '--bundle-audit', passingBundleAudit,
        '--dependency-audit', malformedDependencyAudit,
        '--license-report', licenseReport,
        '--installer-smoke', installerSmoke,
        '--parity-report', parityReport,
        '--platform', 'windows',
        '--source-revision', expectedRevision,
        '--output', malformedDependencyOutput,
    ], { cwd: root }),
)
const malformedDependencyReport = JSON.parse(fs.readFileSync(malformedDependencyOutput, 'utf8'))
assert.ok(malformedDependencyReport.failures.includes('Tauri dependency audit schema version is invalid'))
assert.ok(malformedDependencyReport.failures.includes('Tauri dependency audit policy must be tauri-release'))
assert.ok(malformedDependencyReport.failures.includes('Tauri dependency audit has no manifest list'))

const strictPolicyAudit = path.join(work, 'strict-policy-dependency-audit.json')
const strictPolicyOutput = path.join(work, 'strict-policy-release-gate.json')
fs.writeFileSync(strictPolicyAudit, JSON.stringify({
    schemaVersion: 1,
    policy: 'strict',
    manifests: ['package.json', 'app/package.json'],
    passed: true,
    findings: [],
}))
await assert.rejects(
    execFileAsync(process.execPath, [gate,
        '--bundle-audit', passingBundleAudit,
        '--dependency-audit', strictPolicyAudit,
        '--license-report', licenseReport,
        '--installer-smoke', installerSmoke,
        '--parity-report', parityReport,
        '--platform', 'windows',
        '--source-revision', expectedRevision,
        '--output', strictPolicyOutput,
    ], { cwd: root }),
)
const strictPolicyReport = JSON.parse(fs.readFileSync(strictPolicyOutput, 'utf8'))
assert.ok(strictPolicyReport.failures.includes('Tauri dependency audit policy must be tauri-release'))

const mismatchOutput = path.join(work, 'mismatch-release-gate.json')
await assert.rejects(
    execFileAsync(process.execPath, [gate,
        '--bundle-audit', passingBundleAudit,
        '--dependency-audit', dependencyAudit,
        '--license-report', licenseReport,
        '--installer-smoke', installerSmoke,
        '--parity-report', passingParityReport,
        '--platform', 'windows',
        '--source-revision', expectedRevision,
        '--output', mismatchOutput,
    ], { cwd: root }),
)
const mismatchReport = JSON.parse(fs.readFileSync(mismatchOutput, 'utf8'))
assert.ok(mismatchReport.failures.includes('release metadata dependency lock hash does not match checkout: yarn.lock'))
assert.ok(mismatchReport.failures.includes('release metadata dependency lock hash does not match checkout: src-tauri/Cargo.lock'))
assert.ok(!mismatchReport.failures.includes('parity report did not pass'))

const malformedPassingParityOutput = path.join(work, 'malformed-passing-parity-release-gate.json')
await assert.rejects(
    execFileAsync(process.execPath, [gate,
        '--bundle-audit', passingBundleAudit,
        '--dependency-audit', dependencyAudit,
        '--license-report', licenseReport,
        '--installer-smoke', installerSmoke,
        '--parity-report', malformedPassingParityReport,
        '--platform', 'windows',
        '--source-revision', expectedRevision,
        '--output', malformedPassingParityOutput,
    ], { cwd: root }),
)
const malformedPassingParityGateReport = JSON.parse(fs.readFileSync(malformedPassingParityOutput, 'utf8'))
assert.ok(malformedPassingParityGateReport.failures.includes('parity report features has pending entries'))
assert.ok(malformedPassingParityGateReport.failures.includes('parity report has no manifest provenance'))

assert.ok(mismatchReport.failures.includes('missing parity automated evidence path'))

const weakBundleAudit = path.join(work, 'weak-bundle-audit.json')
const weakBundleOutput = path.join(work, 'weak-bundle-release-gate.json')
fs.writeFileSync(weakBundleAudit, JSON.stringify({ passed: true, sourceRevision: expectedRevision }))
await assert.rejects(
    execFileAsync(process.execPath, [gate,
        '--bundle-audit', weakBundleAudit,
        '--dependency-audit', dependencyAudit,
        '--license-report', licenseReport,
        '--installer-smoke', installerSmoke,
        '--parity-report', parityReport,
        '--parity-evidence', parityEvidence,
        '--platform', 'windows',
        '--source-revision', expectedRevision,
        '--output', weakBundleOutput,
    ], { cwd: root }),
)
const weakBundleGateReport = JSON.parse(fs.readFileSync(weakBundleOutput, 'utf8'))
assert.ok(weakBundleGateReport.failures.includes('bundle audit schema version is invalid'))
assert.ok(weakBundleGateReport.failures.includes('bundle audit has no file manifest'))
assert.ok(weakBundleGateReport.failures.includes('bundle audit target is missing'))

const weakLicenseReport = path.join(work, 'weak-license-report.json')
const weakLicenseOutput = path.join(work, 'weak-license-release-gate.json')
fs.writeFileSync(weakLicenseReport, JSON.stringify({ passed: true, sourceRevision: expectedRevision }))
await assert.rejects(
    execFileAsync(process.execPath, [gate,
        '--bundle-audit', passingBundleAudit,
        '--dependency-audit', dependencyAudit,
        '--license-report', weakLicenseReport,
        '--installer-smoke', installerSmoke,
        '--parity-report', parityReport,
        '--parity-evidence', parityEvidence,
        '--platform', 'windows',
        '--source-revision', expectedRevision,
        '--output', weakLicenseOutput,
    ], { cwd: root }),
)
const weakLicenseGateReport = JSON.parse(fs.readFileSync(weakLicenseOutput, 'utf8'))
assert.ok(weakLicenseGateReport.failures.includes('license report schema version is invalid'))
assert.ok(weakLicenseGateReport.failures.includes('license report has no LICENSE entry'))
assert.ok(weakLicenseGateReport.failures.includes('license report has no cargo dependencies'))

console.log('Release gate contract fixtures passed')
