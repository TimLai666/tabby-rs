import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'

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

const primaryPatterns = {
    linux: file => file.endsWith('.AppImage'),
    macos: file => file.endsWith('.app.tar.gz'),
    windows: file => file.endsWith('.exe'),
}
const primary = files.filter(primaryPatterns[platform])
assert.equal(primary.length, 1, `release must contain exactly one ${platform} updater artifact`)
const primaryPath = primary[0]
assert.ok(fs.existsSync(`${primaryPath}.sig`), `updater signature is missing for ${path.basename(primaryPath)}`)

const output = argument('--output')
if (output) {
    const report = {
        schemaVersion: 1,
        platform,
        bundles,
        primaryUpdaterArtifact: path.relative(staging, primaryPath).split(path.sep).join('/'),
        files: basenames.sort(),
    }
    fs.writeFileSync(path.resolve(output), `${JSON.stringify(report, null, 2)}\n`)
}

console.log(primaryPath)
