import crypto from 'node:crypto'
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import yaml from 'js-yaml'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

function uniqueChecks (features, platform = null) {
    const checks = new Set()
    for (const feature of features ?? []) {
        if (platform && !(feature.platforms ?? []).includes(platform)) continue
        for (const check of feature.tests?.automated ?? []) checks.add(check)
    }
    return [...checks]
}

export function selectParityChecks ({ parity, scripts, platform = null, requestedChecks = null } = {}) {
    const expectedChecks = uniqueChecks(parity?.features, platform)
    const allAvailable = uniqueChecks(parity?.features)
    const selected = requestedChecks
        ? [...new Set(requestedChecks.map(check => check.trim()).filter(Boolean))]
        : expectedChecks
    const missingScripts = selected.filter(check => typeof scripts?.[`test:${check}`] !== 'string')
    const unknownChecks = selected.filter(check => !allAvailable.includes(check))
    return { checks: selected, expectedChecks, missingScripts, unknownChecks }
}

function hashStream () {
    const hash = crypto.createHash('sha256')
    let bytes = 0
    return {
        update (chunk) {
            hash.update(chunk)
            bytes += chunk.length
        },
        finish () {
            return { bytes, sha256: hash.digest('hex') }
        },
    }
}

export function runYarnCheck (name, {
    cwd = root,
    env = process.env,
    spawnImpl = spawn,
    output = process,
} = {}) {
    const executable = process.platform === 'win32' ? 'yarn.cmd' : 'yarn'
    const command = `${executable} run test:${name}`
    const started = Date.now()
    const stdout = hashStream()
    const stderr = hashStream()

    return new Promise(resolve => {
        let settled = false
        const finish = result => {
            if (settled) return
            settled = true
            resolve({
                name,
                command,
                durationMs: Date.now() - started,
                ...result,
                stdout: stdout.finish(),
                stderr: stderr.finish(),
            })
        }
        let child
        try {
            child = spawnImpl(executable, ['run', `test:${name}`], {
                cwd,
                env,
                stdio: ['ignore', 'pipe', 'pipe'],
            })
        } catch (error) {
            finish({ passed: false, status: 'failed', exitCode: null, signal: null, error: error.message })
            return
        }
        child.stdout.on('data', chunk => {
            stdout.update(chunk)
            output.stdout?.write(chunk)
        })
        child.stderr.on('data', chunk => {
            stderr.update(chunk)
            output.stderr?.write(chunk)
        })
        child.on('error', error => finish({ passed: false, status: 'failed', exitCode: null, signal: null, error: error.message }))
        child.on('close', (exitCode, signal) => finish({
            passed: exitCode === 0,
            status: exitCode === 0 ? 'passed' : 'failed',
            exitCode,
            signal,
        }))
    })
}

export async function executeChecks (checks, { runCheck = runYarnCheck, onCheckStart = () => {} } = {}) {
    const results = []
    for (const name of checks) {
        onCheckStart(name)
        results.push(await runCheck(name))
    }
    return results
}

