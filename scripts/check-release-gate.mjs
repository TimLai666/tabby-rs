import fs from 'node:fs'
import crypto from 'node:crypto'
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
const dependencyAuditPath = path.resolve(argument('--dependency-audit') || path.join(root, 'dependency-audit.json'))
const licenseReportPath = path.resolve(argument('--license-report') || path.join(root, 'license-report.json'))
const benchmarksDirectory = path.resolve(argument('--benchmarks-dir') || path.join(root, 'benchmarks'))
const installerSmokePath = path.resolve(argument('--installer-smoke') || path.join(root, 'installer-smoke.json'))
const parityReportPath = path.resolve(argument('--parity-report') || path.join(root, 'parity-report.json'))
const parityEvidencePath = argument('--parity-evidence') ? path.resolve(argument('--parity-evidence')) : null
const outputPath = path.resolve(argument('--output') || path.join(root, 'release-gate.json'))
const expectedRevision = argument('--source-revision') || process.env.GITHUB_SHA || null
const expectedPlatform = argument('--platform') || null
const expectedArch = argument('--arch') || null
const expectedTarget = argument('--target') || null
const failures = []
const sha256Pattern = /^[0-9a-f]{64}$/i
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

function sha256 (filePath) {
    return crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex')
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

const parityReport = readJson(parityReportPath, 'parity report')
if (parityReport) {
    if (parityReport.schemaVersion !== 1) failures.push('parity report schema version is invalid')
    if (parityReport.passed !== true) failures.push('parity report did not pass')
    if (!Array.isArray(parityReport.failures)) failures.push('parity report has no failures list')
    else if (parityReport.failures.length > 0) failures.push('parity report has failures')
    for (const [name, summary] of [['features', parityReport.features], ['platforms', parityReport.platforms]]) {
        if (!summary || typeof summary !== 'object') {
            failures.push(`parity report has no ${name} summary`)
            continue
        }
        if (!Number.isInteger(summary.total) || summary.total < 1) failures.push(`parity report ${name} total is invalid`)
        if (!summary.statuses || typeof summary.statuses !== 'object') {
            failures.push(`parity report ${name} has no statuses`)
            continue
        }
        if (!Array.isArray(summary.pending)) failures.push(`parity report ${name} has no pending list`)
        else if (summary.pending.length > 0) failures.push(`parity report ${name} has pending entries`)
        const nonPassingStatuses = Object.entries(summary.statuses)
            .filter(([status, count]) => count > 0 && !['passed', 'accepted-difference'].includes(status))
            .map(([status]) => status)
        if (nonPassingStatuses.length > 0) {
            failures.push(`parity report ${name} has non-passing statuses: ${nonPassingStatuses.join(', ')}`)
        }
    }
}

if (!parityEvidencePath) failures.push('missing parity automated evidence path')
const parityEvidence = parityEvidencePath ? readJson(parityEvidencePath, 'parity automated evidence') : null
if (parityEvidencePath && parityEvidence) {
    if (parityEvidence.schemaVersion !== 1 || parityEvidence.kind !== 'tabby-rs-parity-automated-evidence') {
        failures.push('parity automated evidence schema is invalid')
    }
    if (parityEvidence.passed !== true) failures.push('parity automated evidence did not pass')
    if (expectedRevision && parityEvidence.sourceRevision !== expectedRevision) failures.push(`parity automated evidence sourceRevision must match ${expectedRevision}`)
    if (expectedPlatform && parityEvidence.platform !== expectedPlatform) failures.push(`parity automated evidence platform must match ${expectedPlatform}`)
    if (expectedArch && parityEvidence.arch !== expectedArch) failures.push(`parity automated evidence arch must match ${expectedArch}`)
    if (expectedTarget && parityEvidence.target !== expectedTarget) failures.push(`parity automated evidence target must match ${expectedTarget}`)
    const expectedAutomatedChecks = new Set()
    for (const feature of featuresDocument?.features || []) {
        if (!expectedPlatform || !(feature.platforms || []).includes(expectedPlatform)) continue
        for (const check of feature.tests?.automated || []) expectedAutomatedChecks.add(check)
    }
    const manifestExpectedChecks = [...expectedAutomatedChecks].sort()
    if (!Array.isArray(parityEvidence.expectedChecks) || parityEvidence.expectedChecks.length === 0) failures.push('parity automated evidence has no expected checks')
    else if (JSON.stringify([...new Set(parityEvidence.expectedChecks)].sort()) !== JSON.stringify(manifestExpectedChecks)) failures.push('parity automated evidence expected checks do not match parity manifest')
    if (!Array.isArray(parityEvidence.checks)) failures.push('parity automated evidence has no check results')
    else {
        const names = parityEvidence.checks.map(check => check?.name).filter(name => typeof name === 'string')
        if (new Set(names).size !== names.length) failures.push('parity automated evidence has duplicate check results')
        if (parityEvidence.checks.some(check => check?.passed !== true)) failures.push('parity automated evidence contains failed checks')
        if (JSON.stringify([...new Set(names)].sort()) !== JSON.stringify(manifestExpectedChecks)) failures.push('parity automated evidence results do not match parity manifest')
        for (const expectedCheck of manifestExpectedChecks) {
            if (!names.includes(expectedCheck)) failures.push(`parity automated evidence is missing check: ${expectedCheck}`)
        }
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
        if (expectedTarget && report.target !== expectedTarget) errors.push(`target must match ${expectedTarget}`)
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
    if (bundleAudit.schemaVersion !== 1) failures.push('bundle audit schema version is invalid')
    if (!Array.isArray(bundleAudit.files) || bundleAudit.files.length === 0) failures.push('bundle audit has no file manifest')
    if (typeof bundleAudit.artifactSha256 !== 'string' || !sha256Pattern.test(bundleAudit.artifactSha256)) failures.push('bundle audit artifactSha256 is invalid')
    if (!Array.isArray(bundleAudit.findings)) failures.push('bundle audit has no findings list')
    if (!Array.isArray(bundleAudit.missing)) failures.push('bundle audit has no missing list')
    for (const [index, file] of (bundleAudit.files || []).entries()) {
        if (typeof file?.path !== 'string' || file.path.length === 0) failures.push(`bundle audit file ${index} has no path`)
        if (!Number.isInteger(file?.size) || file.size < 0) failures.push(`bundle audit file ${index} has an invalid size`)
        if (typeof file?.sha256 !== 'string' || !sha256Pattern.test(file.sha256)) failures.push(`bundle audit file ${index} has an invalid hash`)
    }
    for (const name of ['sourceRevision', 'platform', 'arch', 'target']) {
        if (typeof bundleAudit[name] !== 'string' || bundleAudit[name].length === 0) failures.push(`bundle audit ${name} is missing`)
    }
    if (bundleAudit.passed !== true) {
        failures.push('bundle audit did not pass')
    }
    if (expectedRevision && bundleAudit.sourceRevision !== expectedRevision) failures.push(`bundle audit sourceRevision must match ${expectedRevision}`)
    if (expectedPlatform && bundleAudit.platform !== expectedPlatform) failures.push(`bundle audit platform must match ${expectedPlatform}`)
    if (expectedArch && bundleAudit.arch !== expectedArch) failures.push(`bundle audit arch must match ${expectedArch}`)
    if (expectedTarget && bundleAudit.target !== expectedTarget) failures.push(`bundle audit target must match ${expectedTarget}`)
}

const dependencyAudit = readJson(dependencyAuditPath, 'Tauri dependency audit')
if (dependencyAudit) {
    if (dependencyAudit.schemaVersion !== 1) failures.push('Tauri dependency audit schema version is invalid')
    if (dependencyAudit.policy !== 'tauri-release') failures.push('Tauri dependency audit policy must be tauri-release')
    if (!Array.isArray(dependencyAudit.manifests)) {
        failures.push('Tauri dependency audit has no manifest list')
    } else {
        for (const manifest of ['package.json', 'app/package.json']) {
            if (!dependencyAudit.manifests.includes(manifest)) {
                failures.push(`Tauri dependency audit is missing manifest: ${manifest}`)
            }
        }
    }
    if (!Array.isArray(dependencyAudit.findings)) failures.push('Tauri dependency audit has no findings list')
    if (dependencyAudit.passed !== true) failures.push('Tauri dependency audit did not pass')
}

const licenseReport = readJson(licenseReportPath, 'license report')
if (licenseReport) {
    if (licenseReport.schemaVersion !== 1) failures.push('license report schema version is invalid')
    if (typeof licenseReport.sourceRevision !== 'string' || licenseReport.sourceRevision.length === 0) failures.push('license report sourceRevision is missing')
    if (typeof licenseReport.license?.path !== 'string' || licenseReport.license.path.length === 0) failures.push('license report has no LICENSE entry')
    if (typeof licenseReport.license?.sha256 !== 'string' || !sha256Pattern.test(licenseReport.license.sha256)) failures.push('license report LICENSE hash is invalid')
    const notices = licenseReport.thirdPartyNotices
    if (typeof notices?.path !== 'string' || notices.path.length === 0) failures.push('license report has no third-party notices entry')
    if (!Number.isInteger(notices?.packageRows) || notices.packageRows < 1) failures.push('license report third-party packageRows is invalid')
    if (typeof notices?.sha256 !== 'string' || !sha256Pattern.test(notices.sha256)) failures.push('license report third-party notices hash is invalid')
    if (!Array.isArray(notices?.dependencies?.npm) || notices.dependencies.npm.length === 0) failures.push('license report has no npm dependencies')
    if (!Array.isArray(notices?.dependencies?.cargo) || notices.dependencies.cargo.length === 0) failures.push('license report has no cargo dependencies')
    for (const ecosystem of ['npm', 'cargo']) {
        for (const [index, dependency] of (notices?.dependencies?.[ecosystem] || []).entries()) {
            for (const field of ['name', 'version', 'license', 'manifest']) {
                if (typeof dependency?.[field] !== 'string' || dependency[field].length === 0) {
                    failures.push(`license report ${ecosystem} dependency ${index} has no ${field}`)
                }
            }
        }
    }
    if (licenseReport.passed !== true) failures.push('license report did not pass')
    if (expectedRevision && licenseReport.sourceRevision !== expectedRevision) {
        failures.push(`license report sourceRevision must match ${expectedRevision}`)
    }
}

const installerSmoke = readJson(installerSmokePath, 'installer smoke report')
if (installerSmoke) {
    if (installerSmoke.passed !== true) {
        failures.push('installer smoke did not pass')
    }
    if (installerSmoke.planOnly === true) {
        failures.push('installer smoke must execute install, launch, and uninstall operations')
    }
    if (expectedPlatform && installerSmoke.platform !== expectedPlatform) {
        failures.push(`installer smoke platform must match ${expectedPlatform}`)
    }
    const smokePlatform = expectedPlatform || installerSmoke.platform
    if (!['linux', 'macos', 'windows'].includes(smokePlatform)) {
        failures.push(`installer smoke platform is invalid: ${smokePlatform || '<missing>'}`)
    }
    const requiredActions = smokePlatform === 'macos'
        ? ['copy', 'launch', 'uninstall']
        : ['install', 'launch', 'uninstall']
    const actions = new Set(Array.isArray(installerSmoke.operations)
        ? installerSmoke.operations.map(operation => operation?.action).filter(action => typeof action === 'string')
        : [])
    for (const action of requiredActions) {
        if (!actions.has(action)) failures.push(`installer smoke is missing ${action} operation`)
    }
    const launchOperations = Array.isArray(installerSmoke.operations)
        ? installerSmoke.operations.filter(operation => operation?.action === 'launch')
        : []
    if (launchOperations.length > 0 && launchOperations.some(operation => operation.identity?.userDataPreserved !== true)) {
        failures.push('installer smoke launch did not verify user data preservation')
    }
}

if (bundleAudit) {
    const metadataPath = path.join(path.dirname(bundleAuditPath), 'tabby-rs-metadata.json')
    const metadata = readJson(metadataPath, 'release metadata')
    if (metadata) {
        if (!metadata.dependencyLocks || typeof metadata.dependencyLocks !== 'object') {
            failures.push('release metadata has no dependency lock hashes')
        } else {
            for (const lockFile of ['yarn.lock', 'src-tauri/Cargo.lock']) {
                const expectedHash = metadata.dependencyLocks[lockFile]
                if (!/^[0-9a-f]{64}$/i.test(expectedHash || '')) {
                    failures.push(`release metadata dependency lock hash is invalid: ${lockFile}`)
                    continue
                }
                const lockPath = path.join(root, lockFile)
                if (!fs.existsSync(lockPath)) {
                    failures.push(`release metadata dependency lock file is missing: ${lockFile}`)
                    continue
                }
                if (sha256(lockPath) !== expectedHash.toLowerCase()) {
                    failures.push(`release metadata dependency lock hash does not match checkout: ${lockFile}`)
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
    const target = validBenchmarkReports[0].target
    const binarySha256 = validBenchmarkReports[0].provenance?.binary?.sha256
    for (const report of validBenchmarkReports.slice(1)) {
        if (report.fixtureSha256 !== fixtureSha256) failures.push('benchmark reports must use one config fixture')
        if (report.artifactSha256 !== artifactSha256) failures.push('benchmark reports must use one bundle artifact')
        if (report.target !== target) failures.push('benchmark reports must use one matrix target')
        if (report.provenance?.binary?.sha256 !== binarySha256) failures.push('benchmark reports must use one benchmark binary')
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
