import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-web/src/platform.ts'), 'utf8')

assert.match(source, /supportsPluginManagement = false/)
assert.match(source, /openPath \(_path: string\): void \{[\s\S]*?new UnsupportedCapabilityError\('filesystem'\)/)

console.log('Web capability boundary contract passed')
