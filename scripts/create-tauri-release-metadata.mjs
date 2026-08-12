import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'

const output = path.resolve(process.env.TABBY_RS_RELEASE_STAGING || 'release-staging')
const channel = process.env.TABBY_RS_RELEASE_CHANNEL
const version = process.env.TABBY_RS_RELEASE_VERSION
const publicKey = process.env.TABBY_RS_UPDATE_PUBLIC_KEY
const revision = process.env.GITHUB_SHA || 'local'

assert.ok(channel === 'stable' || channel === 'nightly', 'TABBY_RS_RELEASE_CHANNEL must be stable or nightly')
assert.ok(version, 'TABBY_RS_RELEASE_VERSION is required')
assert.ok(publicKey?.trim(), 'TABBY_RS_UPDATE_PUBLIC_KEY is required')

fs.mkdirSync(output, { recursive: true })
fs.copyFileSync('LICENSE', path.join(output, 'LICENSE'))
fs.copyFileSync('src-tauri/icons/icon.png', path.join(output, 'tabby-rs-icon.png'))
fs.writeFileSync(path.join(output, 'updater-public-key.txt'), `${publicKey.trim()}\n`)
fs.writeFileSync(path.join(output, 'tabby-rs-metadata.json'), `${JSON.stringify({
    product: 'Tabby RS',
    channel,
    version,
    revision,
    osCodeSigning: 'not-performed',
}, null, 2)}\n`)
console.log(`Prepared release metadata at ${output}`)
