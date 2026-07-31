#!/usr/bin/env node

import * as fs from 'node:fs/promises'
import * as path from 'node:path'
import * as url from 'node:url'

const repositoryRoot = path.resolve(
    path.dirname(url.fileURLToPath(import.meta.url)),
    '../..',
)

const requiredFiles = [
    'app/dist-tauri/index.html',
    'app/dist-tauri/bundle.js',
]

const failures = []
for (const relativePath of requiredFiles) {
    const absolutePath = path.join(repositoryRoot, relativePath)
    try {
        const stat = await fs.stat(absolutePath)
        if (!stat.isFile() || stat.size === 0) {
            failures.push(`${relativePath} is not a non-empty file`)
        } else {
            console.log(`✓ ${relativePath}: ${stat.size} byte(s)`)
        }
    } catch (error) {
        failures.push(`${relativePath} is missing: ${error.message}`)
    }
}

const html = await fs.readFile(path.join(repositoryRoot, 'app/dist-tauri/index.html'), 'utf8').catch(() => '')
if (/\brequire\s*\(/.test(html) || /preload\.js/.test(html)) {
    failures.push('Tauri index.html contains a Node require or Electron preload reference')
}

if (failures.length > 0) {
    console.error('\nTauri output verification failed:')
    for (const failure of failures) {
        console.error(`- ${failure}`)
    }
    process.exit(1)
}

console.log('\nTauri renderer output is present and Node-free at the document boundary.')
