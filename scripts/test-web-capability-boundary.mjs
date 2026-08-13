import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-web/src/platform.ts'), 'utf8')
const pluginSettings = fs.readFileSync(path.join(root, 'tabby-plugin-manager/src/components/pluginsSettingsTab.component.pug'), 'utf8')

assert.match(source, /supportsPluginManagement = false/)
assert.match(source, /openPath \(_path: string\): void \{[\s\S]*?new UnsupportedCapabilityError\('filesystem'\)/)
assert.match(source, /showItemInFolder \(_path: string\): void \{[\s\S]*?new UnsupportedCapabilityError\('filesystem'\)/)
assert.match(source, /installPlugin \(_name: string, _version: string\): Promise<void> \{[\s\S]*?new UnsupportedCapabilityError\('pluginInstall'\)/)
assert.match(source, /uninstallPlugin \(_name: string\): Promise<void> \{[\s\S]*?new UnsupportedCapabilityError\('pluginInstall'\)/)
assert.match(source, /cancelPluginOperation \(_id: string\): Promise<void> \{[\s\S]*?new UnsupportedCapabilityError\('pluginInstall'\)/)
assert.match(source, /getWinSCPPath \(\): string\|null \{[\s\S]*?new UnsupportedCapabilityError\('filesystem'\)/)
assert.match(source, /async exec \(_app: string, _argv: string\[\]\): Promise<void> \{[\s\S]*?new UnsupportedCapabilityError\('localPty'\)/)
assert.match(pluginSettings, /button\.btn\.btn-secondary\.btn-sm\.ms-auto\(\*ngIf='canManagePlugins\(\)'/)

console.log('Web capability boundary contract passed')
