import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'

import { validateBenchmarkReport } from './benchmark/schema.mjs'

const execFileAsync = promisify(execFile)
const root = fileURLToPath(new URL('..', import.meta.url))
const fixture = path.join(root, 'scripts', 'fixtures', 'benchmark-process.mjs')
const runner = path.join(root, 'scripts', 'run-benchmarks.mjs')
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-benchmark-test-'))
const outputDir = path.join(work, 'benchmarks')
const bundleDir = path.join(work, 'bundle')
const frameReport = path.join(work, 'frames.json')
const configFixture = path.join(work, 'config.json')
const childPidFile = path.join(work, 'child-pids.txt')
const childCleanupFile = path.join(work, 'child-cleanup.txt')

function isProcessAlive (pid) {
    try {
        process.kill(pid, 0)
        return true
    } catch (error) {
        return error?.code === 'EPERM'
    }
}

fs.mkdirSync(bundleDir, { recursive: true })
fs.writeFileSync(path.join(bundleDir, 'app.bin'), 'benchmark bundle\n')
fs.writeFileSync(frameReport, JSON.stringify({
    method: 'requestAnimationFrame trace',
    samples: 2,
    p95FrameTimeMs: 16.7,
    droppedFrameCount: 0,
}))
fs.writeFileSync(configFixture, JSON.stringify({ plugins: [] }))

await execFileAsync(process.execPath, [runner,
    '--output-dir', outputDir,
    '--samples', '2',
    '--memory-wait-ms', '10',
    '--binary', process.execPath,
    '--binary-path', process.execPath,
    '--binary-args', JSON.stringify([fixture, '--ready']),
    '--output-command', process.execPath,
    '--output-args', JSON.stringify([fixture, '--output', '131072']),
    '--bundle', bundleDir,
    '--ui-frame-report', frameReport,
    '--config-fixture-path', configFixture,
    '--config-fixture', 'benchmark-test',
    '--platform', 'linux',
    '--arch', 'x86_64',
    '--target', 'fixture-target',
], { cwd: root })

const expected = {
    startup: 'cold-start-to-terminal-ready-ms',
    memory: 'idle-process-tree-rss-bytes',
    output: 'large-output',
    'bundle-size': 'bundle-size-bytes',
}
for (const [name, metric] of Object.entries(expected)) {
    const report = JSON.parse(fs.readFileSync(path.join(outputDir, `${name}.json`), 'utf8'))
    assert.deepEqual(validateBenchmarkReport(report, metric), [])
    assert.equal(report.samples, name === 'bundle-size' ? 1 : 2)
    assert.equal(report.commit.length, 40)
    assert.equal(report.configFixture, 'benchmark-test')
    assert.equal(report.platform, 'linux')
    assert.equal(report.arch, 'x86_64')
    assert.equal(report.target, 'fixture-target')
    assert.equal(report.provenance.binary.sha256.length, 64)
}

const outputReport = JSON.parse(fs.readFileSync(path.join(outputDir, 'output.json'), 'utf8'))
assert.equal(outputReport.bytes, 131072)
assert.equal(outputReport.uiFrameResponsiveness.droppedFrameCount, 0)
assert.equal(outputReport.uiFrameResponsiveness.traceSha256.length, 64)
assert.equal(outputReport.artifactSha256.length, 64)
assert.ok(validateBenchmarkReport(outputReport, expected.output, { requireLargeOutput: true })
    .some(error => error.startsWith('bytes must be at least ')))

const aggregateReport = JSON.parse(fs.readFileSync(path.join(outputDir, 'benchmark-report.json'), 'utf8'))
assert.equal(aggregateReport.host, 'tauri')
assert.equal(aggregateReport.commit, outputReport.commit)
assert.deepEqual(Object.keys(aggregateReport.reports).sort(), ['bundle-size', 'memory', 'output', 'startup'])

