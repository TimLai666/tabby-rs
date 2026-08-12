import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/updater.service.ts'), 'utf8')

assert.match(source, /PlatformService, TranslateService/)
assert.match(source, /await this\.platform\.showMessageBox\(\{[\s\S]*buttons:/)
assert.match(source, /if \(confirmation\.response !== 0\) \{[\s\S]*return/)
assert.match(source, /if \(confirmation\.response !== 0\)[\s\S]*await this\.bridge\.invoke\('update\.install'/)

console.log('Tauri updater confirmation contract passed')
