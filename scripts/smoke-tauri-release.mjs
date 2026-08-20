#!/usr/bin/env node

import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import { execFile, spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'
import { fileURLToPath } from 'node:url'

import { auditBundle } from './check-tauri-bundle.mjs'
import { resolveMacosApplicationIcon } from './macos-app-icon.mjs'

const execFileAsync = promisify(execFile)
const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const canonicalMacosIcon = path.join(repositoryRoot, 'build', 'mac', 'icon.icns')

const args = process.argv.slice(2)
const argument = name => {
    const index = args.indexOf(name)
    return index === -1 ? null : args[index + 1]
}
const required = name => {
    const value = argument(name)
    assert.ok(value, `${name} is required`)
    return value
}

const staging = path.resolve(required('--staging'))
const platform = required('--platform')
const reportPath = argument('--output')
const planOnly = args.includes('--plan')
assert.ok(['linux', 'macos', 'windows'].includes(platform), `unsupported platform: ${platform}`)
assert.ok(fs.existsSync(staging), `release staging directory does not exist: ${staging}`)

function filesWithExtension (extension) {
    const matches = []
    const visit = directory => {
        for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
            const filePath = path.join(directory, entry.name)
            if (entry.isDirectory()) visit(filePath)
            else if (entry.isFile() && entry.name.toLowerCase().endsWith(extension.toLowerCase())) matches.push(filePath)
        }
    }
    visit(staging)
    return matches
}

function oneArtifact (extension) {
    const files = filesWithExtension(extension)
    assert.equal(files.length, 1, `expected exactly one ${extension} artifact, found ${files.length}`)
    return files[0]
}

function sha256 (filePath) {
    return crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex')
}

function assertMacosIcon (filePath, label) {
    assert.ok(fs.existsSync(canonicalMacosIcon), `canonical macOS icon is missing: ${canonicalMacosIcon}`)
    assert.ok(fs.existsSync(filePath), `${label} is missing: ${filePath}`)
    assert.equal(
        sha256(filePath),
        sha256(canonicalMacosIcon),
        `${label} does not match build/mac/icon.icns`,
    )
}

function writeReport (operations) {
    const report = {
        schemaVersion: 1,
        platform,
        staging,
        planOnly,
        passed: true,
        operations,
    }
    if (reportPath) {
        fs.mkdirSync(path.dirname(path.resolve(reportPath)), { recursive: true })
        fs.writeFileSync(path.resolve(reportPath), `${JSON.stringify(report, null, 2)}\n`)
    }
    console.log(JSON.stringify(report, null, 2))
}

function assertCommand (command, argsList, options = {}) {
    return execFileAsync(command, argsList, { encoding: 'utf8', windowsHide: true, ...options })
}

async function waitForFile (filePath, timeoutMs = 60000) {
    const deadline = Date.now() + timeoutMs
    while (Date.now() < deadline) {
        if (fs.existsSync(filePath)) {
            const report = JSON.parse(fs.readFileSync(filePath, 'utf8'))
            assert.equal(report.schemaVersion, 1, 'installer smoke marker schema is unsupported')
            assert.equal(report.ready, true, 'installer smoke marker is not ready')
            assert.ok(report.identity && typeof report.identity === 'object', 'installer smoke marker has no identity')
            return report
        }
        await new Promise(resolve => setTimeout(resolve, 250))
    }
    throw new Error(`installed application did not write ready marker within ${timeoutMs}ms: ${filePath}`)
}

async function waitForPathGone (filePath, timeoutMs = 15000) {
    const deadline = Date.now() + timeoutMs
    while (Date.now() < deadline) {
        if (!fs.existsSync(filePath)) return
        await new Promise(resolve => setTimeout(resolve, 250))
    }
    assert.ok(!fs.existsSync(filePath), `uninstaller left the application executable behind: ${filePath}`)
}

async function terminate (child) {
    if (child.exitCode !== null || child.signalCode !== null) return
    child.kill()
    await new Promise(resolve => {
        const timer = setTimeout(() => {
            child.kill('SIGKILL')
            resolve()
        }, 3000)
        child.once('close', () => {
            clearTimeout(timer)
            resolve()
        })
    })
}

function waitForReadyOrExit (child, marker, processOutput) {
    const exited = new Promise((_, reject) => {
        child.once('error', reject)
        child.once('close', (code, signal) => {
            reject(new Error(
                [
                    `installed application exited before writing ready marker (code=${code ?? 'null'}, signal=${signal ?? 'null'}, marker=${marker})`,
                    `stdout=${processOutput.stdout.trim() || '<empty>'}`,
                    `stderr=${processOutput.stderr.trim() || '<empty>'}`,
                ].join('\n'),
            ))
        })
    })
    // The race may resolve from the marker first. Attach a handler now so a
    // later normal process exit cannot become an unhandled rejection.
    exited.catch(() => {})
    return Promise.race([waitForFile(marker), exited])
}

