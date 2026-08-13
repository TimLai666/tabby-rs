import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-web/src/services/hostApp.service.ts'), 'utf8')

assert.match(source, /newWindow \(\): void \{[\s\S]*?window\.open\(window\.location\.href, '_blank', 'noopener,noreferrer'\)/)
assert.match(source, /if \(!opened\) \{[\s\S]*?this\.logger\.warn\('Browser blocked opening a new window'\)/)
assert.doesNotMatch(source, /newWindow \(\): void \{\s*throw new Error\('Not implemented'\)/)

console.log('Web host-app capability contract passed')