const tamperedReport = { ...outputReport, median: outputReport.median + 1 }
assert.ok(validateBenchmarkReport(tamperedReport, expected.output).includes('median does not match values'))
const wrongTargetReport = { ...outputReport, target: 'wrong-target' }
assert.ok(validateBenchmarkReport(wrongTargetReport, expected.output).includes('environment.toolchain must match target'))
const missingBinaryEvidence = {
    ...outputReport,
    provenance: { ...outputReport.provenance, binary: undefined },
}
assert.ok(validateBenchmarkReport(missingBinaryEvidence, expected.output).some(error => error.includes('provenance.binary must be an object')))

let exitedBeforeReady
try {
    await execFileAsync(process.execPath, [runner,
        '--output-dir', path.join(work, 'exited-before-ready'),
        '--samples', '1',
        '--ready-timeout-ms', '1000',
        '--binary', process.execPath,
        '--binary-path', process.execPath,
        '--binary-args', JSON.stringify([fixture, '--unknown']),
        '--bundle', bundleDir,
        '--config-fixture-path', configFixture,
        '--platform', 'linux',
        '--arch', 'x86_64',
        '--target', 'fixture-target',
    ], { cwd: root })
} catch (error) {
    exitedBeforeReady = error
}
assert.ok(exitedBeforeReady)
assert.match(exitedBeforeReady.stderr, /exited before writing ready marker \(exited with code 1; stderr:/)
assert.match(exitedBeforeReady.stderr, /unknown benchmark fixture mode: --unknown/)

let timedOutBeforeReady
try {
    await execFileAsync(process.execPath, [runner,
        '--output-dir', path.join(work, 'timed-out-before-ready'),
        '--samples', '1',
        '--ready-timeout-ms', '100',
        '--binary', process.execPath,
        '--binary-path', process.execPath,
        '--binary-args', JSON.stringify(['-e', 'setInterval(() => {}, 1000)']),
        '--bundle', bundleDir,
        '--config-fixture-path', configFixture,
        '--platform', 'linux',
        '--arch', 'x86_64',
        '--target', 'fixture-target',
    ], { cwd: root })
} catch (error) {
    timedOutBeforeReady = error
}
assert.ok(timedOutBeforeReady)
assert.match(timedOutBeforeReady.stderr, /did not write ready marker within 100ms \(still running\)/)

let processTreeCleanup
let processTreeChildPids
try {
    await execFileAsync(process.execPath, [runner,
        '--output-dir', path.join(work, 'process-tree-cleanup'),
        '--samples', '1',
        '--warmup-samples', '1',
        '--memory-wait-ms', '10',
        '--binary', process.execPath,
        '--binary-path', process.execPath,
        '--binary-args', JSON.stringify([fixture, '--ready-with-child', childPidFile, childCleanupFile]),
        '--output-command', process.execPath,
        '--output-args', JSON.stringify([fixture, '--output', '131072']),
        '--bundle', bundleDir,
        '--ui-frame-report', frameReport,
        '--config-fixture-path', configFixture,
        '--config-fixture', 'benchmark-process-tree',
        '--platform', 'linux',
        '--arch', 'x86_64',
        '--target', 'fixture-target',
    ], { cwd: root })
    processTreeChildPids = fs.readFileSync(childPidFile, 'utf8').trim().split(/\s+/).filter(Boolean).map(Number)
    if (process.platform !== 'win32') processTreeCleanup = fs.readFileSync(childCleanupFile, 'utf8')
} finally {
    if (fs.existsSync(childPidFile)) {
        for (const value of fs.readFileSync(childPidFile, 'utf8').trim().split(/\s+/).filter(Boolean)) {
            try { process.kill(Number(value), 'SIGKILL') } catch {}
        }
    }
}
assert.equal(processTreeChildPids?.length, 4, 'benchmark must record every descendant process')
if (process.platform === 'win32') {
    await new Promise(resolve => setTimeout(resolve, 100))
    assert.ok(processTreeChildPids.every(pid => !isProcessAlive(pid)), 'benchmark cleanup must terminate every Windows descendant process')
} else {
    assert.equal(processTreeCleanup?.trim().split('\n').length, 4, 'benchmark cleanup must terminate every descendant process')
}

console.log('Benchmark harness fixtures passed')