async function launchAndCheck (executable, cwd, environment, { preserveUserData = false } = {}) {
    const markerDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-installer-smoke-ready-'))
    const marker = path.join(markerDirectory, 'ready.marker')
    const dataDirectory = path.join(markerDirectory, 'data')
    const userDataSentinel = path.join(dataDirectory, 'user-data-preservation-sentinel.txt')
    const processOutput = { stdout: '', stderr: '' }
    const appendOutput = (stream, chunk) => {
        const maxOutputLength = 12000
        if (processOutput[stream].length >= maxOutputLength) return
        processOutput[stream] += chunk
        if (processOutput[stream].length > maxOutputLength) {
            processOutput[stream] = `${processOutput[stream].slice(0, maxOutputLength)}\n[output truncated]`
        }
    }
    if (environment.HOME) {
        fs.mkdirSync(environment.HOME, { recursive: true })
    }
    let keepMarkerDirectory = false
    const child = spawn(executable, [], {
        cwd,
        env: {
            ...process.env,
            ...environment,
            TABBY_RS_INSTALLER_SMOKE_READY_FILE: marker,
            TABBY_RS_INSTALLER_SMOKE_DATA_DIR: dataDirectory,
        },
        stdio: ['ignore', 'pipe', 'pipe'],
        windowsHide: true,
    })
    child.stdout.setEncoding('utf8')
    child.stderr.setEncoding('utf8')
    child.stdout.on('data', chunk => appendOutput('stdout', chunk))
    child.stderr.on('data', chunk => appendOutput('stderr', chunk))
    try {
        const readyReport = await waitForReadyOrExit(child, marker, processOutput)
        const identity = readyReport.identity
        assert.equal(identity.productName, 'Tabby RS')
        assert.equal(identity.appIdentifier, 'io.tabbyrs.app')
        assert.equal(identity.cliName, 'tabby-rs')
        assert.equal(identity.urlScheme, 'tabby-rs')
        assert.equal(identity.dataDirName, 'tabby-rs')
        assert.equal(path.resolve(identity.dataDir), path.resolve(dataDirectory), 'installer smoke used an unexpected data directory')
        assert.match(path.basename(identity.executable).toLowerCase(), /^tabby-rs(?:\.exe)?$/)
        await terminate(child)
        if (preserveUserData) {
            fs.mkdirSync(dataDirectory, { recursive: true })
            fs.writeFileSync(userDataSentinel, 'preserve-user-data\n')
            keepMarkerDirectory = true
        }
        return {
            appIdentifier: identity.appIdentifier,
            cliName: identity.cliName,
            dataDirectory: identity.dataDir,
            urlScheme: identity.urlScheme,
            userDataSentinel: preserveUserData ? userDataSentinel : null,
            cleanupDirectory: preserveUserData ? markerDirectory : null,
        }
    } finally {
        await terminate(child)
        if (!keepMarkerDirectory) {
            fs.rmSync(markerDirectory, { recursive: true, force: true })
        }
    }
}

function reportIdentity (identity) {
    return {
        appIdentifier: identity.appIdentifier,
        cliName: identity.cliName,
        dataDirectory: identity.dataDirectory,
        userDataPreserved: identity.userDataPreserved === true,
        urlScheme: identity.urlScheme,
    }
}

function assertUserDataPreserved (identity) {
    assert.ok(identity.userDataSentinel, 'installer smoke did not create a user-data sentinel')
    assert.ok(fs.existsSync(identity.userDataSentinel), 'uninstall removed user data')
    assert.equal(fs.readFileSync(identity.userDataSentinel, 'utf8'), 'preserve-user-data\n', 'uninstall changed user data')
    identity.userDataPreserved = true
    fs.rmSync(identity.cleanupDirectory, { recursive: true, force: true })
}

function findFile (directory, predicate) {
    const entries = fs.readdirSync(directory, { withFileTypes: true })
    for (const entry of entries) {
        const candidate = path.join(directory, entry.name)
        if (entry.isFile() && predicate(candidate)) return candidate
        if (entry.isDirectory()) {
            const result = findFile(candidate, predicate)
            if (result) return result
        }
    }
    return null
}

function auditInstalledBundle (directory, label, operations) {
    const report = auditBundle(directory)
    assert.equal(report.passed, true, `${label} bundle audit failed: ${JSON.stringify(report.findings)}`)
    operations.push({ action: 'audit', target: label, files: report.files.length })
}

function packageName (command, artifact, query) {
    return execFileAsync(command, query(artifact), { encoding: 'utf8' }).then(({ stdout }) => stdout.trim())
}

