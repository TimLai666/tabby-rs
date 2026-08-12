import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'

import { validateBenchmarkReport } from './benchmark/schema.mjs'

const execFileAsync = promisify(execFile)
const root = path.resolve(new URL('..', import.meta.url).pathname)
const fixture = path.join(root, 'scripts', 'fixtures', 'benchmark-process.mjs')
const runner = path.join(root, 'scripts', 'run-benchmarks.mjs')
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-benchmark-test-'))
const outputDir = path.join(work, 'benchmarks')
const bundleDir = path.join(work, 'bundle')
const frameReport = path.join(work, 'frames.json')
const configFixture = path.join(work, 'config.json')

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
    '--binary-args', JSON.stringify([fixture, '--ready']),
    '--output-command', process.execPath,
    '--output-args', JSON.stringify([fixture, '--output', '131072']),
    '--bundle', bundleDir,
    '--ui-frame-report', frameReport,
    '--config-fixture-path', configFixture,
    '--config-fixture', 'benchmark-test',
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
}

const outputReport = JSON.parse(fs.readFileSync(path.join(outputDir, 'output.json'), 'utf8'))
assert.equal(outputReport.bytes, 131072)
assert.equal(outputReport.uiFrameResponsiveness.droppedFrameCount, 0)
assert.equal(outputReport.uiFrameResponsiveness.traceSha256.length, 64)
assert.equal(outputReport.artifactSha256.length, 64)

console.log('Benchmark harness fixtures passed')
