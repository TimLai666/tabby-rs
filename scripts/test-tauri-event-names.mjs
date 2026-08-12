#!/usr/bin/env node

import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'

const root = process.cwd()

function walk (directory, extension) {
    return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
        const entryPath = path.join(directory, entry.name)
        if (entry.isDirectory()) {
            return walk(entryPath, extension)
        }
        return entry.name.endsWith(extension) ? [entryPath] : []
    })
}

function readHostEventKeys (source) {
    const matches = source.matchAll(/interface HostEventMap\s*\{([\s\S]*?)\n\}/g)
    return [...matches].flatMap(match => [...match[1].matchAll(/'([^']+)'\s*:/g)].map(key => key[1]))
}

function assertValidEventNames (names, sourceDescription) {
    for (const name of names) {
        assert.match(name, /^[A-Za-z0-9_:/-]+$/, `${sourceDescription} contains invalid Tauri event name: ${name}`)
        assert.doesNotMatch(name, /\./, `${sourceDescription} contains dotted Tauri event name: ${name}`)
    }
}

const hostBridgeSource = fs.readFileSync(path.join(root, 'tabby-tauri', 'src', 'api', 'hostBridge.ts'), 'utf8')
const ptyBridgeSource = fs.readFileSync(path.join(root, 'tabby-tauri', 'src', 'api', 'ptyBridge.ts'), 'utf8')
const declaredEvents = new Set([
    ...readHostEventKeys(hostBridgeSource),
    ...readHostEventKeys(ptyBridgeSource),
])
assert.ok(declaredEvents.size > 0, 'no Tauri host events were declared')
assertValidEventNames(declaredEvents, 'HostEventMap')

const listenerEvents = new Set()
for (const file of walk(path.join(root, 'tabby-tauri', 'src'), '.ts')) {
    const source = fs.readFileSync(file, 'utf8')
    for (const match of source.matchAll(/\.listen\(\s*['"]([^'"]+)['"]/g)) {
        listenerEvents.add(match[1])
    }
}
assertValidEventNames(listenerEvents, 'Tauri listeners')
for (const event of listenerEvents) {
    assert.ok(declaredEvents.has(event), `listener uses undeclared Tauri event: ${event}`)
}

const emittedEvents = new Set()
for (const file of walk(path.join(root, 'src-tauri', 'src'), '.rs')) {
    const source = fs.readFileSync(file, 'utf8')
    for (const match of source.matchAll(/\.emit\(\s*"([^"]+)"/g)) {
        emittedEvents.add(match[1])
    }
}
assertValidEventNames(emittedEvents, 'Rust emitters')
for (const event of emittedEvents) {
    assert.ok(declaredEvents.has(event), `Rust emitter uses undeclared Tauri event: ${event}`)
}

console.log(`Tauri event name contract passed (${declaredEvents.size} declared, ${listenerEvents.size} listeners, ${emittedEvents.size} emitters)`)
