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
const installerSmokePath = path.resolve(argument('--installer-smoke') || path.join(root, 'installer-smoke.json'))
const outputPath = path.resolve(argument('--output') || path.join(root, 'release-gate.json'))
const expectedRevision = argument('--source-revision') || process.env.GITHUB_SHA || null
const expectedPlatform = argument('--platform') || null
const expectedArch = argument('--arch') || null
const expectedTarget = argument('--target') || null
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

function readJson (filePath, label) {
    if (!fs.existsSync(filePath)) {
        failures.push(`missing ${label}: ${filePath}`)
        return null
    }
    try {
        return JSON.parse(fs.readFileSync(filePath, 'utf8'))
    } catch (error) {
        failures.push(`invalid ${label} ${filePath}: ${error.message}`)
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
        const errors = validateBenchmarkReport(report, benchmark.metric, { requireLargeOutput: true })
        if (expectedRevision && report.commit !== expectedRevision) errors.push(`commit must match ${expectedRevision}`)
        if (expectedPlatform && report.platform !== expectedPlatform) errors.push(`platform must match ${expectedPlatform}`)
        if (expectedArch && report.arch !== expectedArch) errors.push(`arch must match ${expectedArch}`)
        for (const error of errors) {
            failures.push(`${benchmark.name}: ${error}`)
        }
        benchmark.report = report
    } catch (error) {
        failures.push(`invalid benchmark report ${benchmark.path}: ${error.message}`)
    }
}

const bundleAudit = readJson(bundleAuditPath, 'bundle audit')
if (bundleAudit) {
    if (bundleAudit.passed !== true) {
        failures.push('bundle audit did not pass')
    }
    if (expectedRevision && bundleAudit.sourceRevision !== expectedRevision) failures.push(`bundle audit sourceRevision must match ${expectedRevision}`)
    if (expectedPlatform && bundleAudit.platform !== expectedPlatform) failures.push(`bundle audit platform must match ${expectedPlatform}`)
    if (expectedArch && bundleAudit.arch !== expectedArch) failures.push(`bundle audit arch must match ${expectedArch}`)
    if (expectedTarget && bundleAudit.target !== expectedTarget) failures.push(`bundle audit target must match ${expectedTarget}`)
}

const licenseReport = readJson(licenseReportPath, 'license report')
if (licenseReport && licenseReport.passed !== true) {
    failures.push('license report did not pass')
}
if (licenseReport && expectedRevision && licenseReport.sourceRevision !== expectedRevision) {
    failures.push(`license report sourceRevision must match ${expectedRevision}`)
}

const installerSmoke = readJson(installerSmokePath, 'installer smoke report')
if (installerSmoke && installerSmoke.passed !== true) {
    failures.push('installer smoke did not pass')
}
if (installerSmoke && expectedPlatform && installerSmoke.platform !== expectedPlatform) {
    failures.push(`installer smoke platform must match ${expectedPlatform}`)
}

if (bundleAudit) {
    const metadataPath = path.join(path.dirname(bundleAuditPath), 'tabby-rs-metadata.json')
    const metadata = readJson(metadataPath, 'release metadata')
    if (metadata) {
        if (!metadata.dependencyLocks || typeof metadata.dependencyLocks !== 'object') {
            failures.push('release metadata has no dependency lock hashes')
        } else {
            for (const lockFile of ['yarn.lock', 'src-tauri/Cargo.lock']) {
                if (!/^[0-9a-f]{64}$/i.test(metadata.dependencyLocks[lockFile] || '')) {
                    failures.push(`release metadata dependency lock hash is invalid: ${lockFile}`)
                }
            }
        }
        if (!metadata.toolchain || typeof metadata.toolchain !== 'object') failures.push('release metadata has no toolchain versions')
    }
}

const validBenchmarkReports = benchmarkFiles.map(benchmark => benchmark.report).filter(Boolean)
if (validBenchmarkReports.length > 1) {
    const fixtureSha256 = validBenchmarkReports[0].fixtureSha256
    const artifactSha256 = validBenchmarkReports[0].artifactSha256
    for (const report of validBenchmarkReports.slice(1)) {
        if (report.fixtureSha256 !== fixtureSha256) failures.push('benchmark reports must use one config fixture')
        if (report.artifactSha256 !== artifactSha256) failures.push('benchmark reports must use one bundle artifact')
    }
    if (bundleAudit?.artifactSha256 && artifactSha256 !== bundleAudit.artifactSha256) {
        failures.push('benchmark reports must match bundle audit artifactSha256')
    }
}

const report = {
    schemaVersion: 1,
    passed: failures.length === 0,
    failures,
    sourceRevision: expectedRevision || 'local',
}
fs.mkdirSync(path.dirname(outputPath), { recursive: true })
fs.writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`)
console.log(JSON.stringify(report, null, 2))
if (!report.passed) {
    process.exitCode = 1
}
