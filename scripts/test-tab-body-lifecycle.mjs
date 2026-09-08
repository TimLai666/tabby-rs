import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-core/src/components/tabBody.component.ts'), 'utf8')

assert.match(source, /ngOnChanges \(changes\) \{[\s\S]*?setImmediate\(\(\) => \{[\s\S]*?this\.placeholder\?\.insert\(this\.tab\.hostView\)/)

console.log('Tab body initial view lifecycle contract passed')
