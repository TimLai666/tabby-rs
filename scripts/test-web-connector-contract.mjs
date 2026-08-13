import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const entry = fs.readFileSync(path.join(root, 'web/entry.ts'), 'utf8')
const polyfills = fs.readFileSync(path.join(root, 'web/polyfills.ts'), 'utf8')

assert.match(
    entry,
    /window\['__connector__'\]\s*=\s*options\.connector/,
    'web bootstrap must publish its connector for the browser socket shim',
)
assert.match(
    polyfills,
    /window\['__connector__'\]\.createSocket\(\.\.\.args\)/,
    'browser socket shim must use the bootstrap connector',
)

console.log('Web connector transport contract passed')
