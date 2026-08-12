import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'

const staging = path.resolve(process.env.TABBY_RS_RELEASE_STAGING || 'release-staging')
const output = path.resolve(process.env.TABBY_RS_UPDATE_MANIFEST_OUTPUT || path.join(staging, 'update-manifest.json'))
const channel = process.env.TABBY_RS_RELEASE_CHANNEL
const version = process.env.TABBY_RS_RELEASE_VERSION
const platform = process.env.TABBY_RS_UPDATE_PLATFORM
const arch = process.env.TABBY_RS_UPDATE_ARCH
const artifactURLTemplate = process.env.TABBY_RS_UPDATE_ARTIFACT_URL
const publishedAt = process.env.TABBY_RS_RELEASE_PUBLISHED_AT || new Date().toISOString()

assert.ok(channel === 'stable' || channel === 'nightly', 'TABBY_RS_RELEASE_CHANNEL must be stable or nightly')
assert.ok(version, 'TABBY_RS_RELEASE_VERSION is required')
assert.ok(platform, 'TABBY_RS_UPDATE_PLATFORM is required')
assert.ok(arch, 'TABBY_RS_UPDATE_ARCH is required')
assert.ok(artifactURLTemplate, 'TABBY_RS_UPDATE_ARTIFACT_URL is required')

function walk (directory) {
    const files = []
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
        const filePath = path.join(directory, entry.name)
        if (entry.isDirectory()) {
            files.push(...walk(filePath))
        } else if (entry.isFile()) {
            files.push(filePath)
        }
    }
    return files
}

const signaturePath = process.env.TABBY_RS_UPDATE_SIGNATURE
    ? path.resolve(process.env.TABBY_RS_UPDATE_SIGNATURE)
    : (() => {
        const candidates = walk(staging).filter(filePath => filePath.endsWith('.sig'))
        assert.equal(candidates.length, 1, 'staging must contain exactly one updater signature')
        return candidates[0]
    })()
assert.ok(fs.existsSync(signaturePath), `updater signature does not exist: ${signaturePath}`)
const artifactPath = path.resolve(process.env.TABBY_RS_UPDATE_ARTIFACT || signaturePath.slice(0, -'.sig'.length))
assert.ok(fs.existsSync(artifactPath), `signed updater artifact does not exist: ${artifactPath}`)

const artifact = fs.readFileSync(artifactPath)
const signature = fs.readFileSync(signaturePath, 'utf8').trim()
const artifactURL = artifactURLTemplate
    .replaceAll('{{channel}}', channel)
    .replaceAll('{{version}}', version)
    .replaceAll('{{platform}}', platform)
    .replaceAll('{{arch}}', arch)
    .replaceAll('{{artifact}}', path.basename(artifactPath))
const parsedURL = new URL(artifactURL)
assert.equal(parsedURL.protocol, 'https:', 'TABBY_RS_UPDATE_ARTIFACT_URL must be HTTPS')
assert.equal(parsedURL.username, '', 'TABBY_RS_UPDATE_ARTIFACT_URL must not contain credentials')
assert.equal(parsedURL.password, '', 'TABBY_RS_UPDATE_ARTIFACT_URL must not contain credentials')
assert.equal(parsedURL.hash, '', 'TABBY_RS_UPDATE_ARTIFACT_URL must not contain a fragment')
assert.ok(signature, 'updater signature must not be empty')
assert.ok(artifact.length > 0, 'updater artifact must not be empty')
const manifest = {
    version,
    notes: process.env.TABBY_RS_RELEASE_NOTES || '',
    pub_date: publishedAt,
    url: artifactURL,
    signature,
    schemaVersion: 1,
    channel,
    platform,
    arch,
    sha256: crypto.createHash('sha256').update(artifact).digest('hex'),
    size: artifact.length,
    requiresConfigMigration: process.env.TABBY_RS_REQUIRES_CONFIG_MIGRATION === 'true',
}

fs.mkdirSync(path.dirname(output), { recursive: true })
fs.writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`)
console.log(`Created updater manifest at ${output}`)

export { manifest }
