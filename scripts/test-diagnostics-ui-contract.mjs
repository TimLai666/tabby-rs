import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(
    path.join(root, 'tabby-tauri/src/components/diagnosticsSettingsTab.component.ts'),
    'utf8',
)
const exportMethod = source.match(/async exportBundle \(\): Promise<void> \{([\s\S]*?)\n    \}/)?.[1]

assert.ok(exportMethod, 'diagnostics export method is missing')
assert.doesNotMatch(exportMethod, /loadPreview\(/, 'export must not bypass the preview step')
assert.match(exportMethod, /if \(!this\.preview\)/)
assert.match(exportMethod, /Preview diagnostics before exporting\./)
assert.match(exportMethod, /return/)
assert.match(exportMethod, /this\.diagnostics\.exportBundle\(\)/)

console.log('Diagnostics UI preview gate contract passed')