export function createEvidenceReport ({
    results = [],
    missingScripts = [],
    unknownChecks = [],
    expectedChecks = [],
    sourceRevision = 'local',
    platform = null,
    arch = process.arch,
    target = null,
    platformRequiredChecks = [],
    unverifiedRequiredChecks = [],
    generatedAt = new Date().toISOString(),
} = {}) {
    const failures = [
        ...missingScripts.map(check => `missing yarn script for parity check: ${check}`),
        ...unknownChecks.map(check => `requested parity check is not listed in parity/features.yaml: ${check}`),
        ...results.filter(result => !result.passed).map(result => `${result.name}: ${result.error || `exit ${result.exitCode ?? 'unknown'}`}`),
    ]
    const recordedChecks = [...new Set([
        ...results.map(result => result.name),
        ...missingScripts,
        ...unknownChecks,
    ])]
    const missingExpectedChecks = expectedChecks.filter(check => !recordedChecks.includes(check))
    if (missingExpectedChecks.length > 0) failures.push(`missing expected platform checks: ${missingExpectedChecks.join(', ')}`)
    if (unverifiedRequiredChecks.length > 0) failures.push(`unverified required platform checks: ${unverifiedRequiredChecks.join(', ')}`)
    return {
        schemaVersion: 1,
        kind: 'tabby-rs-parity-automated-evidence',
        sourceRevision,
        generatedAt,
        platform,
        arch,
        target,
        platformRequiredChecks,
        unverifiedRequiredChecks,
        checks: results.map(result => ({
            name: result.name,
            command: result.command || `yarn run test:${result.name}`,
            status: result.status || (result.passed ? 'passed' : 'failed'),
            passed: result.passed === true,
            exitCode: result.exitCode ?? null,
            signal: result.signal ?? null,
            durationMs: result.durationMs ?? null,
            error: result.error || null,
            stdout: result.stdout || { bytes: 0, sha256: crypto.createHash('sha256').update('').digest('hex') },
            stderr: result.stderr || { bytes: 0, sha256: crypto.createHash('sha256').update('').digest('hex') },
        })),
        expectedChecks,
        selectedChecks: recordedChecks,
        failures,
        passed: failures.length === 0 && missingScripts.length === 0 && unknownChecks.length === 0 && results.length > 0,
    }
}

function argument (args, name) {
    const index = args.indexOf(name)
    return index === -1 ? null : args[index + 1]
}

function platformName () {
    return { darwin: 'macos', win32: 'windows', linux: 'linux' }[process.platform] || process.platform
}

async function main () {
    const args = process.argv.slice(2)
    const featuresPath = path.join(root, 'parity/features.yaml')
    const packagePath = path.join(root, 'package.json')
    const parity = yaml.load(fs.readFileSync(featuresPath, 'utf8'))
    const packageJson = JSON.parse(fs.readFileSync(packagePath, 'utf8'))
    const platformDocument = yaml.load(fs.readFileSync(path.join(root, 'parity/platform-matrix.yaml'), 'utf8'))
    const platform = argument(args, '--platform') || platformName()
    const requested = argument(args, '--checks')?.split(',') || null
    const selection = selectParityChecks({ parity, scripts: packageJson.scripts, platform, requestedChecks: requested })
    const results = await executeChecks(selection.checks.filter(check => !selection.missingScripts.includes(check) && !selection.unknownChecks.includes(check)), {
        onCheckStart: name => console.log(`Running parity check: ${name}`),
    })
    const arch = argument(args, '--arch') || process.arch
    const target = argument(args, '--target') || process.env.TABBY_RS_TOOLCHAIN || null
    const platformEntry = (platformDocument?.platforms || []).find(entry => entry.target === target)
    const platformRequiredChecks = platformEntry?.requiredChecks || []
    const automatedChecks = new Set(selection.expectedChecks)
    const unverifiedRequiredChecks = platformRequiredChecks.filter(check => !automatedChecks.has(check))
    const report = createEvidenceReport({
        results,
        missingScripts: selection.missingScripts,
        unknownChecks: selection.unknownChecks,
        expectedChecks: selection.expectedChecks,
        sourceRevision: argument(args, '--source-revision') || process.env.GITHUB_SHA || 'local',
        platform,
        arch,
        target,
        platformRequiredChecks,
        unverifiedRequiredChecks,
    })
    const outputPath = path.resolve(argument(args, '--output') || path.join(root, 'parity-automated-evidence.json'))
    fs.mkdirSync(path.dirname(outputPath), { recursive: true })
    fs.writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`)
    console.log(`Parity automated evidence ${report.passed ? 'passed' : 'failed'}: ${report.checks.length}/${report.selectedChecks.length} checks recorded`)
    if (!report.passed) process.exitCode = 1
}

if (fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) await main()
