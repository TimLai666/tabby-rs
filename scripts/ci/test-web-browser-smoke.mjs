#!/usr/bin/env node

import { spawn } from 'node:child_process'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const fixtureServer = path.join(root, 'scripts/test-web-browser-smoke.mjs')
const resultPrefix = 'TABBY_WEB_BROWSER_RESULT:'

const browserScript = `
async () => {
    const wait = ms => new Promise(resolve => setTimeout(resolve, ms))
    const fill = (id, value) => {
        const input = document.getElementById(id)
        input.value = value
        input.dispatchEvent(new Event('input', { bubbles: true }))
    }
    const submit = id => document.getElementById(id).dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    const click = id => (id.includes(' ') ? document.querySelector('#' + id) : document.getElementById(id)).click()

    fill('fixture-token', 'invalid-token')
    submit('fixture-login')
    const negativeOutput = document.getElementById('fixture-output').textContent
    const negativeStatus = document.getElementById('fixture-status').textContent

    fill('fixture-token', 'tabby-rs-web-fixture-token')
    submit('fixture-login')
    click('fixture-connect button')
    await wait(500)
    fill('fixture-terminal-input', 'echo browser')
    document.getElementById('fixture-terminal-input').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    click('fixture-resize')
    click('fixture-sftp-list')
    click('fixture-save-settings')
    click('fixture-load-settings')
    click('fixture-boot-shared-ui')
    await wait(7000)

    const output = document.getElementById('fixture-output').textContent
    const positiveChecks = [
        'host login accepted',
        'gateway connected',
        'received: "fixture gateway ready',
        'sent: "echo browser',
        'viewport resize observed:',
        'sent: "SFTP-LIST /',
        'settings saved:',
        'settings loaded:',
        'shared plugins loaded: tabby-core, tabby-settings, tabby-terminal, tabby-web',
        'shared Tabby UI bootstrapped',
    ]
    const checks = positiveChecks.map(check => ({ check, passed: output.includes(check) }))

    checks.push({ check: 'invalid token rejected', passed: negativeStatus === 'invalid token' && negativeOutput.includes('host login rejected') })

    return {
        ok: checks.every(check => check.passed),
        checks: checks.filter(check => check.passed).map(check => check.check),
        failed: checks.filter(check => !check.passed).map(check => check.check),
        body: document.body.textContent.slice(-400),
    }
}
`

const server = spawn(process.execPath, [fixtureServer], {
    cwd: root,
    env: process.env,
    stdio: ['ignore', 'pipe', 'pipe'],
})

let serverOutput = ''
let serverError = ''
server.stdout.on('data', chunk => { serverOutput += chunk })
server.stderr.on('data', chunk => { serverError += chunk })

try {
    const url = await waitForFixtureURL()
    const outputDir = await mkdtemp(path.join(os.tmpdir(), 'tabby-web-browser-'))
    try {
        const mainPath = path.join(outputDir, 'main.cjs')
        await writeFile(mainPath, `
const { app, BrowserWindow } = require('electron')
const RESULT_PREFIX = ${JSON.stringify(resultPrefix)}
const BROWSER_SCRIPT = ${JSON.stringify(browserScript)}
const FIXTURE_URL = ${JSON.stringify(url)}

app.commandLine.appendSwitch('disable-gpu-sandbox')
app.commandLine.appendSwitch('disable-software-rasterizer', 'false')

app.whenReady().then(async () => {
  const window = new BrowserWindow({
    show: false,
    width: 900,
    height: 900,
    webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: false },
  })
  let crashed = null
  window.webContents.on('render-process-gone', (_event, details) => { crashed = details })
  try {
    await window.loadURL(FIXTURE_URL)
    const result = await window.webContents.executeJavaScript('(' + BROWSER_SCRIPT + ')()')
    if (crashed) result.ok = false, result.failed.push('renderer process exited')
    console.log(RESULT_PREFIX + JSON.stringify(result))
    if (!result.ok) process.exitCode = 1
  } catch (error) {
    console.log(RESULT_PREFIX + JSON.stringify({ ok: false, checks: [], failed: [String(error)] }))
    process.exitCode = 1
  } finally {
    window.destroy()
    app.quit()
  }
})

app.on('window-all-closed', () => {})
`, 'utf8')

        const electron = electronBinary()
        const command = process.platform === 'linux' ? 'xvfb-run' : electron
        const args = process.platform === 'linux'
            ? ['-a', electron, '--no-sandbox', mainPath]
            : ['--no-sandbox', mainPath]
        const result = await run(command, args, {
            ...process.env,
            ELECTRON_DISABLE_SECURITY_WARNINGS: 'true',
        })
        process.stdout.write(result.stdout)
        process.stderr.write(result.stderr)
        const payload = extractPayload(result.stdout)
        if (result.code !== 0 || !payload.ok) {
            throw new Error(payload.failed?.join('; ') || `Web browser fixture failed with exit code ${result.code}`)
        }
        console.log(`Web browser smoke passed: ${payload.checks.join(', ')}`)
    } finally {
        await rm(outputDir, { recursive: true, force: true })
    }
} finally {
    server.kill('SIGTERM')
    if (serverError) process.stderr.write(serverError)
}

function waitForFixtureURL () {
    return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error(`Fixture server did not start: ${serverOutput}`)), 15000)
        const check = () => {
            const match = serverOutput.match(/Web browser fixture: (http:\/\/127\.0\.0\.1:\d+\/)/)
            if (match) {
                clearTimeout(timeout)
                resolve(match[1])
                return
            }
            if (server.exitCode !== null) {
                clearTimeout(timeout)
                reject(new Error(`Fixture server exited: ${serverOutput}\\n${serverError}`))
                return
            }
            setTimeout(check, 25)
        }
        check()
    })
}

function extractPayload (stdout) {
    for (const line of stdout.split(/\\r?\\n/).reverse()) {
        const index = line.indexOf(resultPrefix)
        if (index !== -1) return JSON.parse(line.slice(index + resultPrefix.length))
    }
    return { ok: false, checks: [], failed: ['browser fixture did not emit a structured result'] }
}

function electronBinary () {
    if (process.platform === 'win32') return path.join(root, 'node_modules', 'electron', 'dist', 'electron.exe')
    if (process.platform === 'darwin') return path.join(root, 'node_modules', 'electron', 'dist', 'Electron.app', 'Contents', 'MacOS', 'Electron')
    return path.join(root, 'node_modules', 'electron', 'dist', 'electron')
}

function run (command, args, env) {
    return new Promise((resolve, reject) => {
        const child = spawn(command, args, { cwd: root, env })
        let stdout = ''
        let stderr = ''
        child.stdout.on('data', chunk => { stdout += chunk })
        child.stderr.on('data', chunk => { stderr += chunk })
        child.on('error', reject)
        child.on('close', code => resolve({ code, stdout, stderr }))
    })
}