async function smokeWindows () {
    const installer = oneArtifact('.exe')
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-installer-smoke-windows-'))
    const installDirectory = path.join(root, 'installed')
    const operations = [{ action: 'install', artifact: path.basename(installer), destination: installDirectory }]
    try {
        if (!planOnly) {
            await assertCommand(installer, ['/S', `/D=${installDirectory}`])
            const executable = findFile(installDirectory, file => path.basename(file).toLowerCase() === 'tabby-rs.exe')
            assert.ok(executable, `NSIS installer did not install tabby-rs.exe under ${installDirectory}`)
            const identity = await launchAndCheck(executable, path.dirname(executable), { APPDATA: path.join(root, 'appdata') }, { preserveUserData: true })
            auditInstalledBundle(installDirectory, 'windows-nsis-install', operations)
            const uninstaller = findFile(installDirectory, file => path.basename(file).toLowerCase() === 'uninstall.exe')
            assert.ok(uninstaller, 'NSIS installer did not install an uninstaller')
            await assertCommand(uninstaller, ['/S'])
            await waitForPathGone(executable)
            assertUserDataPreserved(identity)
            operations.push({ action: 'launch', executable: path.relative(installDirectory, executable), identity: reportIdentity(identity) })
            operations.push({ action: 'uninstall', executable: path.relative(installDirectory, uninstaller) })
        }
        writeReport(operations)
    } finally {
        fs.rmSync(root, { recursive: true, force: true })
    }
}

async function smokeMacos () {
    const dmg = oneArtifact('.dmg')
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-installer-smoke-macos-'))
    const mount = path.join(root, 'mount')
    const installDirectory = path.join(root, 'Applications')
    const operations = [{ action: 'mount', artifact: path.basename(dmg) }, { action: 'copy', destination: installDirectory }]
    try {
        if (!planOnly) {
            fs.mkdirSync(mount)
            fs.mkdirSync(installDirectory)
            await assertCommand('hdiutil', ['attach', '-nobrowse', '-readonly', '-mountpoint', mount, dmg])
            try {
                const app = fs.readdirSync(mount).find(file => file.endsWith('.app'))
                assert.ok(app, `DMG contains no .app bundle: ${mount}`)
                const sourceApp = path.join(mount, app)
                const installedApp = path.join(installDirectory, app)
                assertMacosIcon(path.join(mount, '.VolumeIcon.icns'), 'DMG volume icon')
                const applicationIcon = resolveMacosApplicationIcon(sourceApp)
                assertMacosIcon(applicationIcon, 'macOS application icon')
                operations.push({ action: 'icon-audit', volumeIcon: '.VolumeIcon.icns', applicationIcon: path.relative(sourceApp, applicationIcon) })
                fs.cpSync(sourceApp, installedApp, { recursive: true })
                const executableDirectory = path.join(installedApp, 'Contents', 'MacOS')
                const executable = findFile(executableDirectory, file => path.basename(file) === 'tabby-rs')
                assert.ok(executable, `application bundle has no executable: ${installedApp}`)
                const identity = await launchAndCheck(executable, installedApp, { HOME: path.join(root, 'home') }, { preserveUserData: true })
                auditInstalledBundle(installedApp, 'macos-dmg-app', operations)
                fs.rmSync(installedApp, { recursive: true, force: true })
                assert.ok(!fs.existsSync(installedApp), 'DMG copy could not be removed')
                assertUserDataPreserved(identity)
                operations.push({ action: 'launch', executable: path.relative(installedApp, executable), identity: reportIdentity(identity) })
                operations.push({ action: 'uninstall', executable: path.relative(installDirectory, installedApp) })
            } finally {
                await assertCommand('hdiutil', ['detach', mount, '-force'])
            }
        }
        writeReport(operations)
    } finally {
        fs.rmSync(root, { recursive: true, force: true })
    }
}

async function smokeAppImage (artifact, root, operations) {
    const extractDirectory = path.join(root, 'appimage')
    fs.mkdirSync(extractDirectory)
    await assertCommand(artifact, ['--appimage-extract'], { cwd: staging })
    const extracted = path.join(staging, 'squashfs-root')
    assert.ok(fs.existsSync(extracted), `AppImage extraction did not create ${extracted}`)
    fs.renameSync(extracted, extractDirectory)
    const executable = path.join(extractDirectory, 'AppRun')
    assert.ok(fs.existsSync(executable), `AppImage extraction has no AppRun: ${extractDirectory}`)
    fs.chmodSync(executable, 0o755)
    operations.push({ action: 'install', artifact: path.basename(artifact), target: 'AppImage/AppRun' })
    const identity = await launchAndCheck(executable, extractDirectory, { XDG_CONFIG_HOME: path.join(root, 'config') }, { preserveUserData: true })
    auditInstalledBundle(extractDirectory, 'linux-appimage-extract', operations)
    fs.rmSync(extractDirectory, { recursive: true, force: true })
    assert.ok(!fs.existsSync(extractDirectory), 'AppImage uninstall left the extracted application behind')
    assertUserDataPreserved(identity)
    operations.push({ action: 'launch', executable: 'AppImage/AppRun', identity: reportIdentity(identity) })
    operations.push({ action: 'uninstall', target: 'AppImage/AppRun' })
}

