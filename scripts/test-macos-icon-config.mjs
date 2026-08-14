import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const buildScript = fs.readFileSync(path.join(root, 'scripts/build-macos.mjs'), 'utf8')

assert.match(buildScript, /const macIcon = ['"]mac\/icon\.icns['"]/, 'macOS build must use the canonical icon asset')
assert.match(buildScript, /config:\s*\{[\s\S]*?icon:\s*macIcon/, 'Electron Builder must receive the app icon')
assert.match(buildScript, /mac:\s*\{[\s\S]*?icon:\s*macIcon/, 'macOS app packaging must receive the app icon')
assert.match(buildScript, /dmg:\s*\{[\s\S]*?icon:\s*macIcon/, 'DMG volume packaging must receive the volume icon')
assert.ok(fs.statSync(path.join(root, 'build/mac/icon.icns')).isFile(), 'canonical macOS icon is missing')

console.log('macOS icon configuration fixture passed')
