import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'

const channel = process.env.TABBY_RS_RELEASE_CHANNEL
const version = process.env.TABBY_RS_RELEASE_VERSION
const publicKey = process.env.TABBY_RS_UPDATE_PUBLIC_KEY
const endpoint = process.env.TABBY_RS_UPDATE_ENDPOINT
const bundleTargets = process.env.TABBY_RS_BUNDLE_TARGETS?.split(',').filter(Boolean)
const output = process.env.TABBY_RS_RELEASE_CONFIG || path.resolve('src-tauri/tauri.release.conf.json')
const iconAssets = [
    'icons/icon.icns',
    'icons/icon.png',
    'icons/icon.ico',
    '../build/mac/icon.icns',
]

assert.ok(channel === 'stable' || channel === 'nightly', 'TABBY_RS_RELEASE_CHANNEL must be stable or nightly')
assert.ok(version, 'TABBY_RS_RELEASE_VERSION is required for a release build')
assert.ok(publicKey?.trim(), 'TABBY_RS_UPDATE_PUBLIC_KEY is required for a release build')
assert.ok(endpoint?.startsWith('https://'), 'TABBY_RS_UPDATE_ENDPOINT must be an HTTPS URL')
assert.ok(bundleTargets?.length, 'TABBY_RS_BUNDLE_TARGETS must list at least one target')

const config = {
    version,
    bundle: {
        active: true,
        targets: bundleTargets,
        createUpdaterArtifacts: true,
        icon: iconAssets,
    },
    plugins: {
        updater: {
            pubkey: publicKey.trim(),
            endpoints: [endpoint],
        },
    },
}
fs.mkdirSync(path.dirname(output), { recursive: true })
fs.writeFileSync(output, `${JSON.stringify(config, null, 2)}\n`)
console.log(`Generated ${output} for ${channel} release`)
