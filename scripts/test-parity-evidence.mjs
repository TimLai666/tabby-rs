import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { createEvidenceReport, executeChecks, selectParityChecks } from './collect-parity-evidence.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const packageJson = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'))

const parity = {
    features: [
        { platforms: ['linux'], tests: { automated: ['alpha', 'shared'] } },
        { platforms: ['windows'], tests: { automated: ['shared', 'beta'] } },
    ],
}
const selected = selectParityChecks({ parity, scripts: { 'test:alpha': 'fixture', 'test:shared': 'fixture' }, platform: 'linux' })
assert.deepEqual(selected.checks, ['alpha', 'shared'])
assert.deepEqual(selected.expectedChecks, ['alpha', 'shared'])
assert.deepEqual(selected.missingScripts, [])
assert.deepEqual(selectParityChecks({ parity, scripts: {}, requestedChecks: ['alpha', 'unknown'] }).missingScripts, ['alpha', 'unknown'])
assert.deepEqual(selectParityChecks({ parity, scripts: {}, requestedChecks: ['alpha', 'unknown'] }).unknownChecks, ['unknown'])

const executed = []
const results = await executeChecks(['alpha', 'shared'], {
    runCheck: async name => {
        executed.push(name)
        return {
            name,
            command: `fixture ${name}`,
            status: name === 'alpha' ? 'passed' : 'failed',
            passed: name === 'alpha',
            exitCode: name === 'alpha' ? 0 : 2,
            durationMs: 4,
            stdout: { bytes: 5, sha256: 'a'.repeat(64) },
            stderr: { bytes: 7, sha256: 'b'.repeat(64) },
        }
    },
})
assert.deepEqual(executed, ['alpha', 'shared'])
const failedReport = createEvidenceReport({
    results,
    platform: 'linux',
    target: 'fixture-target',
    sourceRevision: 'fixture-revision',
    generatedAt: '2026-01-01T00:00:00.000Z',
    expectedChecks: ['alpha', 'shared'],
    platformRequiredChecks: ['local-shell', 'manual-check'],
    unverifiedRequiredChecks: ['manual-check'],
})
assert.equal(failedReport.passed, false)
assert.match(failedReport.failures[0], /shared/)
assert.equal(Object.hasOwn(failedReport.checks[0], 'stdoutText'), false)
assert.equal(failedReport.checks[0].stdout.sha256.length, 64)
assert.deepEqual(failedReport.platformRequiredChecks, ['local-shell', 'manual-check'])
assert.equal(failedReport.unverifiedRequiredChecks[0], 'manual-check')

const automatedOnlyReport = createEvidenceReport({
    results: [{ name: 'alpha', passed: true, status: 'passed', exitCode: 0 }],
    sourceRevision: 'fixture-revision',
    expectedChecks: ['alpha'],
    platformRequiredChecks: ['local-shell', 'manual-check'],
    unverifiedRequiredChecks: ['manual-check'],
})
assert.equal(automatedOnlyReport.passed, true)
assert.deepEqual(automatedOnlyReport.failures, [])

const incompleteReport = createEvidenceReport({
    results: [{ name: 'alpha', passed: true, status: 'passed', exitCode: 0 }],
    expectedChecks: ['alpha', 'shared'],
})
assert.equal(incompleteReport.passed, false)
assert.match(incompleteReport.failures[0], /missing expected platform checks: shared/)

const passingReport = createEvidenceReport({
    results: [{ name: 'alpha', passed: true, status: 'passed', exitCode: 0 }],
    sourceRevision: 'fixture-revision',
    generatedAt: '2026-01-01T00:00:00.000Z',
})
assert.equal(passingReport.passed, true)
assert.equal(passingReport.kind, 'tabby-rs-parity-automated-evidence')
assert.equal(typeof packageJson.scripts['test:parity-evidence'], 'string')
console.log('Parity automated evidence fixtures passed')
