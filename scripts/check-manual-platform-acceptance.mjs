import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import yaml from 'js-yaml'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sha256Pattern = /^[0-9a-f]{64}$/i
const allowedStatuses = new Set(['passed', 'failed', 'not-run'])

function argument (args, name) {
    const index = args.indexOf(name)
    return index === -1 ? null : args[index + 1]
}

function isRelativeEvidencePath (value) {
    if (typeof value !== 'string' || value.length === 0 || path.isAbsolute(value)) return false
    return !value.split(/[\\/]+/).includes('..')
}

function isNonEmptyString (value) {
    return typeof value === 'string' && value.trim().length > 0
}

function evidenceFilePath (evidenceRoot, relativePath) {
    return path.resolve(evidenceRoot, relativePath)
}

function validateEvidenceFiles (files, label, evidenceRoot, failures) {
    for (const file of files || []) {
        const resolved = evidenceFilePath(evidenceRoot, file)
        if (!fs.existsSync(resolved) || !fs.statSync(resolved).isFile()) {
            failures.push(`${label} evidence file is missing: ${file}`)
        }
    }
}

function validateEnvironment (environment, failures) {
    if (!environment || typeof environment !== 'object') {
        failures.push('environment is missing')
        return
    }
    for (const field of ['os', 'webview', 'toolchain']) {
        if (!isNonEmptyString(environment[field])) failures.push(`environment ${field} is missing`)
    }
    if (!isNonEmptyString(environment.testedAt) || Number.isNaN(Date.parse(environment.testedAt))) {
        failures.push('environment testedAt must be an RFC 3339 timestamp')
    }
}

function validateChecks (checks, requiredChecks, evidenceRoot, failures) {
    if (!Array.isArray(checks)) {
        failures.push('checks must be an array')
        return
    }
    const ids = checks.map(check => check?.id).filter(isNonEmptyString)
    if (new Set(ids).size !== ids.length) failures.push('checks contain duplicate ids')
    const required = new Set(requiredChecks)
    for (const id of ids) {
        if (!required.has(id)) failures.push(`unknown manual platform check: ${id}`)
    }
    for (const id of requiredChecks) {
        const matches = checks.filter(check => check?.id === id)
        if (matches.length === 0) {
            failures.push(`missing manual platform check: ${id}`)
            continue
        }
        const check = matches[0]
        if (!allowedStatuses.has(check.status)) failures.push(`manual platform check ${id} has invalid status`)
        if (check.status !== 'passed') failures.push(`manual platform check ${id} is ${check.status || 'missing'}`)
        if (!Array.isArray(check.steps) || check.steps.length === 0 || check.steps.some(step => !isNonEmptyString(step))) {
            failures.push(`manual platform check ${id} has no observable steps`)
        }
        if (!Array.isArray(check.evidence) || check.evidence.length === 0 || check.evidence.some(file => !isRelativeEvidencePath(file))) {
            failures.push(`manual platform check ${id} has invalid evidence paths`)
        } else {
            validateEvidenceFiles(check.evidence, `manual platform check ${id}`, evidenceRoot, failures)
        }
    }
}

function validateArtifacts (artifacts, evidenceRoot, failures) {
    if (!Array.isArray(artifacts) || artifacts.length === 0) {
        failures.push('artifacts must be a non-empty array')
        return
    }
    for (const [index, artifact] of artifacts.entries()) {
        if (!isRelativeEvidencePath(artifact?.path)) failures.push(`artifact ${index} has an invalid relative path`)
        if (!sha256Pattern.test(artifact?.sha256 || '')) {
            failures.push(`artifact ${index} has an invalid SHA-256`)
            continue
        }
        if (!isRelativeEvidencePath(artifact?.path)) continue
        const artifactPath = evidenceFilePath(evidenceRoot, artifact.path)
        if (!fs.existsSync(artifactPath) || !fs.statSync(artifactPath).isFile()) {
            failures.push(`artifact ${index} file is missing: ${artifact.path}`)
        } else if (sha256Pattern.test(artifact.sha256) && sha256(artifactPath) !== artifact.sha256.toLowerCase()) {
            failures.push(`artifact ${index} SHA-256 does not match file: ${artifact.path}`)
        }
    }
}

