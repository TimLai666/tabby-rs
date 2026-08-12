#!/usr/bin/env node

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const args = process.argv.slice(2)
const argument = name => {
    const index = args.indexOf(name)
    return index === -1 ? null : args[index + 1]
}
const inputPath = path.resolve(args[0] || path.join(root, 'release-assets'))
const outputPath = path.resolve(argument('--output') || path.join(inputPath, 'release-gate-aggregate.json'))
const expectedRevision = argument('--source-revision') || process.env.GITHUB_SHA || null
const expectedChannel = argument('--channel') || null
const expectedTargets = argument('--expected-targets') ? JSON.parse(argument('--expected-targets')) : []
const failures = []

function walk (directory) {
    if (!fs.existsSync(directory)) return []
    const files = []
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
        const filePath = path.join(directory, entry.name)
        if (entry.isDirectory()) files.push(...walk(filePath))
        else if (entry.isFile()) files.push(filePath)
    }
    return files
}

if (!Array.isArray(expectedTargets) || expectedTargets.some(target => typeof target !== 'string' || !target)) {
    failures.push('expected targets must be a non-empty array of strings')
}

const gatePaths = walk(inputPath).filter(filePath => path.basename(filePath) === 'release-gate.json')
const gates = []
for (const gatePath of gatePaths) {
    try {
        const gate = JSON.parse(fs.readFileSync(gatePath, 'utf8'))
        const metadataPath = path.join(path.dirname(gatePath), 'tabby-rs-metadata.json')
        const metadata = fs.existsSync(metadataPath) ? JSON.parse(fs.readFileSync(metadataPath, 'utf8')) : null
        gates.push({
            path: path.relative(inputPath, gatePath).split(path.sep).join('/'),
            passed: gate.passed === true,
            failures: Array.isArray(gate.failures) ? gate.failures : ['release gate has no failure list'],
            sourceRevision: metadata?.revision || gate.sourceRevision || null,
            channel: metadata?.channel || null,
            target: metadata?.target || null,
            platform: metadata?.platform || null,
            arch: metadata?.arch || null,
        })
    } catch (error) {
        failures.push(`invalid release gate ${gatePath}: ${error.message}`)
    }
}

if (gates.length !== expectedTargets.length) {
    failures.push(`expected ${expectedTargets.length} release gates but found ${gates.length}`)
}
const seenTargets = new Set()
for (const gate of gates) {
    if (!gate.passed) failures.push(`${gate.path}: child release gate failed`)
    if (expectedRevision && gate.sourceRevision !== expectedRevision) failures.push(`${gate.path}: source revision mismatch`)
    if (expectedChannel && gate.channel !== expectedChannel) failures.push(`${gate.path}: release channel mismatch`)
    if (!expectedTargets.includes(gate.target)) failures.push(`${gate.path}: unexpected release target ${gate.target || '<missing>'}`)
    if (seenTargets.has(gate.target)) failures.push(`${gate.path}: duplicate release target ${gate.target}`)
    seenTargets.add(gate.target)
}
for (const target of expectedTargets) {
    if (!seenTargets.has(target)) failures.push(`missing release target ${target}`)
}

const report = {
    schemaVersion: 1,
    passed: failures.length === 0,
    sourceRevision: expectedRevision || gates[0]?.sourceRevision || 'local',
    channel: expectedChannel || gates[0]?.channel || null,
    targets: gates.map(gate => ({ target: gate.target, platform: gate.platform, arch: gate.arch, passed: gate.passed })),
    gates,
    failures,
}
fs.mkdirSync(path.dirname(outputPath), { recursive: true })
fs.writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`)
console.log(JSON.stringify(report, null, 2))
if (!report.passed) process.exitCode = 1
