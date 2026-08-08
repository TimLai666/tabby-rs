#!/usr/bin/env node

import { readdir, readFile } from 'node:fs/promises'
import path from 'node:path'

const root = process.cwd()
const violations = []

async function walk (directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
        if (entry.name === 'node_modules' || entry.name === 'dist' || entry.name === 'typings') {
            continue
        }
        const fullPath = path.join(directory, entry.name)
        if (entry.isDirectory()) {
            await walk(fullPath)
            continue
        }
        if (!entry.isFile() || !/\.(?:ts|tsx|js|mjs|cjs)$/.test(entry.name)) {
            continue
        }

        const relative = path.relative(root, fullPath).split(path.sep).join('/')
        if (!relative.includes('/src/')) {
            continue
        }
        if (relative.startsWith('tabby-terminal/src/renderer/')) {
            continue
        }

        const source = await readFile(fullPath, 'utf8')
        if (/\b(?:from\s*|require\s*\()\s*['"]@xterm\//.test(source)) {
            violations.push(relative)
        }
    }
}

for (const entry of await readdir(root, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.startsWith('tabby-')) {
        continue
    }
    await walk(path.join(root, entry.name))
}

if (violations.length) {
    console.error('xterm.js imports must stay inside tabby-terminal/src/renderer/:')
    for (const violation of violations.sort()) {
        console.error(`  - ${violation}`)
    }
    process.exitCode = 1
} else {
    console.log('xterm.js dependency boundary is clean.')
}
