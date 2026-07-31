#!/usr/bin/env node

import * as fs from 'node:fs/promises'
import * as path from 'node:path'
import * as url from 'node:url'
import { allPackages } from '../vars.mjs'

const repositoryRoot = path.resolve(
    path.dirname(url.fileURLToPath(import.meta.url)),
    '../..',
)

async function countFiles (directory) {
    let entries
    try {
        entries = await fs.readdir(directory, { withFileTypes: true })
    } catch (error) {
        if (error?.code === 'ENOENT') {
            return 0
        }
        throw error
    }

    let count = 0
    for (const entry of entries) {
        const entryPath = path.join(directory, entry.name)
        if (entry.isDirectory()) {
            count += await countFiles(entryPath)
        } else if (entry.isFile() || entry.isSymbolicLink()) {
            count++
        }
    }
    return count
}

const buildTargets = ['app', ...allPackages]
const failures = []

for (const target of buildTargets) {
    const relativeDirectory = path.join(target, 'dist')
    const absoluteDirectory = path.join(repositoryRoot, relativeDirectory)
    const fileCount = await countFiles(absoluteDirectory)

    if (fileCount === 0) {
        failures.push(`${relativeDirectory} is missing or empty`)
        continue
    }

    console.log(`✓ ${relativeDirectory}: ${fileCount} generated file(s)`)
}

if (failures.length > 0) {
    console.error('\nBuild output verification failed:')
    for (const failure of failures) {
        console.error(`- ${failure}`)
    }
    process.exit(1)
}

console.log(`\nVerified ${buildTargets.length} build output directories.`)
