import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const platform = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/platform.service.ts'), 'utf8')
const shellProvider = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/shellProvider.service.ts'), 'utf8')
const pluginSettings = fs.readFileSync(path.join(root, 'tabby-plugin-manager/src/components/pluginsSettingsTab.component.ts'), 'utf8')
const bridge = fs.readFileSync(path.join(root, 'tabby-tauri/src/api/hostBridge.ts'), 'utf8')
const desktop = fs.readFileSync(path.join(root, 'src-tauri/src/commands/desktop.rs'), 'utf8')
const registration = fs.readFileSync(path.join(root, 'src-tauri/src/lib.rs'), 'utf8')
const tauriConfig = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'))
const capabilities = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/capabilities/default.json'), 'utf8'))
const tauriEntry = fs.readFileSync(path.join(root, 'app/src/entry.tauri.ts'), 'utf8')
const tauriPolyfills = fs.readFileSync(path.join(root, 'app/src/tauri-polyfills.ts'), 'utf8')
const tauriWebpack = fs.readFileSync(path.join(root, 'app/webpack.config.tauri.mjs'), 'utf8')

assert.match(platform, /async exec \(app: string, argv: string\[\]\): Promise<void> \{[\s\S]*?desktop\.exec/)
assert.match(platform, /getWinSCPPath \(\): string \| null \{\s*return null\s*\}/)
assert.doesNotMatch(shellProvider, /tabby-electron\/src\/icons/)
assert.match(shellProvider, /require\('\.\.\/icons\/alpine\.svg'\)/)
assert.doesNotMatch(pluginSettings, /FORCE_ENABLE\s*=\s*\[[^\]]*tabby-electron/)
assert.ok(fs.existsSync(path.join(root, 'tabby-tauri/src/icons/alpine.svg')))
assert.match(bridge, /'desktop\.exec':[\s\S]*?request: \{ executable: string; args: string\[\] \}/)
assert.match(desktop, /pub async fn desktop_exec/)
assert.match(desktop, /Command::new\(&request\.executable\)\s*\.args\(&request\.args\)/)
assert.doesNotMatch(desktop, /Command::new\("(?:sh|bash|cmd|powershell)"\)/)
assert.match(registration, /desktop_exec/)
assert.deepEqual(tauriConfig.app.security.dangerousDisableAssetCspModification, ['style-src'])
assert.ok(capabilities.permissions.includes('notification:default'))
assert.match(tauriEntry, /import ['"]\.\/tauri-polyfills['"]\n/)
assert.match(tauriPolyfills, /setImmediate/)
assert.match(tauriWebpack, /test: \/logo\\\.svg\$\/[\s\S]*?type: 'asset\/resource'/)
assert.match(tauriWebpack, /test: \/\\\.svg\$\/[\s\S]*?svg-inline-loader[\s\S]*?exclude: \/logo\\\.svg\$\//)

console.log('Tauri platform exec and WinSCP fallback contract passed')
