#!/usr/bin/env node

import crypto from 'node:crypto'
import { execFileSync, spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { assertBenchmarkReport, BENCHMARK_METRICS, MIN_LARGE_OUTPUT_BYTES } from './benchmark/schema.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

function parseArguments (argv) {
    const options = {}
    for (let index = 0; index < argv.length; index++) {
        const argument = argv[index]
        if (!argument.startsWith('--')) throw new Error(`unexpected argument: ${argument}`)
        const name = argument.substring(2)
        const value = argv[++index]
        if (value === undefined || value.startsWith('--')) throw new Error(`--${name} requires a value`)
        options[name] = value
    }
    return options
}

function required (options, name) {
    if (!options[name]) throw new Error(`--${name} is required`)
    return options[name]
}

function jsonArgument (options, name, fallback = []) {
    if (!options[name]) return fallback
    const value = JSON.parse(options[name])
    if (!Array.isArray(value)) throw new Error(`--${name} must be a JSON array`)
    return value
}

function gitRevision () {
    return process.env.GITHUB_SHA || execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
}

function sha256 (value) {
    return crypto.createHash('sha256').update(value).digest('hex')
}

function summary (values) {
    const sorted = [...values].sort((left, right) => left - right)
    const percentile = fraction => sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * fraction) - 1))]
    return {
        values,
        median: percentile(0.5),
        p95: percentile(0.95),
    }
}

function environment () {
    return {
        os: `${os.platform()} ${os.release()}`,
        arch: process.arch,
        node: process.version,
        toolchain: process.env.TABBY_RS_TOOLCHAIN || 'unspecified',
    }
}

function normalizedPlatform (value) {
    return { win32: 'windows', darwin: 'macos', linux: 'linux' }[value] || value
}

function normalizedArch (value) {
    return { x64: 'x86_64', arm64: 'aarch64' }[value] || value
}

async function waitForFile (filePath, timeoutMs) {
    const deadline = Date.now() + timeoutMs
    while (Date.now() < deadline) {
        if (fs.existsSync(filePath)) return
        await new Promise(resolve => setTimeout(resolve, 10))
    }
    throw new Error(`benchmark process did not write ready marker within ${timeoutMs}ms: ${filePath}`)
}

async function terminate (child) {
    if (child.exitCode !== null || child.signalCode !== null) return
    child.kill()
    await new Promise(resolve => {
        const timer = setTimeout(() => {
            child.kill('SIGKILL')
            resolve()
        }, 2000)
        child.once('close', () => {
            clearTimeout(timer)
            resolve()
        })
    })
}

async function readySample (command, args, timeoutMs, configFixturePath) {
    const markerDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-ready-'))
    const marker = path.join(markerDirectory, 'ready.marker')
    const dataDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-benchmark-data-'))
    fs.copyFileSync(configFixturePath, path.join(dataDirectory, 'config.yaml'))
    const started = process.hrtime.bigint()
    const child = spawn(command, args, {
        cwd: root,
        env: {
            ...process.env,
            TABBY_RS_BENCHMARK_READY_FILE: marker,
            TABBY_RS_BENCHMARK_DATA_DIR: dataDirectory,
        },
        stdio: 'ignore',
    })
    try {
        await waitForFile(marker, timeoutMs)
        return Number(process.hrtime.bigint() - started) / 1e6
    } finally {
        await terminate(child)
        fs.rmSync(markerDirectory, { recursive: true, force: true })
        fs.rmSync(dataDirectory, { recursive: true, force: true })
    }
}

