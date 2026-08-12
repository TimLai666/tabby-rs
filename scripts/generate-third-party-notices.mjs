import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

function packageJsonPaths (directory) {
    if (!fs.existsSync(directory)) {
        return []
    }
    const result = []
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
        if (entry.name.startsWith('.')) {
            continue
        }
        const entryPath = path.join(directory, entry.name)
        if (entry.isDirectory() && entry.name.startsWith('@')) {
            result.push(...packageJsonPaths(entryPath))
        } else if (entry.isDirectory()) {
            const packagePath = path.join(entryPath, 'package.json')
            if (fs.existsSync(packagePath)) {
                result.push(packagePath)
            }
        }
    }
    return result
}

function collectPackages () {
    const packages = new Map()
    for (const packagePath of packageJsonPaths(path.join(root, 'node_modules'))) {
        try {
            const packageJson = JSON.parse(fs.readFileSync(packagePath, 'utf8'))
            if (!packageJson.name || !packageJson.version) {
                continue
            }
            const key = `${packageJson.name}@${packageJson.version}`
            packages.set(key, {
                name: packageJson.name,
                version: packageJson.version,
                license: typeof packageJson.license === 'string' ? packageJson.license : 'SEE PACKAGE',
                path: path.relative(root, packagePath).split(path.sep).join('/'),
            })
        } catch {
            // A broken package manifest must not make the generated notice
            // contain guessed licensing information.
        }
    }
    return [...packages.values()].sort((left, right) => left.name.localeCompare(right.name) || left.version.localeCompare(right.version))
}

function outputPath (argv) {
    const index = argv.indexOf('--output')
    const value = index === -1 ? 'THIRD_PARTY_NOTICES.md' : argv[index + 1]
    assert.ok(value, '--output requires a path')
    return path.resolve(root, value)
}

const packages = collectPackages()
const lines = [
    '# Third-party notices',
    '',
    'This file is generated from the installed dependency manifests used for this build.',
    'Each package remains under its own license. The package manifest path is included so the exact license text can be audited from the locked build inputs.',
    '',
    '| Package | Version | License | Manifest |',
    '| --- | --- | --- | --- |',
    ...packages.map(packageInfo => `| ${packageInfo.name} | ${packageInfo.version} | ${packageInfo.license} | ${packageInfo.path} |`),
    '',
]
const destination = outputPath(process.argv.slice(2))
fs.mkdirSync(path.dirname(destination), { recursive: true })
fs.writeFileSync(destination, lines.join('\n'))
console.log(`Generated ${packages.length} third-party notices at ${path.relative(root, destination)}`)
