import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { auditDependencyMetadata } from './ci/assert-tauri-dependency-safety.mjs'

function createManifest (contents) {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-dependency-audit-'))
    const manifestPath = path.join(directory, 'package.json')
    fs.writeFileSync(manifestPath, JSON.stringify(contents))
    return manifestPath
}

const cleanManifest = createManifest({
    dependencies: { '@angular/core': '^15.2.6' },
    devDependencies: { typescript: '^4.9.5' },
})
assert.deepEqual(auditDependencyMetadata([cleanManifest]).findings, [])
assert.equal(auditDependencyMetadata([cleanManifest]).passed, true)

for (const section of ['dependencies', 'optionalDependencies', 'peerDependencies', 'devDependencies']) {
    const forbiddenManifest = createManifest({ [section]: { electron: '38' } })
    const report = auditDependencyMetadata([forbiddenManifest])
    assert.equal(report.passed, false)
    assert.deepEqual(report.findings.map(finding => [finding.rule, finding.section, finding.package]), [
        ['electron-runtime-dependency', section, 'electron'],
    ])
}

const nativeManifest = createManifest({ dependencies: { keytar: '^7.9.0', 'node-pty': '^1.2.0' } })
const nativeReport = auditDependencyMetadata([nativeManifest])
assert.equal(nativeReport.passed, false)
assert.deepEqual(nativeReport.findings.map(finding => finding.rule), [
    'node-native-dependency',
    'node-native-dependency',
])

const malformedManifest = createManifest({ dependencies: [] })
const malformedReport = auditDependencyMetadata([malformedManifest])
assert.equal(malformedReport.passed, false)
assert.equal(malformedReport.findings[0].rule, 'invalid-dependency-section')

const invalidManifestDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-dependency-audit-invalid-'))
const invalidManifest = path.join(invalidManifestDirectory, 'package.json')
fs.writeFileSync(invalidManifest, '{')
const invalidReport = auditDependencyMetadata([invalidManifest])
assert.equal(invalidReport.passed, false)
assert.equal(invalidReport.findings[0].rule, 'invalid-package-manifest')

console.log('Tauri dependency metadata audit fixtures passed')
