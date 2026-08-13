#!/usr/bin/env node
import { readdir, readFile, stat } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const outputPaths = (process.argv.length > 2 ? process.argv.slice(2) : ['tabby-web/dist', 'web/dist'])
    .map(target => path.resolve(root, target))

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

const outputFiles = []
for (const outputPath of outputPaths) {
    const files = await collectJavaScriptFiles(outputPath)
    if (!files.length) {
        throw new Error(`Web bundle audit found no JavaScript files under ${outputPath}`)
    }
    outputFiles.push(...files)
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
    // The browser compatibility layer registers inert mocks for native-only
    // modules so legacy plugins can load without pulling native code into the
    // bundle. Keep those exact registration names allowed, but still reject
    // imports/requires and any other occurrence of the native package names.
    const auditableSource = source.replace(
        /Tabby\.registerMock\(\s*(['"])(?:keytar|@serialport\/bindings(?:-cpp)?)\1/g,
        'Tabby.registerMock(__allowed_web_mock__',
    )
    for (const [name, pattern] of forbidden) {
        if (pattern.test(auditableSource)) {
            findings.push(`${name} in ${path.relative(root, file)}`)
        }
    }
}

if (findings.length) {
    throw new Error(`Web bundle contains forbidden runtime references: ${findings.join('; ')}`)
}

console.log(`Web bundle audit passed: ${outputFiles.length} JavaScript file(s) across ${outputPaths.length} target(s)`)
