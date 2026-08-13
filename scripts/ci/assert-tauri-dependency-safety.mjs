#!/usr/bin/env node

import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')

const dependencySections = [
    'dependencies',
    'optionalDependencies',
    'peerDependencies',
    'devDependencies',
]

const forbiddenDependencies = [
    {
        id: 'electron-runtime-dependency',
        packages: new Set([
            '@electron/notarize',
            '@electron/rebuild',
            '@electron/remote',
            '@types/electron-debug',
            'electron',
            'electron-builder',
            'electron-config',
            'electron-debug',
            'electron-download',
            'electron-installer-snap',
            'electron-promise-ipc',
            'electron-updater',
            'tabby-electron',
        ]),
    },
    {
        id: 'node-native-dependency',
        packages: new Set([
            '@tabby-gang/windows-blurbehind',
            '@tabby-gang/windows-process-tree',
            'fontmanager-redux',
            'glasstron',
            'keytar',
            'macos-native-processlist',
            'native-process-working-directory',
            'node-pty',
            'serialport',
            'serialport-binding-webserialapi',
            'windows-native-registry',
        ]),
    },
]

function defaultManifestPaths () {
    return [
        path.join(root, 'package.json'),
        path.join(root, 'app/package.json'),
    ]
}

function isTauriReleaseExemption (relativePath, section, tauriRelease) {
    if (!tauriRelease) return null
    if (section === 'devDependencies' || section === 'peerDependencies') {
        return 'development-or-peer-dependency-does-not-ship'
    }
    if (relativePath === 'app/package.json' && (section === 'dependencies' || section === 'optionalDependencies')) {
        return 'legacy-electron-manifest-dependency-not-used-by-tauri-entry'
    }
    return null
}

function readManifest (manifestPath, { tauriRelease = false } = {}) {
    const absolutePath = path.resolve(manifestPath)
    let manifest
    try {
        manifest = JSON.parse(fs.readFileSync(absolutePath, 'utf8'))
    } catch (error) {
        return {
            path: path.relative(root, absolutePath).split(path.sep).join('/'),
            findings: [{
                rule: 'invalid-package-manifest',
                path: path.relative(root, absolutePath).split(path.sep).join('/'),
                message: error.message,
            }],
            excluded: [],
        }
    }

    const relativePath = path.relative(root, absolutePath).split(path.sep).join('/')
    if (!manifest || typeof manifest !== 'object' || Array.isArray(manifest)) {
        return {
            path: relativePath,
            findings: [{ rule: 'invalid-package-manifest', path: relativePath, message: 'manifest must be an object' }],
            excluded: [],
        }
    }

    const findings = []
    const excluded = []
    for (const section of dependencySections) {
        const dependencies = manifest[section]
        if (dependencies === undefined) continue
        if (!dependencies || typeof dependencies !== 'object' || Array.isArray(dependencies)) {
            findings.push({
                rule: 'invalid-dependency-section',
                path: relativePath,
                section,
                message: 'dependency section must be an object',
            })
            continue
        }
        for (const packageName of Object.keys(dependencies)) {
            const rule = forbiddenDependencies.find(candidate => candidate.packages.has(packageName))
            if (!rule) continue
            const finding = {
                rule: rule.id,
                path: relativePath,
                section,
                package: packageName,
                version: dependencies[packageName],
            }
            const exemption = isTauriReleaseExemption(relativePath, section, tauriRelease)
            if (exemption) {
                excluded.push({ ...finding, reason: exemption })
            } else {
                findings.push(finding)
            }
        }
    }

    return { path: relativePath, findings, excluded }
}

export function auditDependencyMetadata (manifestPaths = defaultManifestPaths(), { tauriRelease = false } = {}) {
    const manifests = manifestPaths.map(manifestPath => readManifest(manifestPath, { tauriRelease }))
    const findings = manifests.flatMap(manifest => manifest.findings)
    const excluded = manifests.flatMap(manifest => manifest.excluded)
    return {
        schemaVersion: 1,
        policy: tauriRelease ? 'tauri-release' : 'strict',
        manifests: manifests.map(manifest => manifest.path),
        findings,
        excluded,
        passed: findings.length === 0,
    }
}

function parseArguments (argv) {
    const args = [...argv]
    const outputIndex = args.indexOf('--output')
    const outputPath = outputIndex === -1 ? null : args[outputIndex + 1]
    assert.ok(!outputPath || outputPath.length > 0, '--output requires a path')
    return { outputPath, tauriRelease: args.includes('--tauri-release') }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    const { outputPath, tauriRelease } = parseArguments(process.argv.slice(2))
    const report = auditDependencyMetadata(undefined, { tauriRelease })
    if (outputPath) {
        fs.mkdirSync(path.dirname(path.resolve(outputPath)), { recursive: true })
        fs.writeFileSync(path.resolve(outputPath), `${JSON.stringify(report, null, 2)}\n`)
    }
    console.log(JSON.stringify(report, null, 2))
    if (!report.passed) process.exitCode = 1
}
