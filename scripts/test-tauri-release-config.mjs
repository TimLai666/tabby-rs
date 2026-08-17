import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const temporaryDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-release-config-'))
const output = path.join(temporaryDirectory, 'tauri.release.conf.json')

try {
    execFileSync(process.execPath, ['scripts/create-tauri-release-config.mjs'], {
        cwd: root,
        env: {
            ...process.env,
            TABBY_RS_RELEASE_CHANNEL: 'stable',
            TABBY_RS_RELEASE_VERSION: '1.0.0-test',
            TABBY_RS_UPDATE_PUBLIC_KEY: 'test-public-key',
            TABBY_RS_UPDATE_ENDPOINT: 'https://updates.example.test/stable',
            TABBY_RS_BUNDLE_TARGETS: 'dmg,nsis',
            TABBY_RS_RELEASE_CONFIG: output,
        },
        stdio: 'ignore',
    })

    const config = JSON.parse(fs.readFileSync(output, 'utf8'))
    assert.deepEqual(config.bundle.icon, [
        'icons/icon.icns',
        'icons/icon.png',
        'icons/icon.ico',
        '../build/mac/icon.icns',
    ])
    assert.deepEqual(config.bundle.targets, ['dmg', 'nsis'])
} finally {
    fs.rmSync(temporaryDirectory, { recursive: true, force: true })
}

console.log('Tauri release configuration icon fixture passed')
