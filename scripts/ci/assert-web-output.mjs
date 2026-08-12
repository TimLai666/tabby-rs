#!/usr/bin/env node
import { readdir, readFile, stat } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const outputPath = path.resolve(root, process.argv[2] ?? 'tabby-web/dist')

async function collectJavaScriptFiles(target) {
    const targetStats = await stat(target)
    if (targetStats.isFile()) {
        return [target]
    }
    if (!targetStats.isDirectory()) {
        throw new Error(`Web bundle audit target is not a file or directory: ${target}`)
    }

    const files = []
    for (const entry of await readdir(target, { withFileTypes: true })) {
        const entryPath = path.join(target, entry.name)
        if (entry.isDirectory()) {
            files.push(...await collectJavaScriptFiles(entryPath))
        } else if (entry.isFile() && /\.(?:c|m)?js$/i.test(entry.name)) {
            files.push(entryPath)
        }
    }
    return files.sort()
}

const outputFiles = await collectJavaScriptFiles(outputPath)
if (!outputFiles.length) {
    throw new Error(`Web bundle audit found no JavaScript files under ${outputPath}`)
}

const forbidden = [
    ['Tauri package', /@tauri-apps[\\/]/i],
    ['Rust source or asset', /(?:src-tauri|\.rs(?:["'\\/]|$))/i],
    ['native PTY', /node-pty(?:[\\/]|["'])/i],
    ['native serial', /(?:@serialport|serialport)(?:[\\/]|["'])/i],
    ['native font manager', /fontmanager-redux(?:[\\/]|["'])/i],
    ['native keychain', /keytar(?:[\\/]|["'])/i],
    ['desktop updater', /electron-updater(?:[\\/]|["'])/i],
    ['desktop IPC', /electron-promise-ipc(?:[\\/]|["'])/i],
    ['native process helper', /(?:native-process-working-directory|windows-native-registry)(?:[\\/]|["'])/i],
    ['Rust SSH runtime', /(?:^|["'])russh(?:[\\/]|["'])/i],
]

const findings = []
for (const file of outputFiles) {
    const source = await readFile(file, 'utf8')
    for (const [name, pattern] of forbidden) {
        if (pattern.test(source)) {
            findings.push(`${name} in ${path.relative(root, file)}`)
        }
    }
}

if (findings.length) {
    throw new Error(`Web bundle contains forbidden runtime references: ${findings.join('; ')}`)
}

console.log(`Web bundle audit passed: ${outputFiles.length} JavaScript file(s) under ${outputPath}`)