function processTreeRssBytes (pid) {
    if (process.platform === 'win32') {
        const script = `$root=${pid}; $ids=@($root); do { $new=@(Get-CimInstance Win32_Process | Where-Object { $ids -contains $_.ParentProcessId } | Select-Object -ExpandProperty ProcessId); $before=$ids.Count; $ids=@($ids+$new | Sort-Object -Unique) } while ($ids.Count -gt $before); $sum=0; foreach ($id in $ids) { try { $sum += (Get-Process -Id $id -ErrorAction Stop).WorkingSet64 } catch {} }; Write-Output $sum`
        return Number(execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { encoding: 'utf8' }).trim())
    }
    const rows = execFileSync('ps', ['-axo', 'pid=,ppid=,rss='], { encoding: 'utf8' })
        .split(/\r?\n/)
        .map(line => line.trim().split(/\s+/).map(Number))
        .filter(row => row.length === 3 && row.every(Number.isFinite))
    const children = new Map()
    for (const [childPid, parentPid, rssKb] of rows) {
        if (!children.has(parentPid)) children.set(parentPid, [])
        children.get(parentPid).push([childPid, rssKb])
    }
    const pending = [pid]
    const seen = new Set()
    let totalKb = 0
    while (pending.length > 0) {
        const current = pending.pop()
        if (seen.has(current)) continue
        seen.add(current)
        for (const [childPid, rssKb] of children.get(current) || []) {
            totalKb += rssKb
            pending.push(childPid)
        }
    }
    const own = rows.find(([rowPid]) => rowPid === pid)
    return (totalKb + (own?.[2] || 0)) * 1024
}

async function memorySample (command, args, waitMs, timeoutMs, configFixturePath) {
    const markerDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-memory-'))
    const marker = path.join(markerDirectory, 'ready.marker')
    const dataDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-benchmark-data-'))
    fs.copyFileSync(configFixturePath, path.join(dataDirectory, 'config.yaml'))
    const child = spawn(command, args, {
        cwd: root,
        env: {
            ...process.env,
            TABBY_RS_BENCHMARK_READY_FILE: marker,
            TABBY_RS_BENCHMARK_DATA_DIR: dataDirectory,
        },
        stdio: 'ignore',
    })
    try {
        await waitForFile(marker, timeoutMs)
        await new Promise(resolve => setTimeout(resolve, waitMs))
        return processTreeRssBytes(child.pid)
    } finally {
        await terminate(child)
        fs.rmSync(markerDirectory, { recursive: true, force: true })
        fs.rmSync(dataDirectory, { recursive: true, force: true })
    }
}

async function outputSample (command, args, maxOutputBytes) {
    const started = process.hrtime.bigint()
    const result = await new Promise((resolve, reject) => {
        const child = spawn(command, args, { cwd: root, env: process.env, stdio: ['ignore', 'pipe', 'pipe'] })
        const stdout = []
        const stderr = []
        let outputBytes = 0
        let exceededLimit = false
        child.stdout.on('data', chunk => {
            outputBytes += chunk.length
            if (outputBytes <= maxOutputBytes) stdout.push(chunk)
            else if (!exceededLimit) {
                exceededLimit = true
                child.kill('SIGKILL')
            }
        })
        child.stderr.on('data', chunk => stderr.push(chunk))
        child.once('error', reject)
        child.once('close', code => resolve({ code, stdout: Buffer.concat(stdout), stderr: Buffer.concat(stderr), exceededLimit }))
    })
    if (result.exceededLimit) throw new Error(`output command exceeded --max-output-bytes (${maxOutputBytes})`)
    if (result.code !== 0) throw new Error(`output command exited with ${result.code}: ${result.stderr.toString('utf8')}`)
    const durationSeconds = Number(process.hrtime.bigint() - started) / 1e9
    return {
        bytes: result.stdout.length,
        checksum: crypto.createHash('sha256').update(result.stdout).digest('hex'),
        throughput: result.stdout.length / Math.max(durationSeconds, Number.EPSILON),
    }
}

function walkFiles (directory, relative = '') {
    const files = []
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
        const relativePath = path.join(relative, entry.name)
        const absolutePath = path.join(directory, entry.name)
        if (entry.isDirectory()) files.push(...walkFiles(absolutePath, relativePath))
        else if (entry.isFile()) files.push({ absolutePath, relativePath: relativePath.split(path.sep).join('/') })
    }
    return files
}

