#!/usr/bin/env node

import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const script = path.join(root, 'scripts/create-release-checksums.mjs')
const staging = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-release-checksums-'))
const output = path.join(staging, 'SHA256SUMS')

try {
    fs.mkdirSync(path.join(staging, 'nested'))
    fs.writeFileSync(path.join(staging, 'artifact.exe'), 'installer')
    fs.writeFileSync(path.join(staging, 'nested', 'manifest.json'), '{"version":1}\n')

    execFileSync(process.execPath, [script, staging, '--output', output], { encoding: 'utf8' })
    const first = fs.readFileSync(output, 'utf8')
    assert.deepEqual(first.trim().split('\n').map(line => line.slice(66)), [
        'artifact.exe',
        'nested/manifest.json',
    ])
    assert.match(first, new RegExp(`${crypto.createHash('sha256').update('installer').digest('hex')}  artifact\.exe`))
    assert.doesNotMatch(first, /SHA256SUMS/)

    execFileSync(process.execPath, [script, staging, '--output', output], { encoding: 'utf8' })
    assert.equal(fs.readFileSync(output, 'utf8'), first)
} finally {
    fs.rmSync(staging, { recursive: true, force: true })
}

console.log('Release checksum fixtures passed')
