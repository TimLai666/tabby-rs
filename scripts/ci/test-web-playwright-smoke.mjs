#!/usr/bin/env node

import { spawn } from 'node:child_process'
import net from 'node:net'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright-core'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const fixtureServer = path.join(root, 'scripts/test-web-browser-smoke.mjs')
const resultPrefix = 'TABBY_WEB_PLAYWRIGHT_RESULT:'

const server = spawn(process.execPath, [fixtureServer], {
    cwd: root,
    env: process.env,
    stdio: ['ignore', 'pipe', 'pipe'],
})

let serverOutput = ''
let serverError = ''
server.stdout.on('data', chunk => { serverOutput += chunk })
server.stderr.on('data', chunk => { serverError += chunk })

let electron = null
let browser = null
let outputDir = null

try {
    const fixtureURL = await waitForFixtureURL()
    const remoteDebuggingPort = await reservePort()
    outputDir = await mkdtemp(path.join(os.tmpdir(), 'tabby-web-playwright-'))
    const mainPath = path.join(outputDir, 'main.cjs')
    await writeFile(mainPath, createElectronMain(fixtureURL), 'utf8')

    const electronPath = electronBinary()
    const command = process.platform === 'linux' ? 'xvfb-run' : electronPath
    const args = process.platform === 'linux'
        ? ['-a', electronPath, '--no-sandbox', `--remote-debugging-port=${remoteDebuggingPort}`, mainPath]
        : ['--no-sandbox', `--remote-debugging-port=${remoteDebuggingPort}`, mainPath]
    electron = spawn(command, args, {
        cwd: root,
        env: {
            ...process.env,
            ELECTRON_DISABLE_SECURITY_WARNINGS: 'true',
        },
        detached: process.platform !== 'win32',
        stdio: ['ignore', 'pipe', 'pipe'],
    })

    let electronOutput = ''
    let electronError = ''
    electron.stdout.on('data', chunk => { electronOutput += chunk })
    electron.stderr.on('data', chunk => { electronError += chunk })

    browser = await connectToElectron(remoteDebuggingPort, () => {
        if (electron.exitCode !== null) {
            throw new Error(`Electron exited before Playwright connected: ${electronOutput}\n${electronError}`)
        }
    })
    const context = browser.contexts()[0]
    const page = await waitForPage(context)
    page.setDefaultTimeout(15000)
    await page.waitForLoadState('domcontentloaded')

    const result = await exerciseFixture(page)
    console.log(resultPrefix + JSON.stringify(result))
    if (!result.ok) {
        throw new Error(result.failed.join('; ') || 'Playwright web smoke failed')
    }
    console.log(`Web Playwright smoke passed: ${result.checks.join(', ')}`)
} finally {
    await browser?.close().catch(() => {})
    await stopProcess(electron, { processGroup: true })
    await stopProcess(server)
    if (serverError) process.stderr.write(serverError)
    if (outputDir) await rm(outputDir, { recursive: true, force: true })
}