function validateFeatureRecords (records, requiredFeatures, evidenceRoot, failures) {
    if (!Array.isArray(records)) {
        failures.push('features must be an array')
        return
    }
    const ids = records.map(feature => feature?.id).filter(isNonEmptyString)
    if (new Set(ids).size !== ids.length) failures.push('features contain duplicate ids')
    const required = new Set(requiredFeatures.map(feature => feature.id))
    for (const id of ids) {
        if (!required.has(id)) failures.push(`unknown manual feature: ${id}`)
    }
    for (const feature of requiredFeatures) {
        const matches = records.filter(record => record?.id === feature.id)
        if (matches.length === 0) {
            failures.push(`missing manual feature: ${feature.id}`)
            continue
        }
        const record = matches[0]
        if (record.status !== 'passed') failures.push(`manual feature ${feature.id} is ${record.status || 'missing'}`)
        if (!Array.isArray(record.steps) || record.steps.length === 0 || record.steps.some(step => !isNonEmptyString(step))) {
            failures.push(`manual feature ${feature.id} has no observable steps`)
        }
        if (!Array.isArray(record.evidence) || record.evidence.length === 0 || record.evidence.some(file => !isRelativeEvidencePath(file))) {
            failures.push(`manual feature ${feature.id} has invalid evidence paths`)
        } else {
            validateEvidenceFiles(record.evidence, `manual feature ${feature.id}`, evidenceRoot, failures)
        }
    }
}

function sha256 (filePath) {
    return crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex')
}

export function validateManualPlatformAcceptance (record, {
    platformEntry,
    featureEntries = [],
    expectedRevision = null,
    expectedArchitecture = null,
    expectedTarget = null,
    evidenceRoot = root,
} = {}) {
    const failures = []
    if (record?.schemaVersion !== 1) failures.push('schemaVersion must be 1')
    if (record?.kind !== 'tabby-rs-manual-platform-acceptance') failures.push('kind is invalid')
    if (!platformEntry || typeof platformEntry !== 'object') {
        failures.push('platform matrix entry is missing')
    } else {
        if (record.platform !== platformEntry.id) failures.push(`platform must be ${platformEntry.id}`)
        if (record.target !== platformEntry.target) failures.push(`target must be ${platformEntry.target}`)
        validateChecks(record.checks, platformEntry.requiredChecks || [], evidenceRoot, failures)
    }
    validateFeatureRecords(record?.features, featureEntries, evidenceRoot, failures)
    if (!isNonEmptyString(record?.sourceRevision)) failures.push('sourceRevision is missing')
    if (expectedRevision && record.sourceRevision !== expectedRevision) failures.push(`sourceRevision must match ${expectedRevision}`)
    if (!isNonEmptyString(record?.architecture)) failures.push('architecture is missing')
    if (expectedArchitecture && record.architecture !== expectedArchitecture) failures.push(`architecture must match ${expectedArchitecture}`)
    if (!isNonEmptyString(record?.target)) failures.push('target is missing')
    if (expectedTarget && record.target !== expectedTarget) failures.push(`target must match ${expectedTarget}`)
    validateEnvironment(record?.environment, failures)
    validateArtifacts(record?.artifacts, evidenceRoot, failures)
    return { passed: failures.length === 0, failures }
}

function readJson (filePath) {
    try {
        return JSON.parse(fs.readFileSync(filePath, 'utf8'))
    } catch (error) {
        return { __error: error.message }
    }
}

async function main () {
    const args = process.argv.slice(2)
    const recordPath = argument(args, '--record')
    const featuresPath = argument(args, '--platform-matrix') || path.join(root, 'parity/platform-matrix.yaml')
    if (!recordPath) {
        console.error('Usage: node scripts/check-manual-platform-acceptance.mjs --record <path> [--source-revision <sha>]')
        process.exitCode = 2
        return
    }
    let matrix
    let features
    try {
        matrix = yaml.load(fs.readFileSync(path.resolve(featuresPath), 'utf8'))
        features = yaml.load(fs.readFileSync(path.join(root, 'parity/features.yaml'), 'utf8'))
    } catch (error) {
        console.error(`Unable to read platform matrix: ${error.message}`)
        process.exitCode = 2
        return
    }
    const resolvedRecordPath = path.resolve(recordPath)
    const record = readJson(resolvedRecordPath)
    if (record.__error) {
        console.error(`Unable to read manual acceptance record: ${record.__error}`)
        process.exitCode = 2
        return
    }
    const platformEntry = (matrix?.platforms || []).find(entry => entry.id === record.platform)
    const platformFamily = platformEntry?.id?.split('-')[0]
    const result = validateManualPlatformAcceptance(record, {
        platformEntry,
        featureEntries: (features?.features || []).filter(feature =>
            platformFamily && (feature.platforms || []).includes(platformFamily) && (feature.tests?.manual || []).length > 0),
        expectedRevision: argument(args, '--source-revision'),
        expectedArchitecture: argument(args, '--architecture'),
        expectedTarget: argument(args, '--target'),
        evidenceRoot: path.resolve(argument(args, '--evidence-root') || path.dirname(path.dirname(resolvedRecordPath))),
    })
    console.log(JSON.stringify(result, null, 2))
    if (!result.passed) process.exitCode = 1
}

if (fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) await main()
