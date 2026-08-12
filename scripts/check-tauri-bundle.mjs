import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const defaultBundlePath = path.join(root, 'app/dist-tauri')

const forbiddenFileRules = [
    { id: 'electron-runtime', pattern: /(^|[/\\])electron(?:\.asar|\.exe)?$|electron-helper/i },
    { id: 'node-runtime', pattern: /(^|[/\\])node(?:\.exe)?$|node-runtime/i },
    { id: 'node-native-addon', pattern: /(?:node-pty|serialport|keytar)[^/\\]*\.node$/i },
]

const forbiddenContentRules = [
    {
        id: 'electron-runtime-import',
        pattern: /(?:\b(?:from|require|import)\s*(?:\(\s*)?["'](?:electron|@electron\/remote)(?:[\\/'"]|$)|\belectron-updater\b)/i,
    },
    { id: 'electron-runtime-binary', binaryOnly: true, pattern: /electron(?:\.asar|\.exe| helper)/i },
    { id: 'node-runtime-binary', binaryOnly: true, pattern: /node(?:\.exe|-runtime)/i },
    { id: 'sentry-sdk-or-endpoint', pattern: /(?:@sentry\/|sentry\.io|SENTRY_DSN)/i },
    { id: 'mixpanel-sdk-or-endpoint', pattern: /(?:mixpanel(?:-browser)?|mixpanel\.com)/i },
]

const requiredFiles = [
    { id: 'license', pattern: /^LICENSE(?:\.txt|\.md)?$/i },
    { id: 'third-party-notices', pattern: /^THIRD_PARTY_NOTICES(?:\.txt|\.md)?$/i },
    { id: 'metadata', pattern: /^tabby-rs-metadata\.json$/i },
    { id: 'icon', pattern: /^tabby-rs-icon\.(?:png|ico|icns|svg)$/i },
    { id: 'updater-public-key', pattern: /^updater-public-key\.txt$/i },
    { id: 'update-manifest', pattern: /^update-manifest\.json$/i },
    { id: 'updater-signature', pattern: /\.sig$/i },
]

function walkFiles (directory, relative = '') {
    const entries = fs.readdirSync(directory, { withFileTypes: true })
    const files = []
    for (const entry of entries) {
        const relativePath = path.join(relative, entry.name)
        const absolutePath = path.join(directory, entry.name)
        if (entry.isDirectory()) {
            files.push(...walkFiles(absolutePath, relativePath))
        } else if (entry.isFile()) {
            files.push({ absolutePath, relativePath: relativePath.split(path.sep).join('/') })
        } else {
            files.push({ absolutePath, relativePath: relativePath.split(path.sep).join('/'), special: true })
        }
    }
    return files
}

function sha256 (filePath) {
    return crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex')
}

function readAuditContent (filePath, size) {
    if (size > 16 * 1024 * 1024) {
        return null
    }
    const bytes = fs.readFileSync(filePath)
    if (bytes.includes(0)) {
        const strings = []
        let current = []
        for (const byte of bytes) {
            if (byte >= 0x20 && byte <= 0x7e) {
                current.push(byte)
            } else {
                if (current.length >= 4) strings.push(Buffer.from(current).toString('ascii'))
                current = []
            }
        }
        if (current.length >= 4) strings.push(Buffer.from(current).toString('ascii'))
        return { text: strings.join('\n'), binary: true }
    }
    return { text: bytes.toString('utf8'), binary: false }
}

export function auditBundle (bundlePath, { release = false } = {}) {
    const resolvedPath = path.resolve(bundlePath)
    assert.ok(fs.existsSync(resolvedPath), `Tauri bundle does not exist: ${resolvedPath}`)
    assert.ok(fs.statSync(resolvedPath).isDirectory(), `Tauri bundle is not a directory: ${resolvedPath}`)

    const files = walkFiles(resolvedPath)
    const findings = []
    const manifest = []
    for (const file of files) {
        if (file.special) {
            findings.push({ rule: 'special-file', path: file.relativePath })
            continue
        }
        const size = fs.statSync(file.absolutePath).size
        manifest.push({ path: file.relativePath, size, sha256: sha256(file.absolutePath) })
        for (const rule of forbiddenFileRules) {
            if (rule.pattern.test(file.relativePath)) {
                findings.push({ rule: rule.id, path: file.relativePath })
            }
        }
        const content = readAuditContent(file.absolutePath, size)
        if (content !== null) {
            for (const rule of forbiddenContentRules) {
                if (rule.binaryOnly && !content.binary) continue
                if (rule.pattern.test(content.text)) {
                    findings.push({ rule: rule.id, path: file.relativePath })
                }
            }
        }
    }

    const missing = []
    if (release) {
        for (const rule of requiredFiles) {
            if (!manifest.some(file => file.path.indexOf('/') === -1 && rule.pattern.test(file.path))) {
                missing.push(rule.id)
            }
        }
    }

    const sortedManifest = manifest.sort((left, right) => left.path.localeCompare(right.path))
    const metadataFile = sortedManifest.find(file => path.basename(file.path).toLowerCase() === 'tabby-rs-metadata.json')
    let metadata = null
    if (metadataFile) {
        try {
            metadata = JSON.parse(fs.readFileSync(path.join(resolvedPath, metadataFile.path), 'utf8'))
        } catch (error) {
            findings.push({ rule: 'invalid-release-metadata', path: metadataFile.path, message: error.message })
        }
    }
    return {
        schemaVersion: 1,
        bundlePath: resolvedPath,
        release,
        files: sortedManifest,
        artifactSha256: crypto.createHash('sha256').update(JSON.stringify(sortedManifest)).digest('hex'),
        sourceRevision: metadata?.revision || null,
        target: metadata?.target || null,
        platform: metadata?.platform || null,
        arch: metadata?.arch || null,
        dependencyLocks: metadata?.dependencyLocks || null,
        findings,
        missing,
        passed: findings.length === 0 && missing.length === 0,
    }
}

function parseArguments (argv) {
    const args = [...argv]
    const bundlePath = args.shift() || defaultBundlePath
    const release = args.includes('--release')
    const outputIndex = args.indexOf('--output')
    const outputPath = outputIndex === -1 ? null : args[outputIndex + 1]
    assert.ok(!outputPath || outputPath.length > 0, '--output requires a path')
    return { bundlePath, release, outputPath }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    const { bundlePath, release, outputPath } = parseArguments(process.argv.slice(2))
    const report = auditBundle(bundlePath, { release })
    if (outputPath) {
        fs.writeFileSync(path.resolve(outputPath), `${JSON.stringify(report, null, 2)}\n`)
    }
    console.log(JSON.stringify(report, null, 2))
    if (!report.passed) {
        process.exitCode = 1
    }
}
