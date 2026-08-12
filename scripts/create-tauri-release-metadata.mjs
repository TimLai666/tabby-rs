import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'

const output = path.resolve(process.env.TABBY_RS_RELEASE_STAGING || 'release-staging')
const channel = process.env.TABBY_RS_RELEASE_CHANNEL
const version = process.env.TABBY_RS_RELEASE_VERSION
const publicKey = process.env.TABBY_RS_UPDATE_PUBLIC_KEY
const revision = process.env.GITHUB_SHA || process.env.TABBY_RS_SOURCE_REVISION || 'local'
const target = process.env.TABBY_RS_RELEASE_TARGET || 'unspecified'
const platform = process.env.TABBY_RS_RELEASE_PLATFORM || 'unspecified'
const arch = process.env.TABBY_RS_RELEASE_ARCH || 'unspecified'

assert.ok(channel === 'stable' || channel === 'nightly', 'TABBY_RS_RELEASE_CHANNEL must be stable or nightly')
assert.ok(version, 'TABBY_RS_RELEASE_VERSION is required')
assert.ok(publicKey?.trim(), 'TABBY_RS_UPDATE_PUBLIC_KEY is required')

const sha256 = filePath => crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex')
const rustVersion = process.env.TABBY_RS_RUSTC_VERSION || (() => {
    try {
        return execFileSync('rustc', ['--version'], { encoding: 'utf8' }).trim()
    } catch {
        return 'unspecified'
    }
})()
const lockFiles = ['yarn.lock', 'src-tauri/Cargo.lock']
const dependencyLocks = Object.fromEntries(lockFiles.map(filePath => {
    assert.ok(fs.statSync(filePath).isFile(), `${filePath} does not exist`)
    return [filePath, sha256(filePath)]
}))

fs.mkdirSync(output, { recursive: true })
fs.copyFileSync('LICENSE', path.join(output, 'LICENSE'))
fs.copyFileSync('src-tauri/icons/icon.png', path.join(output, 'tabby-rs-icon.png'))
fs.writeFileSync(path.join(output, 'updater-public-key.txt'), `${publicKey.trim()}\n`)
fs.writeFileSync(path.join(output, 'tabby-rs-metadata.json'), `${JSON.stringify({
    product: 'Tabby RS',
    channel,
    version,
    revision,
    target,
    platform,
    arch,
    dependencyLocks,
    toolchain: {
        node: process.version,
        rust: rustVersion,
        tauriCli: process.env.TAURI_CLI_VERSION || 'unspecified',
    },
    osCodeSigning: 'not-performed',
}, null, 2)}\n`)
console.log(`Prepared release metadata at ${output}`)
