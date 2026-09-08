import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-core/src/components/appRoot.component.ts'), 'utf8')

assert.match(source, /this\.unsortedTabs = \[\.\.\.this\.app\.tabs\]/)

console.log('App root recovery hydration contract passed')
