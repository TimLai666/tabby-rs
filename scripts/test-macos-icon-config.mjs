import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const buildScript = fs.readFileSync(path.join(root, 'scripts/build-macos.mjs'), 'utf8')
const tauriConfig = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'))

assert.match(buildScript, /const macIcon = ['"]mac\/icon\.icns['"]/, 'macOS build must use the canonical icon asset')
assert.match(buildScript, /config:\s*\{[\s\S]*?icon:\s*macIcon/, 'Electron Builder must receive the app icon')
assert.match(buildScript, /mac:\s*\{[\s\S]*?icon:\s*macIcon/, 'macOS app packaging must receive the app icon')
assert.match(buildScript, /dmg:\s*\{[\s\S]*?icon:\s*macIcon/, 'DMG volume packaging must receive the volume icon')
const canonicalMacIcon = path.join(root, 'build/mac/icon.icns')
assert.ok(fs.statSync(canonicalMacIcon).isFile(), 'canonical macOS icon is missing')

const icns = fs.readFileSync(canonicalMacIcon)
assert.equal(icns.subarray(0, 4).toString('ascii'), 'icns', 'canonical macOS icon is not an ICNS file')
assert.equal(icns.readUInt32BE(4), icns.length, 'canonical macOS icon length is invalid')
const iconTypes = new Set()
for (let offset = 8; offset < icns.length;) {
    assert.ok(offset + 8 <= icns.length, 'canonical macOS icon has a truncated chunk header')
    const type = icns.subarray(offset, offset + 4).toString('ascii')
    const length = icns.readUInt32BE(offset + 4)
    assert.ok(length >= 8, `canonical macOS icon chunk ${type} has an invalid length`)
    assert.ok(offset + length <= icns.length, `canonical macOS icon chunk ${type} exceeds the file`)
    iconTypes.add(type)
    offset += length
}
const legacyRepresentations = ['icp4', 'icp5', 'icp6', 'ic07', 'ic08', 'ic09', 'ic10']
const modernRepresentations = ['ic07', 'ic08', 'ic09', 'ic10', 'ic11', 'ic12', 'ic13', 'ic14']
assert.ok(
    [legacyRepresentations, modernRepresentations].some(representations => representations.every(type => iconTypes.has(type))),
    'canonical macOS icon is missing a complete legacy or modern representation set',
)

const tauriIcons = tauriConfig.bundle?.icon
assert.ok(Array.isArray(tauriIcons), 'Tauri bundle must declare icon assets')
assert.ok(tauriIcons.includes('../build/mac/icon.icns'), 'Tauri macOS bundle must use the canonical icon asset')
assert.equal(
    path.resolve(root, 'src-tauri', tauriIcons.find(icon => icon === '../build/mac/icon.icns')),
    canonicalMacIcon,
    'Tauri canonical icon path must resolve to build/mac/icon.icns',
)

console.log('macOS icon configuration fixture passed')
