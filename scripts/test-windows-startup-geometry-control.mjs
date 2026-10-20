import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'

if (process.argv.includes('--help')) {
    console.log('Usage: node scripts/test-windows-startup-geometry-control.mjs\nWindows-only real Wry negative control. Runs the uncorrected bundled window geometry, requires its specific outer-size failures, and rejects unrelated failures. Other platforms skip.')
    process.exit(0)
}
assert.equal(process.argv.length, 2, 'no arguments supported; use --help')
if (process.platform !== 'win32') {
    console.log('Windows startup geometry negative control skipped on this platform')
    process.exit(0)
}
const result = spawnSync('cargo', ['run', '--locked', '--manifest-path', 'src-tauri/Cargo.toml', '--example', 'window_size_native', '--', '--stock-geometry-control'], { encoding: 'utf8', timeout: 240_000, maxBuffer: 8 * 1024 * 1024 })
const output = `${result.stdout ?? ''}\n${result.stderr ?? ''}`
process.stdout.write(output)
assert.ifError(result.error)
assert.equal(result.signal, null)
assert.equal(result.status, 1, 'stock geometry must fail original outer-size assertions')
const completed = output.match(/window_size_native completed: checks=(\d+) failures=(\d+)/)
assert.ok(completed && Number(completed[1]) > 0, 'completed real Wry checks required')
const failures = output.split(/\r?\n/).filter(line => line.startsWith('window_size_native failure: '))
assert.equal(failures.length, Number(completed[2]), 'all reported failures accounted for')
for (const label of ['sizeFirst', 'sizeSecond']) {
    assert.ok(failures.some(line => line.includes(`startup original outer ${label}:`)), `${label} wrong stock startup size reproduced`)
    assert.match(output, new RegExp(`startup geometry ${label}: dpi=[1-9][0-9]* `), 'live DPI must be available')
    for (const frame of ['native', 'thin', 'full']) {
        assert.ok(failures.some(line => line.includes(`original outer minimum ${label} ${frame}:`)), `${label} ${frame} wrong stock minimum reproduced`)
        assert.ok(failures.some(line => line.includes(`minimum resize ${label} ${frame} PhysicalSize { width: 1, height: 1 }:`)), `${label} ${frame} tiny resize mismatch reproduced`)
    }
}
assert.ok(failures.every(line => /failure: (startup original outer|original outer minimum|minimum resize) /.test(line)), 'unexpected failure outside stock geometry')
assert.doesNotMatch(output, /watchdog:|window_size_native error:|AppError:|real WebView2 child HWND not ready/)
console.log('Real Wry stock-geometry control reproduced both startup and all frame minimum mismatches; unrelated checks passed')
