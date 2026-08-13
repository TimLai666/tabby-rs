#!/usr/bin/env node
import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'

const execFileAsync = promisify(execFile)
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const checker = path.join(root, 'scripts/ci/assert-web-output.mjs')
const fixture = await mkdtemp(path.join(os.tmpdir(), 'tabby-web-audit-'))
const nested = path.join(fixture, 'chunks')
const entry = path.join(fixture, 'index.js')
const chunk = path.join(nested, 'chunk.js')

try {
    await mkdir(nested)
    await writeFile(entry, 'export const entry = true\n')
    await writeFile(chunk, 'export const chunk = true\n')

    const singleFileResult = await execFileAsync(process.execPath, [checker, entry])
    assert.match(singleFileResult.stdout, /1 JavaScript file\(s\)/)

    await writeFile(chunk, 'import { invoke } from "@tauri-apps/api/core"\n')
    await assert.rejects(
        execFileAsync(process.execPath, [checker, fixture]),
        /Tauri package in .*chunks[\\/]chunk\.js/,
    )

    await writeFile(chunk, 'export const chunk = true\n')
    const directoryResult = await execFileAsync(process.execPath, [checker, fixture])
    assert.match(directoryResult.stdout, /2 JavaScript file\(s\)/)
    assert.equal(await readFile(entry, 'utf8'), 'export const entry = true\n')

    await writeFile(chunk, "Tabby.registerMock('keytar', { getPassword: () => null })\nTabby.registerMock('@serialport/bindings', {})\n")
    const compatibilityMockResult = await execFileAsync(process.execPath, [checker, fixture])
    assert.match(compatibilityMockResult.stdout, /2 JavaScript file\(s\)/)

    await writeFile(chunk, "const nativeKeychain = require('keytar')\n")
    await assert.rejects(
        execFileAsync(process.execPath, [checker, fixture]),
        /native keychain in .*chunks[\\/]chunk\.js/,
    )
} finally {
    await rm(fixture, { recursive: true, force: true })
}

console.log('Web output audit contract tests passed')