function bundleStats (bundlePath) {
    const resolved = path.resolve(bundlePath)
    const files = fs.statSync(resolved).isDirectory()
        ? walkFiles(resolved)
        : [{ absolutePath: resolved, relativePath: path.basename(resolved) }]
    if (files.length === 0) throw new Error(`bundle contains no files: ${resolved}`)
    const manifest = files.map(file => ({
        path: file.relativePath,
        size: fs.statSync(file.absolutePath).size,
        sha256: sha256(fs.readFileSync(file.absolutePath)),
    })).sort((left, right) => left.path.localeCompare(right.path))
    const installedFootprintBytes = manifest.reduce((total, file) => total + file.size, 0)
    const largest = [...manifest].sort((left, right) => right.size - left.size)[0]
    const artifactSha256 = sha256(JSON.stringify(manifest))
    return { installedFootprintBytes, largest, artifactSha256 }
}

function baseReport (metric, options, samples, { fixtureSha256, artifactSha256, ...extra } = {}) {
    return {
        schemaVersion: 1,
        metric,
        host: 'tauri',
        platform: options.platform || normalizedPlatform(process.platform),
        arch: options.arch || normalizedArch(process.arch),
        commit: gitRevision(),
        configFixture: options['config-fixture'] || 'default-no-plugins',
        fixtureSha256,
        artifactSha256,
        samples: samples.length,
        warmupSamples: Number(options['warmup-samples'] || 0),
        measuredAt: new Date().toISOString(),
        environment: environment(),
        provenance: { runner: 'scripts/run-benchmarks.mjs', runnerVersion: '1' },
        ...summary(samples),
        ...extra,
    }
}