async function exerciseFixture (page) {
    const modifier = process.platform === 'darwin' ? 'Meta' : 'Control'
    await page.locator('#fixture-token').fill('invalid-token')
    await page.locator('#fixture-login button').click()
    const negativeOutput = await page.locator('#fixture-output').textContent()
    const negativeStatus = await page.locator('#fixture-status').textContent()

    await page.locator('#fixture-token').fill('tabby-rs-web-fixture-token')
    await page.locator('#fixture-login button').click()
    await page.locator('#fixture-connect button').click()
    await page.waitForFunction(() => document.getElementById('fixture-status')?.textContent === 'connected')
    await page.locator('#fixture-terminal-input').fill('echo browser')
    await page.locator('#fixture-terminal-input').press('Enter')
    await page.locator('#fixture-resize').click()
    await page.locator('#fixture-sftp-list').click()
    await page.locator('#fixture-telnet-connect').click()
    await page.locator('#fixture-save-settings').click()
    await page.locator('#fixture-load-settings').click()
    await page.locator('#fixture-boot-shared-ui').click()
    try {
        await page.waitForFunction(
            () => document.getElementById('fixture-output')?.textContent?.includes('sftp list response:')
                && document.getElementById('fixture-output')?.textContent?.includes('web Telnet provider connected')
                && document.getElementById('fixture-output')?.textContent?.includes('shared Tabby UI bootstrapped'),
            null,
            { timeout: 20000 },
        )
    } catch (error) {
        throw new Error(`${error instanceof Error ? error.message : String(error)}\n${await page.locator('#fixture-output').textContent()}`)
    }

    try {
        await page.waitForFunction(() => document.querySelectorAll('tab-header').length === 1)
    } catch (error) {
        const diagnostic = await page.evaluate(() => ({
            output: document.getElementById('fixture-output')?.textContent,
            appRoot: document.querySelector('app-root')?.outerHTML.slice(0, 2000),
            body: document.body.textContent?.slice(-2000),
            config: window.ng?.getComponent?.(document.querySelector('app-root'))?.config?.store?.enableWelcomeTab,
            tabs: window.ng?.getComponent?.(document.querySelector('app-root'))?.app?.tabs?.length,
        }))
        throw new Error(`${error instanceof Error ? error.message : String(error)}\n${JSON.stringify(diagnostic)}`)
    }
    const splitHotkeyState = await dispatchHotkey(page, modifier, 's')
    if (splitHotkeyState.matched !== 'split-right') {
        throw new Error(`split-right hotkey was not matched: ${JSON.stringify(splitHotkeyState)}`)
    }
    await page.evaluate(async () => {
        const appRoot = window.ng?.getComponent?.(document.querySelector('app-root'))
        const splitTab = appRoot.app.activeTab
        const newPane = splitTab.tabsService.create({ type: splitTab.focusedTab.constructor })
        await splitTab.addTab(newPane, splitTab.focusedTab, 'r')
        window.ng?.applyChanges?.(appRoot)
    })
    await page.waitForFunction(() => document.querySelectorAll('split-tab-pane-label').length === 2)
    await page.evaluate(async () => {
        const appRoot = window.ng?.getComponent?.(document.querySelector('app-root'))
        appRoot.app.openNewTabRaw({ type: appRoot.app.activeTab.focusedTab.constructor })
        appRoot.app.selectTab(appRoot.app.tabs[0])
        window.ng?.applyChanges?.(appRoot)
    })
    try {
        await page.waitForFunction(() => document.querySelectorAll('tab-header').length === 2)
    } catch (error) {
        const diagnostic = await page.evaluate(() => {
            const appRoot = window.ng?.getComponent?.(document.querySelector('app-root'))
            return {
                output: document.getElementById('fixture-output')?.textContent,
                appTabs: appRoot?.app?.tabs?.map(tab => ({ type: tab.constructor?.name, title: tab.title })),
                tabHeaders: document.querySelectorAll('tab-header').length,
                contentTabs: document.querySelectorAll('tab-body').length,
                body: document.body.textContent?.slice(-2000),
            }
        })
        throw new Error(`${error instanceof Error ? error.message : String(error)}\n${JSON.stringify(diagnostic)}`)
    }
    const nextTabHotkeyState = await dispatchHotkey(page, modifier, 'ArrowRight')
    if (nextTabHotkeyState.matched !== 'next-tab') {
        throw new Error(`next-tab hotkey was not matched: ${JSON.stringify(nextTabHotkeyState)}`)
    }
    await page.evaluate(() => {
        const appRoot = window.ng?.getComponent?.(document.querySelector('app-root'))
        window.ng?.applyChanges?.(appRoot)
    })
    await page.waitForFunction(() => document.querySelectorAll('tab-header.active').length === 1)
    const commandHotkeyState = await dispatchHotkey(page, modifier, 'p')
    if (commandHotkeyState.matched !== 'command-selector') {
        throw new Error(`command-selector hotkey was not matched: ${JSON.stringify(commandHotkeyState)}`)
    }
    await page.waitForSelector('selector-modal')

    const output = await page.locator('#fixture-output').textContent()
    const positiveChecks = [
        'host login accepted',
        'web SSH provider connected',
        'received: "fixture gateway ready',
        'sent: "echo browser',
        'viewport resize observed:',
        'sftp list response:',
        'web Telnet provider connected',
        'settings saved:',
        'settings loaded:',
        'shared plugins loaded: tabby-core, tabby-settings, tabby-terminal, tabby-web',
        'shared Tabby UI bootstrapped',
    ]
    const checks = positiveChecks.map(check => ({ check, passed: output.includes(check) }))
    checks.push(
        { check: 'shared app root rendered', passed: (await page.locator('app-root').locator('> *').count()) > 0 },
        { check: 'shared theme stylesheet applied', passed: (await page.locator('#theme').count()) > 0 },
        { check: 'shared theme variables applied', passed: await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--theme-bg').trim().length > 0) },
        { check: 'shared tab rendered', passed: (await page.locator('tab-header').count()) === 2 },
        { check: 'shared split layout rendered two panes', passed: (await page.locator('split-tab-pane-label').count()) === 2 },
        { check: 'shared tab switch hotkey selected one tab', passed: (await page.locator('tab-header.active').count()) === 1 },
        { check: 'shared command selector hotkey opened modal', passed: (await page.locator('selector-modal').count()) === 1 },
    )
    checks.push({
        check: 'invalid token rejected',
        passed: negativeStatus === 'invalid token' && negativeOutput.includes('host login rejected'),
    })
    return {
        ok: checks.every(check => check.passed),
        checks: checks.filter(check => check.passed).map(check => check.check),
        failed: checks.filter(check => !check.passed).map(check => check.check),
        body: (await page.locator('body').textContent()).slice(-400),
    }
}

async function dispatchHotkey (page, modifier, key) {
    return await page.evaluate(async ({ modifier, key }) => {
        const appRoot = window.ng?.getComponent?.(document.querySelector('app-root'))
        const hotkeys = appRoot?.hotkeys
        if (!hotkeys) {
            throw new Error('shared HotkeysService is unavailable')
        }
        let matched = null
        let filteredMatched = null
        const subscription = hotkeys.unfilteredHotkey$.subscribe(value => { matched = value })
        const filteredSubscription = hotkeys.hotkey$.subscribe(value => { filteredMatched = value })
        const modifierFlags = modifier === 'Meta' ? { metaKey: true } : { ctrlKey: true }
        const code = key === 'ArrowRight' ? key : `Key${key.toUpperCase()}`
        const emit = async eventName => {
            const timeStamp = performance.now()
            hotkeys.pushKeyEvent(eventName, {
                ...modifierFlags,
                shiftKey: true,
                key,
                code,
                eventName,
                timeStamp,
            })
            await new Promise(resolve => setTimeout(resolve, 2))
        }
        await emit('keydown')
        const state = {
            pressedKeystroke: hotkeys.pressedKeystroke,
            pressedHotkey: hotkeys.pressedHotkey,
            tabs: appRoot.app.tabs.length,
            matched,
            activeTabFocus: appRoot.app.activeTab?.hasFocus,
            activeTabType: appRoot.app.activeTab?.constructor?.name,
            focusedTabType: appRoot.app.activeTab?.focusedTab?.constructor?.name,
            paneCount: appRoot.app.activeTab?.getAllTabs?.().length,
            activeElement: document.activeElement?.outerHTML?.slice(0, 300),
            filteredMatched,
        }
        await emit('keyup')
        subscription.unsubscribe()
        filteredSubscription.unsubscribe()
        return state
    }, { modifier, key })
}

function createElectronMain (fixtureURL) {
    return `
const { app, BrowserWindow } = require('electron')
const FIXTURE_URL = ${JSON.stringify(fixtureURL)}

app.commandLine.appendSwitch('disable-gpu-sandbox')
app.commandLine.appendSwitch('disable-software-rasterizer', 'false')

app.whenReady().then(async () => {
  const window = new BrowserWindow({
    show: false,
    width: 900,
    height: 900,
    webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: false },
  })
  window.webContents.on('render-process-gone', (_event, details) => {
    console.error('renderer process exited', details)
    app.exit(1)
  })
  try {
    await window.loadURL(FIXTURE_URL)
  } catch (error) {
    console.error(error)
    app.exit(1)
  }
})

app.on('window-all-closed', () => {})
`
}

async function connectToElectron (port, onUnavailable) {
    let lastError = null
    for (let attempt = 0; attempt < 200; attempt++) {
        try {
            return await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
        } catch (error) {
            lastError = error
            try {
                onUnavailable()
            } catch (exitError) {
                throw exitError
            }
            await wait(100)
        }
    }
    throw new Error(`Playwright could not connect to Electron: ${lastError?.message ?? 'unknown error'}`)
}

async function waitForPage (context) {
    for (let attempt = 0; attempt < 200; attempt++) {
        const page = context.pages().find(candidate => candidate.url() !== 'about:blank')
        if (page) return page
        await wait(100)
    }
    throw new Error('Electron did not expose a loaded fixture page')
}

function wait (milliseconds) {
    return new Promise(resolve => setTimeout(resolve, milliseconds))
}

function reservePort () {
    return new Promise((resolve, reject) => {
        const listener = net.createServer()
        listener.once('error', reject)
        listener.listen(0, '127.0.0.1', () => {
            const address = listener.address()
            const port = typeof address === 'object' && address ? address.port : null
            listener.close(error => error ? reject(error) : resolve(port))
        })
    })
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
                reject(new Error(`Fixture server exited: ${serverOutput}\n${serverError}`))
                return
            }
            setTimeout(check, 25)
        }
        check()
    })
}

function electronBinary () {
    if (process.platform === 'win32') return path.join(root, 'node_modules', 'electron', 'dist', 'electron.exe')
    if (process.platform === 'darwin') return path.join(root, 'node_modules', 'electron', 'dist', 'Electron.app', 'Contents', 'MacOS', 'Electron')
    return path.join(root, 'node_modules', 'electron', 'dist', 'electron')
}

async function stopProcess (child, { processGroup = false } = {}) {
    if (!child || child.exitCode !== null) return
    if (processGroup && process.platform !== 'win32' && child.pid) {
        try {
            process.kill(-child.pid, 'SIGTERM')
        } catch { }
    } else {
        child.kill('SIGTERM')
    }
    await Promise.race([
        new Promise(resolve => child.once('close', resolve)),
        wait(5000),
    ])
    if (child.exitCode === null) {
        if (processGroup && process.platform !== 'win32' && child.pid) {
            try {
                process.kill(-child.pid, 'SIGKILL')
            } catch { }
        } else {
            child.kill('SIGKILL')
        }
    }
}
