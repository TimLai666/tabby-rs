import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { auditBundle } from './check-tauri-bundle.mjs'

function createReleaseFixture () {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-bundle-audit-'))
    for (const [name, contents] of [
        ['LICENSE', 'MIT'],
        ['THIRD_PARTY_NOTICES.md', 'Dependencies'],
        ['tabby-rs-metadata.json', '{"product":"Tabby RS"}'],
        ['tabby-rs-icon.png', 'icon'],
        ['updater-public-key.txt', 'public-key'],
        ['update-manifest.json', '{}'],
        ['Tabby-RS.AppImage.sig', 'signature'],
        ['bundle/tabby-rs.exe', 'native app'],
    ]) {
        const filePath = path.join(directory, name)
        fs.mkdirSync(path.dirname(filePath), { recursive: true })
        fs.writeFileSync(filePath, contents)
    }
    return directory
}

const passingFixture = createReleaseFixture()
const passingReport = auditBundle(passingFixture, { release: true })
assert.equal(passingReport.passed, true)
assert.equal(passingReport.files.find(file => file.path === 'LICENSE').sha256.length, 64)
assert.deepEqual(passingReport.missing, [])

const forbiddenFixture = createReleaseFixture()
fs.writeFileSync(path.join(forbiddenFixture, 'node.exe'), 'node')
fs.writeFileSync(path.join(forbiddenFixture, 'bundle.js'), 'https://example.sentry.io/123')
const forbiddenReport = auditBundle(forbiddenFixture, { release: true })
assert.equal(forbiddenReport.passed, false)
assert.ok(forbiddenReport.findings.some(finding => finding.rule === 'node-runtime'))
assert.ok(forbiddenReport.findings.some(finding => finding.rule === 'sentry-sdk-or-endpoint'))

const incompleteFixture = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-bundle-audit-incomplete-'))
fs.writeFileSync(path.join(incompleteFixture, 'bundle.js'), 'renderer')
const incompleteReport = auditBundle(incompleteFixture, { release: true })
assert.equal(incompleteReport.passed, false)
assert.deepEqual(incompleteReport.missing.sort(), [
    'icon',
    'license',
    'metadata',
    'third-party-notices',
    'update-manifest',
    'updater-public-key',
    'updater-signature',
])

console.log('Tauri bundle audit fixtures passed')
