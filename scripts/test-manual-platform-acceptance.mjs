import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import yaml from 'js-yaml'

import { validateManualPlatformAcceptance } from './check-manual-platform-acceptance.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const matrix = yaml.load(fs.readFileSync(path.join(root, 'parity/platform-matrix.yaml'), 'utf8'))
const platformEntry = matrix.platforms.find(platform => platform.id === 'windows-x64')
const requiredChecks = platformEntry.requiredChecks
const validRecord = {
    schemaVersion: 1,
    kind: 'tabby-rs-manual-platform-acceptance',
    sourceRevision: '0123456789abcdef0123456789abcdef01234567',
    platform: platformEntry.id,
    architecture: 'x86_64',
    target: platformEntry.target,
    environment: {
        os: 'Windows 11 build fixture',
        webview: 'WebView2 fixture',
        toolchain: 'rustc fixture',
        testedAt: '2026-08-21T00:00:00Z',
    },
    checks: requiredChecks.map(id => ({
        id,
        status: 'passed',
        steps: ['verified ' + id],
        evidence: ['manual/' + platformEntry.id + '/' + id + '.txt'],
    })),
    artifacts: [{ path: 'artifacts/Tabby-RS-setup.exe', sha256: 'a'.repeat(64) }],
}

const valid = validateManualPlatformAcceptance(validRecord, {
    platformEntry,
    expectedRevision: validRecord.sourceRevision,
    expectedArchitecture: validRecord.architecture,
    expectedTarget: validRecord.target,
})
assert.deepEqual(valid, { passed: true, failures: [] })

const missingCheck = structuredClone(validRecord)
missingCheck.checks = missingCheck.checks.slice(1)
const missingResult = validateManualPlatformAcceptance(missingCheck, { platformEntry })
assert.ok(missingResult.failures.includes('missing manual platform check: ' + requiredChecks[0]))

const failedCheck = structuredClone(validRecord)
failedCheck.checks[0].status = 'not-run'
const failedResult = validateManualPlatformAcceptance(failedCheck, { platformEntry })
assert.ok(failedResult.failures.includes('manual platform check ' + requiredChecks[0] + ' is not-run'))

const unsafeEvidence = structuredClone(validRecord)
unsafeEvidence.checks[0].evidence = ['../secret.txt']
const unsafeResult = validateManualPlatformAcceptance(unsafeEvidence, { platformEntry })
assert.ok(unsafeResult.failures.includes('manual platform check ' + requiredChecks[0] + ' has invalid evidence paths'))

const wrongRevision = validateManualPlatformAcceptance(validRecord, {
    platformEntry,
    expectedRevision: 'f'.repeat(40),
})
assert.ok(wrongRevision.failures.includes('sourceRevision must match ' + 'f'.repeat(40)))

console.log('Manual platform acceptance contract passed')