async function run (options) {
    const outputDir = path.resolve(options['output-dir'] || path.join(root, 'benchmarks'))
    const samples = Number(options.samples || 20)
    const warmups = Number(options['warmup-samples'] || 1)
    const readyTimeoutMs = Number(options['ready-timeout-ms'] || 30000)
    const maxOutputBytes = Number(options['max-output-bytes'] || 512 * 1024 * 1024)
    const minimumOutputBytes = Number(options['minimum-output-bytes'] || 1)
    if (!Number.isInteger(samples) || samples < 1) throw new Error('--samples must be a positive integer')
    if (!Number.isInteger(maxOutputBytes) || maxOutputBytes < 1) throw new Error('--max-output-bytes must be a positive integer')
    if (!Number.isInteger(minimumOutputBytes) || minimumOutputBytes < 1) throw new Error('--minimum-output-bytes must be a positive integer')
    if (minimumOutputBytes < MIN_LARGE_OUTPUT_BYTES && options['minimum-output-bytes']) {
        throw new Error(`--minimum-output-bytes must be at least ${MIN_LARGE_OUTPUT_BYTES} for release evidence`)
    }
    fs.mkdirSync(outputDir, { recursive: true })

    const binary = required(options, 'binary')
    const binaryArgs = jsonArgument(options, 'binary-args')
    const configFixturePath = path.resolve(required(options, 'config-fixture-path'))
    if (!fs.statSync(configFixturePath).isFile()) throw new Error(`config fixture is not a file: ${configFixturePath}`)
    const configFixture = options['config-fixture'] || path.basename(configFixturePath)
    const fixtureSha256 = sha256(fs.readFileSync(configFixturePath))
    const bundle = bundleStats(required(options, 'bundle'))
    const commonEvidence = { fixtureSha256, artifactSha256: bundle.artifactSha256 }
    for (let index = 0; index < warmups; index++) await readySample(binary, binaryArgs, readyTimeoutMs, configFixturePath)
    const startupValues = []
    for (let index = 0; index < samples; index++) startupValues.push(await readySample(binary, binaryArgs, readyTimeoutMs, configFixturePath))
    const startup = baseReport(BENCHMARK_METRICS.startup, { ...options, 'config-fixture': configFixture }, startupValues, {
        ...commonEvidence,
        unit: 'ms',
        readyTimeoutMs,
        provenance: { runner: 'scripts/run-benchmarks.mjs', runnerVersion: '1', readyMarker: true },
    })

    const waitMs = Number(options['memory-wait-ms'] || 30000)
    for (let index = 0; index < warmups; index++) await memorySample(binary, binaryArgs, waitMs, readyTimeoutMs, configFixturePath)
    const memoryValues = []
    for (let index = 0; index < samples; index++) memoryValues.push(await memorySample(binary, binaryArgs, waitMs, readyTimeoutMs, configFixturePath))
    const memory = baseReport(BENCHMARK_METRICS.memory, { ...options, 'config-fixture': configFixture }, memoryValues, {
        ...commonEvidence,
        unit: 'bytes',
        waitMs,
        provenance: { runner: 'scripts/run-benchmarks.mjs', runnerVersion: '1', readyMarker: true },
    })

    const outputCommand = required(options, 'output-command')
    const outputArgs = jsonArgument(options, 'output-args')
    for (let index = 0; index < warmups; index++) await outputSample(outputCommand, outputArgs, maxOutputBytes)
    const outputSamples = []
    for (let index = 0; index < samples; index++) outputSamples.push(await outputSample(outputCommand, outputArgs, maxOutputBytes))
    const frameReportPath = path.resolve(required(options, 'ui-frame-report'))
    const frameReportBytes = fs.readFileSync(frameReportPath)
    const outputFrameReport = {
        ...JSON.parse(frameReportBytes.toString('utf8')),
        traceSha256: sha256(frameReportBytes),
    }
    const output = baseReport(BENCHMARK_METRICS.output, { ...options, 'config-fixture': configFixture }, outputSamples.map(sample => sample.throughput), {
        ...commonEvidence,
        unit: 'bytesPerSecond',
        bytes: outputSamples[0].bytes,
        outputSha256: outputSamples[0].checksum,
        uiFrameResponsiveness: outputFrameReport,
        provenance: { runner: 'scripts/run-benchmarks.mjs', runnerVersion: '1', command: [outputCommand, ...outputArgs] },
    })
    if (output.bytes < minimumOutputBytes) {
        throw new Error(`output benchmark produced ${output.bytes} bytes, below the requested minimum of ${minimumOutputBytes}`)
    }
    if (outputSamples.some(sample => sample.bytes !== output.bytes || sample.checksum !== output.outputSha256)) {
        throw new Error('output benchmark samples did not produce identical bytes and checksum')
    }
    const finalBundle = bundleStats(required(options, 'bundle'))
    if (finalBundle.artifactSha256 !== bundle.artifactSha256) throw new Error('bundle changed during benchmark run')

    const bundleReport = baseReport(BENCHMARK_METRICS['bundle-size'], { ...options, 'config-fixture': configFixture }, [bundle.installedFootprintBytes], {
        ...commonEvidence,
        unit: 'bytes',
        artifactBytes: bundle.installedFootprintBytes,
        installedFootprintBytes: bundle.installedFootprintBytes,
        largestFileBytes: bundle.largest.size,
        largestFile: bundle.largest.path,
        artifactSha256: bundle.artifactSha256,
        provenance: { runner: 'scripts/run-benchmarks.mjs', runnerVersion: '1', bundlePath: path.resolve(options.bundle) },
    })

    const reports = { startup, memory, output, 'bundle-size': bundleReport }
    for (const [name, report] of Object.entries(reports)) {
        assertBenchmarkReport(report, BENCHMARK_METRICS[name])
        fs.writeFileSync(path.join(outputDir, `${name}.json`), `${JSON.stringify(report, null, 2)}\n`)
    }
    const aggregate = {
        schemaVersion: 1,
        host: 'tauri',
        commit: startup.commit,
        platform: startup.platform,
        arch: startup.arch,
        configFixture: startup.configFixture,
        fixtureSha256: startup.fixtureSha256,
        artifactSha256: startup.artifactSha256,
        measuredAt: new Date().toISOString(),
        reports,
    }
    fs.writeFileSync(path.join(outputDir, 'benchmark-report.json'), `${JSON.stringify(aggregate, null, 2)}\n`)
    console.log(`Benchmark reports written to ${outputDir}`)
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    run(parseArguments(process.argv.slice(2))).catch(error => {
        console.error(error instanceof Error ? error.stack || error.message : String(error))
        process.exitCode = 1
    })
}
