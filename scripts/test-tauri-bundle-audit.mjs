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
        ['macos/Tabby RS.app.tar.gz.sig', 'signature'],
        ['bundle/tabby-rs.exe', 'native app'],
    ]) {
        const filePath = path.join(directory, name)
        fs.mkdirSync(path.dirname(filePath), { recursive: true })
        fs.writeFileSync(filePath, contents)
    }
    return directory
}

const evidenceFixture = createReleaseFixture()
fs.writeFileSync(
    path.join(evidenceFixture, 'dependency-audit.json'),
    JSON.stringify({
        excluded: [{ package: 'electron-updater' }],
        secret: 'TAURI_SIGNING_PRIVATE_KEY',
    }),
)
const evidenceReport = auditBundle(evidenceFixture, { release: true })
assert.equal(evidenceReport.passed, false)
assert.ok(evidenceReport.findings.some(finding => finding.rule === 'private-key-material'))
assert.equal(evidenceReport.findings.some(finding => finding.path === 'dependency-audit.json' && finding.rule === 'electron-runtime-import'), false)
assert.equal(evidenceReport.files.find(file => file.path === 'LICENSE').sha256.length, 64)
assert.deepEqual(evidenceReport.missing, [])

const passingFixture = createReleaseFixture()
fs.writeFileSync(
    path.join(passingFixture, 'dependency-audit.json'),
    JSON.stringify({ excluded: [{ package: 'electron-updater' }] }),
)
const passingReport = auditBundle(passingFixture, { release: true })
assert.equal(passingReport.passed, true)

const forbiddenFixture = createReleaseFixture()
fs.writeFileSync(path.join(forbiddenFixture, 'node.exe'), 'node')
fs.writeFileSync(path.join(forbiddenFixture, 'bundle.js'), 'https://example.sentry.io/123')
fs.writeFileSync(path.join(forbiddenFixture, 'electron-import.js'), "const { ipcRenderer } = require('electron')\n")
fs.writeFileSync(path.join(forbiddenFixture, 'signing-private-key.pem'), 'private key')
fs.writeFileSync(
    path.join(forbiddenFixture, 'secret.txt'),
    '-----BEGIN PRIVATE KEY-----\n' + 'A'.repeat(32) + '\n-----END PRIVATE KEY-----',
)
const forbiddenReport = auditBundle(forbiddenFixture, { release: true })
assert.equal(forbiddenReport.passed, false)
assert.ok(forbiddenReport.findings.some(finding => finding.rule === 'node-runtime'))
assert.ok(forbiddenReport.findings.some(finding => finding.rule === 'sentry-sdk-or-endpoint'))
assert.ok(forbiddenReport.findings.some(finding => finding.rule === 'electron-runtime-import'))
assert.ok(forbiddenReport.findings.some(finding => finding.rule === 'private-key-file'))
assert.ok(forbiddenReport.findings.some(finding => finding.rule === 'private-key-material'))

const binaryFixture = createReleaseFixture()
fs.writeFileSync(
    path.join(binaryFixture, 'runtime.bin'),
    Buffer.from([0, ...Buffer.from('electron.asar'), 0, ...Buffer.from('node-runtime-required'), 0]),
)
const binaryReport = auditBundle(binaryFixture, { release: true })
assert.equal(binaryReport.passed, false)
assert.ok(binaryReport.findings.some(finding => finding.rule === 'electron-runtime-binary'))
assert.equal(binaryReport.findings.some(finding => finding.rule === 'node-runtime-binary'), false)

const vendorKeyFixture = createReleaseFixture()
const vendorLibraryPath = path.join(vendorKeyFixture, 'appimage/Tabby RS.AppDir/usr/lib/libgnutls.so.30')
fs.mkdirSync(path.dirname(vendorLibraryPath), { recursive: true })
fs.writeFileSync(vendorLibraryPath, '-----BEGIN PRIVATE KEY-----\n' + 'A'.repeat(32) + '\n-----END PRIVATE KEY-----')
const vendorKeyReport = auditBundle(vendorKeyFixture, { release: true })
assert.ok(vendorKeyReport.findings.some(finding =>
    finding.path === 'appimage/Tabby RS.AppDir/usr/lib/libgnutls.so.30' && finding.rule === 'private-key-material'))

const largeBinaryFixture = createReleaseFixture()
const largeBinary = Buffer.alloc(16 * 1024 * 1024 + 1)
Buffer.from('electron.asar').copy(largeBinary, 16 * 1024 * 1024 - 16)
fs.writeFileSync(path.join(largeBinaryFixture, 'large-runtime.bin'), largeBinary)
const largeBinaryReport = auditBundle(largeBinaryFixture, { release: true })
assert.equal(largeBinaryReport.passed, false)
assert.ok(largeBinaryReport.findings.some(finding => finding.rule === 'electron-runtime-binary'))

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
