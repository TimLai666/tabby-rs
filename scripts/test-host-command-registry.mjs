#!/usr/bin/env node
// Every HostRequestMap key must map, through the bridge's dotted-camelCase to
// snake_case conversion, to a command registered in src-tauri/src/lib.rs.
// Renderer fixtures use fake bridges and never notice an unregistered command;
// the real IPC rejects it (found by review: window.canHideOnBlur vs
// window_dock_hide_check).
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const bridgeSource = fs.readFileSync(path.join(root, 'tabby-tauri/src/api/hostBridge.ts'), 'utf8')
const libSource = fs.readFileSync(path.join(root, 'src-tauri/src/lib.rs'), 'utf8')
const bridgeService = fs.readFileSync(path.join(root, 'tabby-tauri/src/services/tauriHostBridge.service.ts'), 'utf8')

// Compile the actual conversion rather than re-typing it.
const conversion = /function toRustCommand \(command: string\): string \{[\s\S]*?\n\}/.exec(bridgeService)
assert.ok(conversion, 'toRustCommand must exist in the Tauri host bridge')
const toRustCommand = new Function(`${ts.transpileModule(conversion[0], { compilerOptions: { target: ts.ScriptTarget.ES2020 } }).outputText}; return toRustCommand`)()

const tree = ts.createSourceFile('hostBridge.ts', bridgeSource, ts.ScriptTarget.Latest, true)
const map = tree.statements.find(s => ts.isInterfaceDeclaration(s) && s.name.text === 'HostRequestMap')
assert.ok(map, 'HostRequestMap must exist')
const keys = map.members.map(member => member.name.getText(tree).replace(/^['"]|['"]$/g, ''))
assert.ok(keys.length > 100, `expected the full request map, found ${keys.length} keys`)

const handler = /generate_handler!\[([\s\S]*?)\]/.exec(libSource)
assert.ok(handler, 'generate_handler! must exist in lib.rs')
const registered = new Set(handler[1].split(',').map(name => name.trim()).filter(Boolean))

const missing = keys.map(key => [key, toRustCommand(key)]).filter(([, rust]) => !registered.has(rust))
assert.deepEqual(missing, [], `HostRequestMap keys without a registered Tauri command: ${JSON.stringify(missing)}`)
console.log(`host command registry: ${keys.length} request keys resolve to registered commands (${registered.size} registered)`)
