import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/platform.service.ts'), 'utf8')
const template = fs.readFileSync(path.join(root, 'tabby-plugin-manager/src/components/pluginsSettingsTab.component.pug'), 'utf8')
const method = source.match(/async getNodeToolchainStatus[\s\S]*?\r?\n    }\r?\n\r?\n    async installPlugin/)

assert.ok(method, 'Tauri node toolchain status method must exist')
assert.match(method[0], /try \{[\s\S]*bridge\.invoke\('plugins\.nodeStatus'/)
assert.match(method[0], /this\.supportsPluginManagement = status\.supported/)
assert.match(method[0], /catch \(error\) \{[\s\S]*this\.supportsPluginManagement = false[\s\S]*throw error/)
assert.match(template, /\.input-group\.mb-3\(\*ngIf='nodeStatus'\)[\s\S]*Node\.js path/)

console.log('Plugin node status contract passed')
