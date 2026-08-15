import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(fileURLToPath(new URL('..', import.meta.url)))
const smoke = path.join(root, 'scripts', 'smoke-tauri-release.mjs')
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-release-smoke-test-'))
const staging = path.join(work, 'release-staging')
fs.mkdirSync(staging)
const smokeSource = fs.readFileSync(smoke, 'utf8')
assert.match(smokeSource, /TABBY_RS_INSTALLER_SMOKE_READY_FILE/)
assert.match(smokeSource, /TABBY_RS_INSTALLER_SMOKE_DATA_DIR/)
assert.match(smokeSource, /appIdentifier.*io\.tabbyrs\.app|identity\.appIdentifier.*'io\.tabbyrs\.app'/s)
assert.match(smokeSource, /unexpected data directory/)
assert.match(smokeSource, /userDataSentinel/)
assert.match(smokeSource, /userDataPreserved/)
assert.match(smokeSource, /uninstall removed user data/)
assert.match(smokeSource, /uninstall changed user data/)
assert.match(smokeSource, /path\.basename\(file\) === 'tabby-rs'/)
assert.match(smokeSource, /icon-audit/)
assert.match(smokeSource, /DMG volume icon/)
assert.doesNotMatch(smokeSource, /TABBY_RS_BENCHMARK_READY_FILE/)

for (const [directory, fixture] of [
    ['windows', 'tabby-rs-setup.exe'],
    ['macos', 'tabby-rs.dmg'],
    ['linux', 'tabby-rs.AppImage'],
    ['linux', 'tabby-rs.deb'],
    ['linux', 'tabby-rs.rpm'],
]) {
    fs.mkdirSync(path.join(staging, directory), { recursive: true })
    fs.writeFileSync(path.join(staging, directory, fixture), fixture)
}

function run (platform, expected) {
    const output = execFileSync(process.execPath, [smoke, '--staging', staging, '--platform', platform, '--plan'], { encoding: 'utf8' })
    const report = JSON.parse(output)
    assert.equal(report.passed, true)
    assert.equal(report.planOnly, true)
    assert.deepEqual(report.operations.map(operation => operation.artifact).filter(Boolean), expected)
}

run('windows', ['tabby-rs-setup.exe'])
run('macos', ['tabby-rs.dmg'])
run('linux', ['tabby-rs.AppImage', 'tabby-rs.deb', 'tabby-rs.rpm'])

fs.rmSync(path.join(staging, 'linux', 'tabby-rs.rpm'))
assert.throws(
    () => execFileSync(process.execPath, [smoke, '--staging', staging, '--platform', 'linux', '--plan'], {
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'pipe'],
    }),
    /expected exactly one \.rpm artifact/,
)

console.log('Tauri release installer smoke fixtures passed')
