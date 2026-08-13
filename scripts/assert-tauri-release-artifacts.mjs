import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const args = process.argv.slice(2)
const argument = name => {
    const index = args.indexOf(name)
    return index === -1 ? null : args[index + 1]
}

const stagingArgument = args[0] && !args[0].startsWith('--') ? args[0] : null
const staging = path.resolve(stagingArgument || process.env.TABBY_RS_RELEASE_STAGING || 'release-staging')
const platform = argument('--platform') || process.env.TABBY_RS_RELEASE_PLATFORM
const bundles = (argument('--bundles') || process.env.TABBY_RS_BUNDLE_TARGETS || '')
    .split(',')
    .map(value => value.trim().toLowerCase())
    .filter(Boolean)

assert.ok(fs.existsSync(staging), `release staging directory does not exist: ${staging}`)
assert.ok(['linux', 'macos', 'windows'].includes(platform), `unsupported release platform: ${platform || '<missing>'}`)
assert.ok(bundles.length > 0, 'release bundles must not be empty')

function walk (directory) {
    return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
        const filePath = path.join(directory, entry.name)
        if (entry.isDirectory()) return walk(filePath)
        return entry.isFile() ? [filePath] : []
    })
}

const files = walk(staging)
const basenames = files.map(filePath => path.basename(filePath))
const requiredExtensions = {
    appimage: ['.AppImage'],
    deb: ['.deb'],
    dmg: ['.dmg'],
    nsis: ['.exe'],
    rpm: ['.rpm'],
}

for (const bundle of bundles) {
    const extensions = requiredExtensions[bundle]
    assert.ok(extensions, `unsupported Tauri bundle type: ${bundle}`)
    for (const extension of extensions) {
        assert.ok(
            basenames.some(file => file.endsWith(extension)),
            `release is missing ${bundle} artifact (${extension})`,
        )
    }
}

function sha256 (filePath) {
    return crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex')
}

function findApplicationBundles (directory) {
    return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
        if (!entry.isDirectory()) return []
        const entryPath = path.join(directory, entry.name)
        if (entry.name.endsWith('.app')) return [entryPath]
        return findApplicationBundles(entryPath)
    })
}

function assertMacosApplicationIcon () {
    const applications = findApplicationBundles(staging)
    assert.equal(applications.length, 1, 'macOS release must contain exactly one application bundle')

    const iconPath = path.join(applications[0], 'Contents', 'Resources', 'icon.icns')
    const sourceIconPath = path.join(root, 'build/mac/icon.icns')
    assert.ok(fs.existsSync(iconPath), `macOS application icon is missing: ${iconPath}`)
    assert.ok(fs.existsSync(sourceIconPath), `source macOS application icon is missing: ${sourceIconPath}`)
    assert.equal(
        sha256(iconPath),
        sha256(sourceIconPath),
        'macOS application icon does not match build/mac/icon.icns',
    )
}

if (platform === 'macos') assertMacosApplicationIcon()

const primaryPatterns = {
    linux: file => file.endsWith('.AppImage'),
    macos: file => file.endsWith('.app.tar.gz'),
    windows: file => file.endsWith('.exe'),
}
const primary = files.filter(primaryPatterns[platform])
assert.equal(primary.length, 1, `release must contain exactly one ${platform} updater artifact`)
const primaryPath = primary[0]
const signaturePath = `${primaryPath}.sig`
assert.ok(fs.existsSync(signaturePath), `updater signature is missing for ${path.basename(primaryPath)}`)

function readJsonIfPresent (filePath, label) {
    if (!fs.existsSync(filePath)) return null
    try {
        return JSON.parse(fs.readFileSync(filePath, 'utf8'))
    } catch (error) {
        assert.fail(`${label} is invalid: ${error.message}`)
    }
}

const metadata = readJsonIfPresent(path.join(staging, 'tabby-rs-metadata.json'), 'release metadata')
if (metadata) {
    assert.ok(typeof metadata.version === 'string' && metadata.version.length > 0, 'release metadata version is missing')
    if (platform !== 'macos') {
        assert.ok(path.basename(primaryPath).includes(metadata.version), `updater artifact name does not contain release version ${metadata.version}`)
    }
    assert.equal(metadata.platform, platform, 'release metadata platform does not match artifact platform')
}

const manifest = readJsonIfPresent(path.join(staging, 'update-manifest.json'), 'update manifest')
if (manifest) {
    assert.ok(metadata, 'update manifest requires release metadata')
    assert.equal(manifest.version, metadata.version, 'update manifest version does not match release metadata')
    assert.equal(manifest.platform, metadata.platform, 'update manifest platform does not match release metadata')
    assert.equal(manifest.arch, metadata.arch, 'update manifest arch does not match release metadata')
    const manifestArtifact = path.basename(decodeURIComponent(new URL(manifest.url).pathname))
    assert.equal(manifestArtifact, path.basename(primaryPath), 'update manifest URL does not point to primary artifact')
    assert.equal(manifest.sha256, crypto.createHash('sha256').update(fs.readFileSync(primaryPath)).digest('hex'), 'update manifest hash does not match primary artifact')
    assert.equal(manifest.size, fs.statSync(primaryPath).size, 'update manifest size does not match primary artifact')
    assert.equal(manifest.signature, fs.readFileSync(signaturePath, 'utf8').trim(), 'update manifest signature does not match artifact signature')
}

const output = argument('--output')
if (output) {
    const report = {
        schemaVersion: 1,
        platform,
        bundles,
        primaryUpdaterArtifact: path.relative(staging, primaryPath).split(path.sep).join('/'),
        version: metadata?.version || null,
        files: basenames.sort(),
    }
    fs.writeFileSync(path.resolve(output), `${JSON.stringify(report, null, 2)}\n`)
}

console.log(primaryPath)
