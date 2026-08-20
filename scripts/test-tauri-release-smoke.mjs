import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(fileURLToPath(new URL('..', import.meta.url)))
const smoke = path.join(root, 'scripts', 'smoke-tauri-release.mjs')
const releaseWorkflow = path.join(root, '.github', 'workflows', 'release.yml')
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-release-smoke-test-'))
const staging = path.join(work, 'release-staging')
fs.mkdirSync(staging)
const smokeSource = fs.readFileSync(smoke, 'utf8')
const releaseWorkflowSource = fs.readFileSync(releaseWorkflow, 'utf8')
const smokeStepStart = releaseWorkflowSource.indexOf('      - name: Smoke test release installer')
const smokeStepEnd = releaseWorkflowSource.indexOf('      - name:', smokeStepStart + 1)
assert.ok(smokeStepStart >= 0, 'release workflow is missing installer smoke step')
const smokeStep = releaseWorkflowSource.slice(smokeStepStart, smokeStepEnd)
assert.match(smokeStep, /if \[ "\$\{\{ matrix\.platform \}\}" = "linux" \]; then/)
assert.match(smokeStep, /xvfb-run -a node scripts\/smoke-tauri-release\.mjs/)
assert.match(smokeStep, /else\s+node scripts\/smoke-tauri-release\.mjs/s)
assert.match(smokeSource, /TABBY_RS_INSTALLER_SMOKE_READY_FILE/)
assert.match(smokeSource, /TABBY_RS_INSTALLER_SMOKE_DATA_DIR/)
assert.match(smokeSource, /exited before writing ready marker/)
assert.match(smokeSource, /stdio: \['ignore', 'pipe', 'pipe'\]/)
assert.match(smokeSource, /stdout=.*processOutput\.stdout/s)
assert.match(smokeSource, /stderr=.*processOutput\.stderr/s)
assert.match(smokeSource, /waitForReadyOrExit/)
assert.match(smokeSource, /appIdentifier.*io\.tabbyrs\.app|identity\.appIdentifier.*'io\.tabbyrs\.app'/s)
assert.match(smokeSource, /unexpected data directory/)
assert.match(smokeSource, /userDataSentinel/)
assert.match(smokeSource, /userDataPreserved/)
assert.match(smokeSource, /uninstall removed user data/)
assert.match(smokeSource, /uninstall changed user data/)
assert.match(smokeSource, /waitForPathGone/)
assert.match(smokeSource, /path\.basename\(file\) === 'tabby-rs'/)
assert.match(smokeSource, /--force-not-root/)
assert.match(smokeSource, /icon-audit/)
assert.match(smokeSource, /DMG volume icon/)
assert.match(smokeSource, /resolveMacosApplicationIcon/)
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