async function smokeDeb (artifact, root, operations) {
    const packageRoot = path.join(root, 'deb-root')
    const adminDirectory = path.join(packageRoot, 'var', 'lib', 'dpkg')
    fs.mkdirSync(adminDirectory, { recursive: true })
    fs.writeFileSync(path.join(adminDirectory, 'status'), '')
    const name = await packageName('dpkg-deb', artifact, value => ['-f', value, 'Package'])
    await assertCommand('dpkg', [`--root=${packageRoot}`, `--admindir=${adminDirectory}`, `--instdir=${packageRoot}`, '--unpack', artifact], { cwd: root })
    const installed = findFile(packageRoot, file => path.basename(file) === 'tabby-rs')
    assert.ok(installed, `DEB installation did not place the application under ${packageRoot}`)
    operations.push({ action: 'install', artifact: path.basename(artifact), package: name, manager: 'dpkg' })
    const identity = await launchAndCheck(installed, path.dirname(installed), { HOME: path.join(root, 'home') }, { preserveUserData: true })
    auditInstalledBundle(packageRoot, 'linux-deb-install', operations)
    await assertCommand('dpkg', [`--root=${packageRoot}`, `--admindir=${adminDirectory}`, `--instdir=${packageRoot}`, '--purge', name], { cwd: root })
    assert.ok(!fs.existsSync(installed), 'DEB purge left the application executable behind')
    assertUserDataPreserved(identity)
    operations.push({ action: 'launch', executable: path.relative(packageRoot, installed), identity: reportIdentity(identity) })
    operations.push({ action: 'uninstall', package: name, manager: 'dpkg' })
}

async function smokeRpm (artifact, root, operations) {
    const packageRoot = path.join(root, 'rpm-root')
    const dbDirectory = path.join(packageRoot, 'var', 'lib', 'rpm')
    fs.mkdirSync(dbDirectory, { recursive: true })
    const name = await packageName('rpm', artifact, value => ['-qp', '--qf', '%{NAME}', value])
    await assertCommand('rpm', [`--root=${packageRoot}`, '--dbpath=/var/lib/rpm', '--initdb'], { cwd: root })
    await assertCommand('rpm', [`--root=${packageRoot}`, '--dbpath=/var/lib/rpm', '--install', artifact], { cwd: root })
    const installed = findFile(packageRoot, file => path.basename(file) === 'tabby-rs')
    assert.ok(installed, `RPM installation did not place the application under ${packageRoot}`)
    operations.push({ action: 'install', artifact: path.basename(artifact), package: name, manager: 'rpm' })
    const identity = await launchAndCheck(installed, path.dirname(installed), { HOME: path.join(root, 'home') }, { preserveUserData: true })
    auditInstalledBundle(packageRoot, 'linux-rpm-install', operations)
    await assertCommand('rpm', [`--root=${packageRoot}`, '--dbpath=/var/lib/rpm', '--erase', name], { cwd: root })
    assert.ok(!fs.existsSync(installed), 'RPM erase left the application executable behind')
    assertUserDataPreserved(identity)
    operations.push({ action: 'launch', executable: path.relative(packageRoot, installed), identity: reportIdentity(identity) })
    operations.push({ action: 'uninstall', package: name, manager: 'rpm' })
}

async function smokeLinux () {
    const artifacts = { appimage: oneArtifact('.appimage'), deb: oneArtifact('.deb'), rpm: oneArtifact('.rpm') }
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-installer-smoke-linux-'))
    const operations = Object.entries(artifacts).map(([kind, artifact]) => ({ action: 'inspect', kind, artifact: path.basename(artifact) }))
    try {
        if (!planOnly) {
            await smokeAppImage(artifacts.appimage, root, operations)
            await smokeDeb(artifacts.deb, root, operations)
            await smokeRpm(artifacts.rpm, root, operations)
        }
        writeReport(operations)
    } finally {
        fs.rmSync(path.join(staging, 'squashfs-root'), { recursive: true, force: true })
        fs.rmSync(root, { recursive: true, force: true })
    }
}

if (platform === 'windows') await smokeWindows()
else if (platform === 'macos') await smokeMacos()
else await smokeLinux()
