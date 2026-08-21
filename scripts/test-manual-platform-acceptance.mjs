import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import yaml from 'js-yaml'

import { validateManualPlatformAcceptance } from './check-manual-platform-acceptance.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const matrix = yaml.load(fs.readFileSync(path.join(root, 'parity/platform-matrix.yaml'), 'utf8'))
const featuresDocument = yaml.load(fs.readFileSync(path.join(root, 'parity/features.yaml'), 'utf8'))
const platformEntry = matrix.platforms.find(platform => platform.id === 'windows-x64')
const requiredChecks = platformEntry.requiredChecks
const requiredFeatures = featuresDocument.features.filter(feature =>
    feature.platforms.includes('windows') && feature.tests?.manual?.length > 0)
const evidenceRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-manual-acceptance-evidence-'))
const evidenceDirectory = path.join(evidenceRoot, 'manual', platformEntry.id)
fs.mkdirSync(evidenceDirectory, { recursive: true })
for (const id of [...requiredChecks, ...requiredFeatures.map(feature => feature.id)]) {
    fs.writeFileSync(path.join(evidenceDirectory, `${id}.txt`), `verified ${id}\n`)
}
const artifactPath = path.join(evidenceRoot, 'artifacts', 'Tabby-RS-setup.exe')
fs.mkdirSync(path.dirname(artifactPath), { recursive: true })
fs.writeFileSync(artifactPath, 'fixture installer\n')
const artifactSha256 = crypto.createHash('sha256').update(fs.readFileSync(artifactPath)).digest('hex')
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
    features: requiredFeatures.map(feature => ({
        id: feature.id,
        status: 'passed',
        steps: ['verified ' + feature.id],
        evidence: ['manual/' + platformEntry.id + '/' + feature.id + '.txt'],
    })),
    artifacts: [{ path: 'artifacts/Tabby-RS-setup.exe', sha256: artifactSha256 }],
}

const valid = validateManualPlatformAcceptance(validRecord, {
    platformEntry,
    featureEntries: requiredFeatures,
    expectedRevision: validRecord.sourceRevision,
    expectedArchitecture: validRecord.architecture,
    expectedTarget: validRecord.target,
    evidenceRoot,
})
assert.deepEqual(valid, { passed: true, failures: [] })

const missingCheck = structuredClone(validRecord)
missingCheck.checks = missingCheck.checks.slice(1)
const missingResult = validateManualPlatformAcceptance(missingCheck, { platformEntry })
assert.ok(missingResult.failures.includes('missing manual platform check: ' + requiredChecks[0]))

const missingFeature = structuredClone(validRecord)
missingFeature.features = missingFeature.features.slice(1)
const missingFeatureResult = validateManualPlatformAcceptance(missingFeature, { platformEntry, featureEntries: requiredFeatures })
assert.ok(missingFeatureResult.failures.includes('missing manual feature: ' + requiredFeatures[0].id))

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

const missingEvidenceFile = structuredClone(validRecord)
missingEvidenceFile.features[0].evidence = ['manual/windows-x64/missing.txt']
const missingEvidenceResult = validateManualPlatformAcceptance(missingEvidenceFile, { platformEntry, featureEntries: requiredFeatures, evidenceRoot })
assert.ok(missingEvidenceResult.failures.includes('manual feature ' + requiredFeatures[0].id + ' evidence file is missing: manual/windows-x64/missing.txt'))

const mismatchedArtifact = structuredClone(validRecord)
mismatchedArtifact.artifacts[0].sha256 = 'f'.repeat(64)
const mismatchedArtifactResult = validateManualPlatformAcceptance(mismatchedArtifact, { platformEntry, featureEntries: requiredFeatures, evidenceRoot })
assert.ok(mismatchedArtifactResult.failures.includes('artifact 0 SHA-256 does not match file: artifacts/Tabby-RS-setup.exe'))

console.log('Manual platform acceptance contract passed')
