const HASH_PATTERN = /^[0-9a-f]{64}$/i
const COMMIT_PATTERN = /^[0-9a-f]{40,64}$/i

export const BENCHMARK_METRICS = {
    startup: 'cold-start-to-terminal-ready-ms',
    memory: 'idle-process-tree-rss-bytes',
    output: 'large-output',
    'bundle-size': 'bundle-size-bytes',
}

export const MIN_LARGE_OUTPUT_BYTES = 100 * 1024 * 1024

function isRecord (value) {
    return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function requireString (errors, object, name) {
    if (typeof object?.[name] !== 'string' || object[name].length === 0) {
        errors.push(`${name} must be a non-empty string`)
    }
}

function requireFiniteNumber (errors, object, name, { integer = false, positive = false } = {}) {
    const value = object?.[name]
    if (typeof value !== 'number' || !Number.isFinite(value) || (integer && !Number.isInteger(value)) || (positive && value <= 0)) {
        errors.push(`${name} must be a finite${integer ? ' integer' : ''}${positive ? ' positive' : ''} number`)
    }
}

function validateSummary (errors, report) {
    if (!Array.isArray(report.values) || report.values.length !== report.samples) {
        errors.push('values length must equal samples')
    } else {
        report.values.forEach((value, index) => {
            if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) {
                errors.push(`values[${index}] must be a finite non-negative number`)
            }
        })
    }
    requireFiniteNumber(errors, report, 'median')
    requireFiniteNumber(errors, report, 'p95')
    if (typeof report.median === 'number' && typeof report.p95 === 'number' && report.p95 < report.median) {
        errors.push('p95 must be greater than or equal to median')
    }
    if (
        Array.isArray(report.values)
        && report.values.length === report.samples
        && report.values.every(value => typeof value === 'number' && Number.isFinite(value) && value >= 0)
        && typeof report.median === 'number'
        && typeof report.p95 === 'number'
    ) {
        const sorted = [...report.values].sort((left, right) => left - right)
        const percentile = fraction => sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * fraction) - 1))]
        if (report.median !== percentile(0.5)) errors.push('median does not match values')
        if (report.p95 !== percentile(0.95)) errors.push('p95 does not match values')
    }
}

export function validateBenchmarkReport (report, expectedMetric, { requireLargeOutput = false } = {}) {
    const errors = []
    if (!isRecord(report)) {
        return ['report must be an object']
    }
    if (report.schemaVersion !== 1) errors.push('schemaVersion must be 1')
    if (report.metric !== expectedMetric) errors.push(`metric must be ${expectedMetric}`)
    if (report.host !== 'tauri') errors.push('host must be tauri')
    for (const name of ['platform', 'arch', 'target', 'unit', 'commit', 'configFixture', 'measuredAt', 'fixtureSha256', 'artifactSha256']) {
        requireString(errors, report, name)
    }
    if (typeof report.commit === 'string' && !COMMIT_PATTERN.test(report.commit)) errors.push('commit must be a full git SHA')
    if (typeof report.fixtureSha256 === 'string' && !HASH_PATTERN.test(report.fixtureSha256)) errors.push('fixtureSha256 must be a SHA-256 digest')
    if (typeof report.artifactSha256 === 'string' && !HASH_PATTERN.test(report.artifactSha256)) errors.push('artifactSha256 must be a SHA-256 digest')
    requireFiniteNumber(errors, report, 'samples', { integer: true, positive: true })
    requireFiniteNumber(errors, report, 'warmupSamples', { integer: true })
    if (typeof report.measuredAt === 'string' && Number.isNaN(Date.parse(report.measuredAt))) errors.push('measuredAt must be an ISO timestamp')
    if (!isRecord(report.environment)) errors.push('environment must be an object')
    for (const name of ['os', 'arch', 'node', 'toolchain']) requireString(errors, report.environment, name)
    if (!isRecord(report.provenance)) errors.push('provenance must be an object')
    requireString(errors, report.provenance, 'runner')
    if (!isRecord(report.provenance?.binary)) errors.push('provenance.binary must be an object')
    requireString(errors, report.provenance?.binary, 'path')
    if (typeof report.provenance?.binary?.sha256 !== 'string' || !HASH_PATTERN.test(report.provenance.binary.sha256)) {
        errors.push('provenance.binary.sha256 must be a SHA-256 digest')
    }
    if (typeof report.samples === 'number' && Number.isInteger(report.samples) && report.samples > 0) validateSummary(errors, report)

    switch (expectedMetric) {
        case BENCHMARK_METRICS.startup:
            if (report.unit !== 'ms') errors.push('startup unit must be ms')
            if (report.provenance?.readyMarker !== true) errors.push('startup must prove a ready marker')
            requireFiniteNumber(errors, report, 'readyTimeoutMs', { positive: true })
            break
        case BENCHMARK_METRICS.memory:
            if (report.unit !== 'bytes') errors.push('memory unit must be bytes')
            if (report.provenance?.readyMarker !== true) errors.push('memory must prove a ready marker')
            requireFiniteNumber(errors, report, 'waitMs', { positive: true })
            break
        case BENCHMARK_METRICS.output:
            if (report.unit !== 'bytesPerSecond') errors.push('output unit must be bytesPerSecond')
            requireFiniteNumber(errors, report, 'bytes', { integer: true, positive: true })
            if (requireLargeOutput && typeof report.bytes === 'number' && report.bytes < MIN_LARGE_OUTPUT_BYTES) {
                errors.push(`bytes must be at least ${MIN_LARGE_OUTPUT_BYTES} for a release gate`)
            }
            if (typeof report.outputSha256 !== 'string' || !HASH_PATTERN.test(report.outputSha256)) errors.push('outputSha256 must be a SHA-256 digest')
            if (!isRecord(report.uiFrameResponsiveness)) errors.push('uiFrameResponsiveness must be an object')
            requireString(errors, report.uiFrameResponsiveness, 'method')
            if (typeof report.uiFrameResponsiveness.traceSha256 !== 'string' || !HASH_PATTERN.test(report.uiFrameResponsiveness.traceSha256)) errors.push('uiFrameResponsiveness.traceSha256 must be a SHA-256 digest')
            requireFiniteNumber(errors, report.uiFrameResponsiveness, 'samples', { integer: true, positive: true })
            requireFiniteNumber(errors, report.uiFrameResponsiveness, 'p95FrameTimeMs')
            requireFiniteNumber(errors, report.uiFrameResponsiveness, 'droppedFrameCount', { integer: true })
            break
        case BENCHMARK_METRICS['bundle-size']:
            if (report.unit !== 'bytes') errors.push('bundle-size unit must be bytes')
            for (const name of ['artifactBytes', 'installedFootprintBytes', 'largestFileBytes']) {
                requireFiniteNumber(errors, report, name, { integer: true, positive: true })
            }
            requireString(errors, report, 'largestFile')
            if (typeof report.artifactSha256 !== 'string' || !HASH_PATTERN.test(report.artifactSha256)) errors.push('artifactSha256 must be a SHA-256 digest')
            break
        default:
            errors.push(`unsupported benchmark metric: ${expectedMetric}`)
    }
    return errors
}

export function assertBenchmarkReport (report, expectedMetric) {
    const errors = validateBenchmarkReport(report, expectedMetric)
    if (errors.length > 0) {
        throw new Error(`${expectedMetric} benchmark is invalid:\n- ${errors.join('\n- ')}`)
    }
}
