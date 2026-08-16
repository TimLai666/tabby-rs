import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'

const root = process.cwd()
const cargo = fs.readFileSync(path.join(root, 'src-tauri/Cargo.toml'), 'utf8')
const platform = fs.readFileSync(path.join(root, 'src-tauri/src/platform/mod.rs'), 'utf8')
const desktop = fs.readFileSync(path.join(root, 'src-tauri/src/desktop.rs'), 'utf8')

assert.match(
    cargo,
    /windows-sys\s*=\s*\{[\s\S]*?Win32_Storage_FileSystem[\s\S]*?Win32_UI_WindowsAndMessaging[\s\S]*?\}/,
    'Windows opacity needs the user32 window-style APIs',
)
assert.match(
    cargo,
    /objc2-app-kit\s*=\s*\{[\s\S]*?NSWindow[\s\S]*?\}/,
    'macOS opacity needs the AppKit NSWindow binding',
)
assert.match(platform, /\.run_on_main_thread\(/, 'native opacity must run on the UI thread')
assert.match(platform, /setAlphaValue\(/, 'macOS opacity must update NSWindow alphaValue')
assert.match(platform, /SetLayeredWindowAttributes\(/, 'Windows opacity must update layered-window alpha')
assert.match(platform, /WS_EX_LAYERED/, 'Windows opacity must preserve the layered-window style')
assert.match(
    desktop,
    /opacity:\s*cfg!\(any\(target_os\s*=\s*"windows",\s*target_os\s*=\s*"macos"\)\)/,
    'opacity capability must match native platform support',
)
assert.match(
    platform,
    /target_os\s*=\s*"linux"[\s\S]*?window opacity is unavailable on Linux/,
    'Linux opacity must remain an explicit unsupported capability',
)

console.log('Tauri native window opacity contract passed')
