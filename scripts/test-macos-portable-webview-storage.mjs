#!/usr/bin/env node
// Compile the production adapter and exercise actual WebKit in owned .app helpers.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, renameSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { randomUUID } from 'node:crypto'

if (process.argv.includes('--help')) {
  console.log('Usage: node scripts/test-macos-portable-webview-storage.mjs\nRuns native storage guards, persistent WebViews, directory isolation and relocation on macOS.\nOther platforms skip. Uses clang, codesign and temporary owned app/data folders; removes them afterward.')
  process.exit(0)
}
assert.equal(process.argv.length, 2, 'unexpected arguments; use --help')
if (process.platform !== 'darwin') {
  console.log('macOS portable WebView storage: skipped (requires WebKit)')
  process.exit(0)
}
const temporary = mkdtempSync(join(tmpdir(), 'tabby-webview-storage-'))
const marker = 'TABBY_STORAGE_NATIVE_EVIDENCE '
const run = (command, args) => {
  const result = spawnSync(command, args, { encoding: 'utf8', timeout: 20000, maxBuffer: 4 * 1024 * 1024 })
  assert.ifError(result.error)
  assert.equal(result.status, 0, command + ': ' + result.stdout + result.stderr)
  return result.stdout
}
let passed = 0
try {
  for (const legacy of [false, true]) {
    let portable = join(temporary, legacy ? 'legacy' : 'automatic')
    const appName = 'Storage.app'
    const executableName = 'WebKitStorageProbe' // Reject unrelated IMPs even when their image contains WebKit.
    const app = join(portable, appName)
    const macos = join(app, 'Contents', 'MacOS')
    mkdirSync(macos, { recursive: true })
    const identifier = 'dev.tabbyrs.storage-test.' + randomUUID()
    writeFileSync(join(app, 'Contents', 'Info.plist'), `<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>${identifier}</string><key>CFBundleExecutable</key><string>${executableName}</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>`)
    const executable = join(macos, executableName)
    run('/usr/bin/clang', [
      '-fobjc-arc', '-fmodules', '-Wall', '-Wextra', '-framework', 'Cocoa', '-framework', 'WebKit',
      ...(legacy ? ['-DTABBY_RS_TEST_LEGACY_STORAGE'] : []),
      resolve('src-tauri/src/platform/macos_webview_storage.m'),
      resolve('scripts/fixtures/macos-portable-webview-storage.m'), '-o', executable,
    ])
    run('/usr/bin/codesign', ['--force', '--sign', '-', app])
    run('/usr/bin/codesign', ['--verify', '--strict', app])
    const key = 'owned-storage-' + randomUUID()
    const valueA = 'marker-a-' + randomUUID()
    const valueB = 'marker-b-' + randomUUID()
    for (const name of ['data-a', 'data-b']) mkdirSync(join(portable, name))
    const invoke = (name, data, expected) => {
      const stdout = run(join(portable, appName, 'Contents', 'MacOS', executableName), [name, join(portable, data), key, expected ?? ''])
      const records = stdout.split('\n').filter(line => line.startsWith(marker))
      assert.equal(records.length, 1, 'expected one complete evidence marker: ' + stdout)
      const evidence = JSON.parse(records[0].slice(marker.length))
      assert.equal(evidence.case, name)
      assert.equal(evidence.passed, true, JSON.stringify(evidence))
      if (name === 'read' || name === 'write') {
        assert.equal(evidence.before, name === 'write' ? null : expected)
        assert.equal(evidence.after, expected)
        assert.equal(evidence.secondary, expected)
      }
      passed++
      console.log('PASS', legacy ? 'legacy' : 'automatic', name, data)
    }
    for (const name of ['guards', 'conflicting-hook', 'repeat-install']) invoke(name, 'data-a')
    invoke('write', 'data-a', valueA)
    invoke('read', 'data-a', valueA)
    invoke('read', 'data-b', null)
    invoke('write', 'data-b', valueB)
    invoke('read', 'data-a', valueA)
    const relocated = portable + '-moved'
    renameSync(portable, relocated)
    portable = relocated
    invoke('read', 'data-a', valueA)
    invoke('read', 'data-b', valueB)
  }
  console.log('macOS portable WebView storage:', passed + '/20 passed')
} finally {
  rmSync(temporary, { recursive: true, force: true })
}
