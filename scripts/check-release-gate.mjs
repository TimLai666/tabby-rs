import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import yaml from 'js-yaml'

import { BENCHMARK_METRICS, validateBenchmarkReport } from './benchmark/schema.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const args = process.argv.slice(2)
const argument = name => {
    const index = args.indexOf(name)
    return index === -1 ? null : args[index + 1]
}
const bundleAuditPath = path.resolve(argument('--bundle-audit') || path.join(root, 'bundle-audit.json'))
const licenseReportPath = path.resolve(argument('--license-report') || path.join(root, 'license-report.json'))
const benchmarksDirectory = path.resolve(argument('--benchmarks-dir') || path.join(root, 'benchmarks'))
const outputPath = path.resolve(argument('--output') || path.join(root, 'release-gate.json'))
const failures = []
const benchmarkFiles = Object.entries(BENCHMARK_METRICS).map(([name, metric]) => ({
    name,
    metric,
    path: path.join(benchmarksDirectory, `${name}.json`),
}))

function readYaml (relativePath) {
    const filePath = path.join(root, relativePath)
    if (!fs.existsSync(filePath)) {
        failures.push(`missing ${relativePath}`)
        return null
    }
    try {
        return yaml.load(fs.readFileSync(filePath, 'utf8'))
    } catch (error) {
        failures.push(`invalid ${relativePath}: ${error.message}`)
        return null
    }
}

const featuresDocument = readYaml('parity/features.yaml')
const platformDocument = readYaml('parity/platform-matrix.yaml')
if (!Array.isArray(featuresDocument?.features) || featuresDocument.features.length === 0) {
    failures.push('parity/features.yaml has no features')
}
if (!Array.isArray(platformDocument?.platforms) || platformDocument.platforms.length === 0) {
    failures.push('parity/platform-matrix.yaml has no platforms')
}
const featureStatuses = new Set(['passed', 'accepted-difference'])
for (const feature of featuresDocument?.features || []) {
    if (!featureStatuses.has(feature.status)) {
        failures.push(`feature ${feature.id || '<unnamed>'} is ${feature.status || 'missing status'}`)
    }
    if (feature.status === 'accepted-difference' && (!feature.reason || !feature.evidence?.length)) {
        failures.push(`feature ${feature.id} accepted-difference lacks reason or evidence`)
    }
    if (feature.status === 'passed' && !feature.evidence?.length) {
        failures.push(`feature ${feature.id} has no evidence`)
    }
}
for (const platform of platformDocument?.platforms || []) {
    if (platform.status !== 'passed') {
        failures.push(`platform ${platform.id || '<unnamed>'} is ${platform.status || 'missing status'}`)
    }
    if (!platform.evidence?.length) {
        failures.push(`platform ${platform.id} has no evidence`)
    }
}
for (const benchmark of benchmarkFiles) {
    if (!fs.existsSync(benchmark.path)) {
        failures.push(`missing benchmark report: ${benchmark.path}`)
        continue
    }
    try {
        const report = JSON.parse(fs.readFileSync(benchmark.path, 'utf8'))
        for (const error of validateBenchmarkReport(report, benchmark.metric)) {
            failures.push(`${benchmark.name}: ${error}`)
        }
    } catch (error) {
        failures.push(`invalid benchmark report ${benchmark.path}: ${error.message}`)
    }
}

if (!fs.existsSync(bundleAuditPath)) {
    failures.push(`missing bundle audit: ${bundleAuditPath}`)
} else {
    const bundleAudit = JSON.parse(fs.readFileSync(bundleAuditPath, 'utf8'))
    if (bundleAudit.passed !== true) {
        failures.push('bundle audit did not pass')
    }
}

if (!fs.existsSync(licenseReportPath)) {
    failures.push(`missing license report: ${licenseReportPath}`)
} else if (JSON.parse(fs.readFileSync(licenseReportPath, 'utf8')).passed !== true) {
    failures.push('license report did not pass')
}

const report = {
    schemaVersion: 1,
    passed: failures.length === 0,
    failures,
    sourceRevision: process.env.GITHUB_SHA || 'local',
}
fs.mkdirSync(path.dirname(outputPath), { recursive: true })
fs.writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`)
console.log(JSON.stringify(report, null, 2))
if (!report.passed) {
    process.exitCode = 1
}
