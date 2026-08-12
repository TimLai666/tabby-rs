import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const core = fs.readFileSync(path.join(root, 'tabby-core/src/services/updater.service.ts'), 'utf8')
const tauri = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/updater.service.ts'), 'utf8')
const settings = fs.readFileSync(path.join(root, 'tabby-settings/src/components/settingsTab.component.ts'), 'utf8')
const template = fs.readFileSync(path.join(root, 'tabby-settings/src/components/settingsTab.component.pug'), 'utf8')

assert.match(core, /canCancel \(\): boolean/)
assert.match(core, /async cancel \(\): Promise<void>/)
assert.match(tauri, /status === 'checking' \|\| this\.updateState\.status === 'downloading'/)
assert.match(tauri, /bridge\.invoke\('update\.cancel'/)
assert.match(settings, /async cancelUpdate \(\)/)
assert.match(template, /updating && updater\.canCancel\(\)/)

console.log('Updater cancellation contract passed')
