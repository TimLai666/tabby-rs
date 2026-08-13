import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-update-manifest-'))
const artifact = path.join(fixture, 'Tabby-RS.AppImage.tar.gz')
const signature = `${artifact}.sig`
const output = path.join(fixture, 'update-manifest.json')
const bytes = Buffer.from('signed artifact')
fs.writeFileSync(artifact, bytes)
fs.writeFileSync(signature, 'signature-from-ci\n')

execFileSync(process.execPath, [path.join(root, 'scripts/create-tauri-update-manifest.mjs')], {
    cwd: root,
    env: {
        ...process.env,
        TABBY_RS_RELEASE_STAGING: fixture,
        TABBY_RS_UPDATE_MANIFEST_OUTPUT: output,
        TABBY_RS_RELEASE_CHANNEL: 'stable',
        TABBY_RS_RELEASE_VERSION: '1.0.231-tabbyrs.2',
        TABBY_RS_UPDATE_PLATFORM: 'linux',
        TABBY_RS_UPDATE_ARCH: 'x86_64',
        TABBY_RS_UPDATE_ARTIFACT_URL: 'https://updates.example.test/{{channel}}/{{version}}/{{platform}}-{{arch}}/{{artifact}}',
    },
})

const manifest = JSON.parse(fs.readFileSync(output, 'utf8'))
assert.equal(manifest.version, '1.0.231-tabbyrs.2')
assert.equal(manifest.channel, 'stable')
assert.equal(manifest.platform, 'linux')
assert.equal(manifest.arch, 'x86_64')
assert.equal(manifest.signature, 'signature-from-ci')
assert.equal(manifest.sha256, crypto.createHash('sha256').update(bytes).digest('hex'))
assert.equal(manifest.size, bytes.length)
assert.equal(manifest.pub_date.length > 0, true)
assert.equal(Object.hasOwn(manifest, 'TAURI_SIGNING_PRIVATE_KEY'), false)

console.log('Tauri update manifest fixtures passed')
