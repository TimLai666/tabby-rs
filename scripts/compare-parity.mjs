import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import yaml from 'js-yaml'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const defaultFeaturesPath = path.join(root, 'parity/features.yaml')
const defaultPlatformsPath = path.join(root, 'parity/platform-matrix.yaml')

function readYaml (filePath) {
    try {
        return yaml.load(fs.readFileSync(filePath, 'utf8'))
    } catch (error) {
        return { __error: `${filePath}: ${error.message}` }
    }
}

function duplicateValues (items, key) {
    const seen = new Set()
    const duplicates = new Set()
    for (const item of items) {
        const value = item?.[key]
        if (!value) continue
        if (seen.has(value)) duplicates.add(value)
        seen.add(value)
    }
    return [...duplicates]
}

export function compareParity ({ featuresPath = defaultFeaturesPath, platformsPath = defaultPlatformsPath } = {}) {
    const featuresDocument = readYaml(featuresPath)
    const platformsDocument = readYaml(platformsPath)
    const failures = []
    const features = Array.isArray(featuresDocument?.features) ? featuresDocument.features : []
    const platforms = Array.isArray(platformsDocument?.platforms) ? platformsDocument.platforms : []

    if (featuresDocument?.__error) failures.push(`invalid features manifest: ${featuresDocument.__error}`)
    if (platformsDocument?.__error) failures.push(`invalid platform manifest: ${platformsDocument.__error}`)
    if (!features.length) failures.push('features manifest has no features')
    if (!platforms.length) failures.push('platform manifest has no platforms')
    if (!featuresDocument?.baseline?.repository) failures.push('features manifest has no baseline repository')
    if (!featuresDocument?.baseline?.commit) failures.push('features manifest has no baseline commit')
    if (!featuresDocument?.baseline?.version) failures.push('features manifest has no baseline version')

    for (const id of duplicateValues(features, 'id')) failures.push(`duplicate feature id: ${id}`)
    for (const id of duplicateValues(platforms, 'id')) failures.push(`duplicate platform id: ${id}`)
    for (const target of duplicateValues(platforms, 'target')) failures.push(`duplicate platform target: ${target}`)

    const acceptedStatuses = new Set(['pending', 'failed', 'passed', 'accepted-difference'])
    for (const feature of features) {
        if (!feature?.id) failures.push('feature is missing id')
        if (!feature?.title) failures.push(`feature ${feature?.id || '<unnamed>'} is missing title`)
        if (!acceptedStatuses.has(feature?.status)) {
            failures.push(`feature ${feature?.id || '<unnamed>'} has invalid status: ${feature?.status || '<missing>'}`)
        }
        if (feature?.status === 'pending' || feature?.status === 'failed') {
            failures.push(`feature ${feature.id} is ${feature.status}`)
        }
        if ((feature?.status === 'passed' || feature?.status === 'accepted-difference') && !feature.evidence?.length) {
            failures.push(`feature ${feature.id} has no evidence for ${feature.status}`)
        }
        if (feature?.status === 'accepted-difference' && !feature.reason) {
            failures.push(`feature ${feature.id} accepted-difference has no reason`)
        }
    }
    for (const platform of platforms) {
        if (!platform?.id) failures.push('platform is missing id')
        if (!platform?.runner) failures.push(`platform ${platform?.id || '<unnamed>'} is missing runner`)
        if (!platform?.target) failures.push(`platform ${platform?.id || '<unnamed>'} is missing target`)
        if (!acceptedStatuses.has(platform?.status)) {
            failures.push(`platform ${platform?.id || '<unnamed>'} has invalid status: ${platform?.status || '<missing>'}`)
        }
        if (platform?.status === 'pending' || platform?.status === 'failed') {
            failures.push(`platform ${platform.id} is ${platform.status}`)
        }
        if (platform?.status === 'passed' && !platform.evidence?.length) {
            failures.push(`platform ${platform.id} has no evidence for passed`)
        }
    }

    const statusCounts = values => values.reduce((counts, item) => {
        const status = item?.status || 'missing'
        counts[status] = (counts[status] || 0) + 1
        return counts
    }, {})
    return {
        schemaVersion: 1,
        baseline: featuresDocument?.baseline || null,
        features: {
            total: features.length,
            statuses: statusCounts(features),
            pending: features.filter(feature => feature.status === 'pending').map(feature => feature.id),
        },
        platforms: {
            total: platforms.length,
            statuses: statusCounts(platforms),
            pending: platforms.filter(platform => platform.status === 'pending').map(platform => platform.id),
        },
        failures,
        passed: failures.length === 0,
    }
}

function argument (args, name) {
    const index = args.indexOf(name)
    return index === -1 ? null : args[index + 1]
}

function escapeHtml (value) {
    return String(value)
        .replaceAll('&', '&amp;')
        .replaceAll('<', '&lt;')
        .replaceAll('>', '&gt;')
        .replaceAll('"', '&quot;')
        .replaceAll("'", '&#39;')
}

export function renderParityHtml (report) {
    const status = report.passed ? 'passed' : 'failed'
    const reportJson = escapeHtml(JSON.stringify(report, null, 2))
    const failures = report.failures.length
        ? `<ul>${report.failures.map(failure => `<li>${escapeHtml(failure)}</li>`).join('')}</ul>`
        : '<p>No parity failures.</p>'
    return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Tabby RS parity report</title>
  <style>
    :root { color-scheme: light dark; font-family: system-ui, sans-serif; }
    body { max-width: 72rem; margin: 2rem auto; padding: 0 1rem; }
    .status { border: 1px solid currentColor; border-radius: .4rem; display: inline-block; padding: .35rem .6rem; }
    .passed { color: #087f23; }
    .failed { color: #b42318; }
    pre { overflow: auto; padding: 1rem; border-radius: .4rem; background: color-mix(in srgb, currentColor 10%, transparent); }
  </style>
</head>
<body>
  <h1>Tabby RS parity report</h1>
  <p class="status ${status}">${status.toUpperCase()}</p>
  <p>Baseline: <code>${escapeHtml(report.baseline?.repository || 'unknown')}</code> at <code>${escapeHtml(report.baseline?.commit || 'unknown')}</code> (${escapeHtml(report.baseline?.version || 'unknown')})</p>
  <h2>Failures</h2>
  ${failures}
  <h2>Machine-readable report</h2>
  <pre>${reportJson}</pre>
</body>
</html>
`
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    const args = process.argv.slice(2)
    const reportOnly = args.includes('--report-only')
    const report = compareParity({
        featuresPath: path.resolve(argument(args, '--features') || defaultFeaturesPath),
        platformsPath: path.resolve(argument(args, '--platforms') || defaultPlatformsPath),
    })
    const outputPath = argument(args, '--output')
    if (outputPath) fs.writeFileSync(path.resolve(outputPath), `${JSON.stringify(report, null, 2)}\n`)
    const htmlOutputPath = argument(args, '--html-output')
    if (htmlOutputPath) fs.writeFileSync(path.resolve(htmlOutputPath), renderParityHtml(report))
    console.log(JSON.stringify(report, null, 2))
    if (!report.passed && !reportOnly) process.exitCode = 1
}
