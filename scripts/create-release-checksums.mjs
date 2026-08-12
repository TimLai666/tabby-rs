#!/usr/bin/env node

import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'

const args = process.argv.slice(2)
const argument = name => {
    const index = args.indexOf(name)
    return index === -1 ? null : args[index + 1]
}
const stagingArgument = args[0] && !args[0].startsWith('--') ? args[0] : null
const staging = path.resolve(stagingArgument || process.env.TABBY_RS_RELEASE_STAGING || 'release-staging')
const output = path.resolve(argument('--output') || path.join(staging, 'SHA256SUMS'))

if (!fs.existsSync(staging) || !fs.statSync(staging).isDirectory()) {
    throw new Error(`release staging directory does not exist: ${staging}`)
}

function walk (directory) {
    return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
        const filePath = path.join(directory, entry.name)
        if (entry.isDirectory()) return walk(filePath)
        return entry.isFile() ? [filePath] : []
    })
}

const files = walk(staging)
    .filter(filePath => path.resolve(filePath) !== output)
    .map(filePath => ({
        filePath,
        relativePath: path.relative(staging, filePath).split(path.sep).join('/'),
    }))
    .sort((left, right) => left.relativePath.localeCompare(right.relativePath))

if (files.length === 0) {
    throw new Error(`release staging directory contains no files: ${staging}`)
}

const lines = files.map(({ filePath, relativePath }) => {
    const digest = crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex')
    return `${digest}  ${relativePath}`
})
fs.writeFileSync(output, `${lines.join('\n')}\n`)
console.log(`Created ${output} for ${files.length} release file(s)`)
