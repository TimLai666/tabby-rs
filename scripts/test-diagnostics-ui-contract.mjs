import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(
    path.join(root, 'tabby-tauri/src/components/diagnosticsSettingsTab.component.ts'),
    'utf8',
)
const service = fs.readFileSync(
    path.join(root, 'tabby-tauri/src/services/diagnostics.service.ts'),
    'utf8',
)
const exportMethod = source.match(/async exportBundle \(\): Promise<void> \{([\s\S]*?)\n    \}/)?.[1]

assert.ok(exportMethod, 'diagnostics export method is missing')
assert.doesNotMatch(exportMethod, /loadPreview\(/, 'export must not bypass the preview step')
assert.match(exportMethod, /if \(!this\.preview\)/)
assert.match(exportMethod, /Preview diagnostics before exporting\./)
assert.match(exportMethod, /return/)
assert.match(exportMethod, /this\.diagnostics\.exportBundle\(\)/)
assert.match(service, /showMessageBox\(\{[\s\S]*?Export the reviewed diagnostic files\?/)
assert.match(service, /confirmation\.response !== 0/)
assert.match(service, /buttons: \[[\s\S]*?translate\.instant\('Export'\)[\s\S]*?translate\.instant\('Cancel'\)/)

console.log('Diagnostics UI preview gate contract passed')
