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

if (!Array.isArray(expectedTargets) || expectedTargets.length === 0 || expectedTargets.some(target => typeof target !== 'string' || !target)) {
    failures.push('expected targets must be a non-empty array of strings')
}

const gatePaths = walk(inputPath).filter(filePath => path.basename(filePath) === 'release-gate.json')
const gates = []
for (const gatePath of gatePaths) {
    try {
        const gate = JSON.parse(fs.readFileSync(gatePath, 'utf8'))
        const metadataPath = path.join(path.dirname(gatePath), 'tabby-rs-metadata.json')
        let metadata = null
        if (!fs.existsSync(metadataPath)) {
            failures.push(`${path.relative(inputPath, gatePath)}: missing release metadata`)
        } else {
            try {
                metadata = JSON.parse(fs.readFileSync(metadataPath, 'utf8'))
            } catch (error) {
                failures.push(`invalid release metadata ${metadataPath}: ${error.message}`)
            }
        }
        gates.push({
            path: path.relative(inputPath, gatePath).split(path.sep).join('/'),
            schemaVersion: gate.schemaVersion,
            passed: gate.passed === true,
            hasFailureList: Array.isArray(gate.failures),
            failures: Array.isArray(gate.failures) ? gate.failures : ['release gate has no failure list'],
            sourceRevision: metadata?.revision || gate.sourceRevision || null,
            channel: metadata?.channel || null,
            version: metadata?.version || null,
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
const seenVersions = new Set()
for (const gate of gates) {
    if (gate.schemaVersion !== 1) failures.push(`${gate.path}: release gate schema version is invalid`)
    if (!gate.passed) failures.push(`${gate.path}: child release gate failed`)
    if (!gate.hasFailureList) failures.push(`${gate.path}: child release gate has no failure list`)
    if (expectedRevision && gate.sourceRevision !== expectedRevision) failures.push(`${gate.path}: source revision mismatch`)
    if (expectedChannel && gate.channel !== expectedChannel) failures.push(`${gate.path}: release channel mismatch`)
    if (typeof gate.version !== 'string' || gate.version.length === 0) failures.push(`${gate.path}: release version is missing`)
    if (typeof gate.platform !== 'string' || gate.platform.length === 0) failures.push(`${gate.path}: release platform is missing`)
    if (typeof gate.arch !== 'string' || gate.arch.length === 0) failures.push(`${gate.path}: release architecture is missing`)
    if (typeof gate.target !== 'string' || gate.target.length === 0) failures.push(`${gate.path}: release target is missing`)
    if (!expectedTargets.includes(gate.target)) failures.push(`${gate.path}: unexpected release target ${gate.target || '<missing>'}`)
    if (seenTargets.has(gate.target)) failures.push(`${gate.path}: duplicate release target ${gate.target}`)
    seenTargets.add(gate.target)
    if (gate.version) seenVersions.add(gate.version)
}
for (const target of expectedTargets) {
    if (!seenTargets.has(target)) failures.push(`missing release target ${target}`)
}
if (seenVersions.size > 1) failures.push(`release versions do not match: ${[...seenVersions].sort().join(', ')}`)

const report = {
    schemaVersion: 1,
    passed: failures.length === 0,
    sourceRevision: expectedRevision || gates[0]?.sourceRevision || 'local',
    channel: expectedChannel || gates[0]?.channel || null,
    version: seenVersions.size === 1 ? [...seenVersions][0] : null,
    targets: gates.map(gate => ({ target: gate.target, platform: gate.platform, arch: gate.arch, passed: gate.passed })),
    gates,
    failures,
}
fs.mkdirSync(path.dirname(outputPath), { recursive: true })
fs.writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`)
console.log(JSON.stringify(report, null, 2))
if (!report.passed) process.exitCode = 1
