import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const bridge = fs.readFileSync(path.join(root, 'tabby-tauri/src/api/hostBridge.ts'), 'utf8')
const commands = fs.readFileSync(path.join(root, 'src-tauri/src/commands/plugins.rs'), 'utf8')
const npm = fs.readFileSync(path.join(root, 'src-tauri/src/plugins/npm.rs'), 'utf8')

assert.match(bridge, /action: 'install' \| 'update' \| 'uninstall'/)
assert.match(commands, /pub async fn plugins_update[\s\S]*?running_operation\(&request\.operation_id, &request\.package_name, "update"\)/)
assert.match(npm, /pub async fn update[\s\S]*?action: "update"\.into\(\)/)

console.log('Plugin operation action contract passed')
