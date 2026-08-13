#!/usr/bin/env node

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const sourceRoots = [
    'tabby-core/src',
    'tabby-terminal/src',
    'tabby-settings/src',
    'tabby-web/src',
    'web',
]
const sourceExtensions = new Set(['.cjs', '.js', '.mjs', '.ts', '.tsx'])
const forbiddenPatterns = [
    { id: 'tauri-package', pattern: /@tauri-apps[\\/]/ },
    { id: 'tauri-package-source', pattern: /(?:^|["'`])(?:\.\.?[\\/])*tabby-tauri(?:[\\/"'`]|$)/ },
    { id: 'rust-source', pattern: /(?:^|["'`])(?:\.\.?[\\/])*src-tauri(?:[\\/"'`]|$)/ },
]

function walk (directory) {
    if (!fs.existsSync(directory)) return []
    return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
        const filePath = path.join(directory, entry.name)
        if (entry.isDirectory()) return walk(filePath)
        return sourceExtensions.has(path.extname(entry.name)) ? [filePath] : []
    })
}

const findings = []
for (const relativeRoot of sourceRoots) {
    for (const filePath of walk(path.join(root, relativeRoot))) {
        const lines = fs.readFileSync(filePath, 'utf8').split(/\r?\n/)
        lines.forEach((line, index) => {
            for (const rule of forbiddenPatterns) {
                if (rule.pattern.test(line)) {
                    findings.push(`${rule.id}: ${path.relative(root, filePath)}:${index + 1}`)
                }
            }
        })
    }
}

if (findings.length > 0) {
    console.error('Web source boundary violations:')
    for (const finding of findings) console.error(`- ${finding}`)
    process.exit(1)
}

console.log('Web source boundary passed')
