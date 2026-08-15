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

const tauriIcons = tauriConfig.bundle?.icon
assert.ok(Array.isArray(tauriIcons), 'Tauri bundle must declare icon assets')
assert.ok(tauriIcons.includes('../build/mac/icon.icns'), 'Tauri macOS bundle must use the canonical icon asset')
assert.equal(
    path.resolve(root, 'src-tauri', tauriIcons.find(icon => icon === '../build/mac/icon.icns')),
    canonicalMacIcon,
    'Tauri canonical icon path must resolve to build/mac/icon.icns',
)

console.log('macOS icon configuration fixture passed')
