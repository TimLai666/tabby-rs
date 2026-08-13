#!/usr/bin/env node

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')

const trackedProductionPath = file => (
    file.startsWith('app/src/')
    || file.startsWith('app/lib/')
    || file.startsWith('src-tauri/src/')
    || file.startsWith('tabby-') && file.includes('/src/')
    || file.startsWith('web/')
    || file.startsWith('.github/workflows/')
    || /^(?:package\.json|yarn\.lock|app\/package\.json|app\/yarn\.lock)$/.test(file)
)

const forbiddenPatterns = [
    { id: 'sentry-sdk-or-endpoint', pattern: /@sentry[\\/]|sentry\.io|SENTRY_(?:DSN|AUTH_TOKEN|ORG|PROJECT)/i },
    { id: 'mixpanel-sdk-or-endpoint', pattern: /mixpanel(?:-browser)?(?:[\\/]|")|mixpanel\.com|MIXPANEL_[A-Z0-9_]+/i },
]

function trackedProductionFiles () {
    return execFileSync('git', ['ls-files', '-z'], { cwd: root })
        .toString('utf8')
        .split('\0')
        .filter(file => file && trackedProductionPath(file))
}

export function findTelemetryViolations (text, file) {
    return forbiddenPatterns
        .filter(rule => rule.pattern.test(text))
        .map(rule => ({ rule: rule.id, path: file }))
}

export function scanProductionFiles (files = trackedProductionFiles()) {
    const findings = []
    for (const relativePath of files) {
        const absolutePath = path.join(root, relativePath)
        const bytes = fs.readFileSync(absolutePath)
        if (bytes.includes(0)) continue
        findings.push(...findTelemetryViolations(bytes.toString('utf8'), relativePath))
    }
    return findings
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    const findings = scanProductionFiles()
    if (findings.length) {
        console.error('Built-in telemetry references found in production files:')
        for (const finding of findings) console.error(`- ${finding.rule}: ${finding.path}`)
        process.exitCode = 1
    } else {
        console.log('Production source contains no built-in Sentry or Mixpanel telemetry references.')
    }
}
