import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/updater.service.ts'), 'utf8')
const updateCommand = fs.readFileSync(path.join(root, 'src-tauri/src/commands/update.rs'), 'utf8')
const updateService = fs.readFileSync(path.join(root, 'src-tauri/src/update/service.rs'), 'utf8')

assert.match(source, /PlatformService, TranslateService/)
assert.match(source, /await this\.platform\.showMessageBox\(\{[\s\S]*buttons:/)
assert.match(source, /if \(confirmation\.response !== 0\) \{[\s\S]*return/)
assert.match(source, /if \(confirmation\.response !== 0\)[\s\S]*await this\.bridge\.invoke\('update\.install'/)
assert.match(updateCommand, /if configured_endpoint\(&request\.channel\)\.is_none\(\) \|\| configured_public_key\(\)\.is_none\(\)/)
assert.match(updateCommand, /configured_public_key\(\)[\s\S]*return Err\(AppError::Io\([\s\S]*let paths/)
assert.match(updateCommand, /emit_state\(&app, &state\);\n    check_update\(&app, &state\)\.await\?/)
assert.match(updateService, /\.configure_client\(configure_updater_client\)/)
assert.match(updateService, /client\.redirect\(reqwest::redirect::Policy::none\(\)\)/)

console.log('Tauri updater confirmation contract passed')
