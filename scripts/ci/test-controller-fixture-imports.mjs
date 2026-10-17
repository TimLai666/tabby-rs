import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath, pathToFileURL } from 'node:url'
import test from 'node:test'

const sourcePath = fileURLToPath(new URL('./capture-windows-apps.mjs', import.meta.url))
const source = readFileSync(sourcePath, 'utf8')
const boundary = source.lastIndexOf('\nlet exitCode = 1\n')
assert.ok(boundary > 0)
for (const fixture of ['capture-lifecycle-fixture.mjs', 'capture-exited-process-fixture.mjs', 'capture-stale-cim-fixture.mjs']) {
    test(fixture + ' imports actual controller dependencies from their source directory', async () => {
        const fixtureSource = readFileSync(new URL(fixture, import.meta.url), 'utf8')
        const start = fixtureSource.indexOf('const loadedSource = ')
        const end = fixtureSource.indexOf('const loadedPath = ', start)
        assert.ok(start >= 0 && end > start)
        const declaration = fixtureSource.slice(start, end)
        const loaded = vm.runInNewContext(declaration + '; loadedSource', {
            source, sourcePath, boundary, path, pathToFileURL, JSON,
            injection: '', postTeardownProbe: '',
            RUNNER_NEEDLE: "const result = spawnSync(powerShell, ['controllable-unused-runner'], {", runnerReplacement: "unused-import-test-replacement",
            runPowerShellNeedle: "const result = spawnSync(powerShell, ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script], {",
        })
        const directory = mkdtempSync(path.join(tmpdir(), 'tabby-controller-import-'))
        try {
            const loadedPath = path.join(directory, 'actual-controller-functions.mjs')
            writeFileSync(loadedPath, loaded, { flag: 'wx' })
            const actual = await import(pathToFileURL(loadedPath))
            assert.equal(typeof actual.snapshotProcessTree, 'function')
            assert.equal(typeof actual.teardownApp, 'function')
        } finally { rmSync(directory, { recursive: true }) }
    })
}
