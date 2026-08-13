import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'

const outputArgument = process.argv.indexOf('--output-dir')
const sourceArgument = process.argv.indexOf('--source-dir')
const outputValue = outputArgument === -1 ? 'src-tauri/icons' : process.argv[outputArgument + 1]
const sourceValue = sourceArgument === -1 ? 'src-tauri/icons' : process.argv[sourceArgument + 1]
assert.ok(outputValue, '--output-dir requires a directory')
assert.ok(sourceValue, '--source-dir requires a directory')
const outputDirectory = path.resolve(outputValue)
const sourceDirectory = path.resolve(sourceValue)

const assets = [
    { encoded: 'icon.png.b64', output: 'icon.png', signature: Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) },
    { encoded: 'icon.ico.b64', output: 'icon.ico', signature: Buffer.from([0x00, 0x00, 0x01, 0x00]) },
]

fs.mkdirSync(outputDirectory, { recursive: true })
for (const asset of assets) {
    const source = path.join(sourceDirectory, asset.encoded)
    const destination = path.join(outputDirectory, asset.output)
    const encoded = fs.readFileSync(source, 'utf8').replace(/\s+/g, '')
    assert.ok(encoded, `${asset.encoded} is empty`)
    const bytes = Buffer.from(encoded, 'base64')
    assert.ok(bytes.length > asset.signature.length, `${asset.encoded} is invalid`)
    assert.deepEqual(bytes.subarray(0, asset.signature.length), asset.signature, `${asset.encoded} has an unexpected file signature`)
    fs.writeFileSync(destination, bytes, { mode: 0o644 })
}

console.log(`Restored Tauri icons in ${outputDirectory}`)
