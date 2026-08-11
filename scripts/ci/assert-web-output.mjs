#!/usr/bin/env node
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const outputPath = path.resolve(root, process.argv[2] ?? 'tabby-web/dist/index.js')
const source = await readFile(outputPath, 'utf8')
const forbidden = [
    ['Tauri package', /@tauri-apps[\\/]/i],
    ['Rust source or asset', /(?:src-tauri|\\.rs(?:["'\\/]|$))/i],
    ['native PTY', /node-pty(?:[\\/]|["'])/i],
    ['native serial', /(?:@serialport|serialport)(?:[\\/]|["'])/i],
    ['native font manager', /fontmanager-redux(?:[\\/]|["'])/i],
    ['native keychain', /keytar(?:[\\/]|["'])/i],
    ['desktop updater', /electron-updater(?:[\\/]|["'])/i],
    ['desktop IPC', /electron-promise-ipc(?:[\\/]|["'])/i],
    ['native process helper', /(?:native-process-working-directory|windows-native-registry)(?:[\\/]|["'])/i],
    ['Rust SSH runtime', /(?:^|["'])russh(?:[\\/]|["'])/i],
]

const findings = forbidden
    .filter(([, pattern]) => pattern.test(source))
    .map(([name]) => name)

if (findings.length) {
    throw new Error(`Web bundle contains forbidden runtime references: ${findings.join(', ')}`)
}

console.log(`Web bundle audit passed: ${outputPath}`)
