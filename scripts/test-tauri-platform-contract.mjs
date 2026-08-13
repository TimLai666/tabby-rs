import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const platform = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/platform.service.ts'), 'utf8')
const bridge = fs.readFileSync(path.join(root, 'tabby-tauri/src/api/hostBridge.ts'), 'utf8')
const desktop = fs.readFileSync(path.join(root, 'src-tauri/src/commands/desktop.rs'), 'utf8')
const registration = fs.readFileSync(path.join(root, 'src-tauri/src/lib.rs'), 'utf8')

assert.match(platform, /async exec \(app: string, argv: string\[\]\): Promise<void> \{[\s\S]*?desktop\.exec/)
assert.match(platform, /getWinSCPPath \(\): string \| null \{\s*return null\s*\}/)
assert.match(bridge, /'desktop\.exec':[\s\S]*?request: \{ executable: string; args: string\[\] \}/)
assert.match(desktop, /pub async fn desktop_exec/)
assert.match(desktop, /Command::new\(&request\.executable\)\s*\.args\(&request\.args\)/)
assert.doesNotMatch(desktop, /Command::new\("(?:sh|bash|cmd|powershell)"\)/)
assert.match(registration, /desktop_exec/)

console.log('Tauri platform exec and WinSCP fallback contract passed')
