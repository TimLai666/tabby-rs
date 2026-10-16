/** Real UI frame selection and restart acceptance for the owned Windows controller. */
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { createOwnedCaptionDriver } from './run-windows-caption-desktop.mjs'
import { exerciseWindowsFrameRestart } from './exercise-windows-frame-restart.mjs'
const yaml = createRequire(import.meta.url)('js-yaml')
const delay = ms => new Promise(resolve => setTimeout(resolve, ms))

export async function runWindowsFrameRestart (options) {
    const { deadline, outputDirectory, powerShell, role, portableDataRoot, replaceApplication, captureNativeWindow, consumedArguments } = options
    let current = { page: options.page, identity: options.identity }
    const evidence = { processes: [], modes: [], terminalDrivers: [] }
    let driver
    const timeout = cap => {
        const remaining = deadline - Date.now()
        if (remaining <= 0) throw new Error('frame restart desktop deadline expired')
        return Math.min(cap, remaining)
    }
    const configureDriver = () => {
        const directory = path.join(outputDirectory, `process-${current.identity.pid}-${evidence.terminalDrivers.length}`)
        mkdirSync(directory)
        driver = createOwnedCaptionDriver({ ...options, page: current.page, role, powerShell,
            ownerPid: current.identity.pid, ownerCreationUtc: current.identity.creationUtc, ownerHwnd: current.identity.hwnd,
            outputDirectory: directory, snapshotProcessTree: () => options.snapshotProcessTree(current.identity.pid),
            captureNativeWindow: directory => captureNativeWindow({ identity: current.identity, directory }) })
        evidence.terminalDrivers.push(driver.evidence)
    }
    const terminal = async () => {
        const page = current.page
        if (await page.locator('settings-tab:visible').count()) await page.keyboard.press('Control+Shift+W', { timeout: timeout(5000) })
        if (await page.locator('.xterm-helper-textarea:visible').count() === 0) await page.keyboard.press('Control+Shift+T', { timeout: timeout(5000) })
        await page.locator('.xterm-helper-textarea:visible').waitFor({ timeout: timeout(15000) })
        if (await page.locator('.xterm-helper-textarea:visible').count() !== 1) throw new Error('restart requires exactly one active terminal pane')
    }
    const settings = async () => {
        await current.page.keyboard.press('Control+,', { timeout: timeout(5000) })
        await current.page.locator('settings-tab .nav-pills a').filter({ hasText: /^\s*Window\s*$/ }).click({ timeout: timeout(10000) })
        return current.page.locator('window-settings-tab')
    }
    const configFrame = () => yaml.load(readFileSync(path.join(portableDataRoot, 'config.yaml'), 'utf8'))?.appearance?.frame
    const visualState = async () => {
        const native = driver.helper(current.identity, 'Snapshot')
        const dom = await current.page.evaluate(() => ({
            titleBars: [...document.querySelectorAll('title-bar')].filter(e => e.getBoundingClientRect().height > 0 && getComputedStyle(e).display !== 'none').length,
            canvases: [...document.body.children].filter(e => e.tagName === 'CANVAS' && e.getAttribute('aria-hidden') === 'true' && e.style.pointerEvents === 'none' && e.style.zIndex === '2147483647')
                .map(e => ({ visible: getComputedStyle(e).display !== 'none' && e.getBoundingClientRect().width > 0, width: e.width, height: e.height })),
        }))
        return { native, dom }
    }
    const sameBounds = (a, b) => a && b && a.width > 0 && a.height > 0 && b.width > 0 && b.height > 0 &&
        ['x', 'y', 'width', 'height'].every(key => Number.isFinite(a[key]) && Number.isFinite(b[key]) && Math.abs(a[key] - b[key]) <= 1)
    const applied = (state, mode, fullscreen = false) => {
        const native = state.native.after, dom = state.dom
        if (!native?.client || !Number.isInteger(native.style)) return false
        const caption = (native.style & 0x00c00000) === 0x00c00000
        const buttons = (state.native.children ?? []).filter(b => b.class === 'Button' || b.class === 'BUTTON').filter(b => Number(b.parent) === current.identity.hwnd && b.visible)
        const canvases = dom.canvases.filter(c => c.visible)
        if (fullscreen) return sameBounds(native.bounds, native.monitor) && !caption && buttons.length === 0 && canvases.length === 0 && dom.titleBars === 0
        if (!caption || native.minimized) return false
        if (dom.titleBars !== (mode === 'full' ? 1 : 0)) return false
        if (mode === 'native') return native.client.y > native.bounds.y + 10 && buttons.length === 0 && canvases.length === 0
        if (Math.abs(native.client.y - native.bounds.y) > 1) return false
        return role === 'native' ? buttons.length === 3 && canvases.length === 1 : true
    }
    let report
    try {
        if (!consumedArguments?.nonce || !consumedArguments.counterFile || typeof replaceApplication !== 'function') throw new Error('actual consumed-argument and replacement evidence is required')
        mkdirSync(outputDirectory)
        configureDriver()
        report = await exerciseWindowsFrameRestart({ identity: current.identity, deadline,
            selectFrame: async ({ identity, mode }) => {
                await terminal()
                await driver.commandProbe(identity, `${mode}-primary-before-restart`, true)
                const secondary = await driver.createSecondary()
                await driver.commandProbe(secondary, `${mode}-secondary-before-restart`, true)
                await driver.activate(identity)
                const panel = await settings(), label = mode[0].toUpperCase() + mode.slice(1)
                await panel.locator(`label[for="frame${label}"]`).click({ timeout: timeout(10000) })
                await current.page.getByRole('button', { name: 'Restart the app to apply changes', exact: true }).waitFor({ timeout: timeout(10000) })
                const until = Date.now() + timeout(10000)
                while (configFrame() !== mode && Date.now() < until) await delay(100)
                const persisted = configFrame() === mode
                const oldWindows = driver.helper(identity, 'EnumerateWindows').ownedWindows
                if (!Array.isArray(oldWindows) || driver.pages.size !== 2 || ![identity.hwnd, secondary.hwnd].every(hwnd => oldWindows.some(w => Number(w.hwnd) === hwnd))) throw new Error('restart must own both primary and secondary windows')
                evidence.modes.push({ mode, oldWindows, sessions: [...driver.sessions.values()], persisted })
                return { persisted, restartRequested: true }
            },
            restart: async ({ identity, mode }) => {
                const old = evidence.modes.at(-1)
                const replacement = await replaceApplication({ identity, mode, page: current.page, oldWindows: old.oldWindows, sessions: old.sessions,
                    directory: path.join(outputDirectory, `${mode}-replacement`) })
                evidence.processes.push(replacement.evidence)
                if (replacement.ownedReplacement && replacement.identity && replacement.page) {
                    current = { identity: replacement.identity, page: replacement.page }
                    configureDriver()
                }
                return replacement
            },
            verifyFrame: async ({ mode }) => {
                const panel = await settings(), label = mode[0].toUpperCase() + mode.slice(1)
                const selected = await panel.locator(`#frame${label}`).isChecked()
                const state = await visualState()
                evidence.modes.at(-1).applied = state
                await current.page.screenshot({ path: path.join(outputDirectory, `${mode}-settings.png`), timeout: timeout(15000) })
                return { selectedFrame: selected ? mode : null, configReadback: configFrame(), frameApplied: applied(state, mode) }
            },
            verifyTerminal: async ({ identity, mode, phase }) => {
                await terminal()
                return driver.commandProbe(identity, `${mode}-${phase}`, true)
            },
            exerciseFullscreen: async ({ mode }) => {
                const before = await visualState()
                await current.page.keyboard.press('F11', { timeout: timeout(5000) })
                let during, until = Date.now() + timeout(15000)
                do { during = await visualState(); if (applied(during, mode, true)) break; await delay(100) } while (Date.now() < until)
                await current.page.screenshot({ path: path.join(outputDirectory, `${mode}-fullscreen.png`), timeout: timeout(15000) })
                const entered = sameBounds(during.native.after.bounds, during.native.after.monitor) && (during.native.after.style & 0x00c00000) === 0
                const hidden = applied(during, mode, true)
                await current.page.keyboard.press('F11', { timeout: timeout(5000) })
                let after; until = Date.now() + timeout(15000)
                do { after = await visualState(); if (applied(after, mode) && sameBounds(after.native.after.bounds, before.native.after.bounds)) break; await delay(100) } while (Date.now() < until)
                evidence.modes.at(-1).fullscreen = { before, during, after }
                return { entered, controlsHidden: hidden, exited: (after.native.after.style & 0x00c00000) === 0x00c00000 && sameBounds(after.native.after.bounds, before.native.after.bounds) && after.native.after.minimized === before.native.after.minimized && after.native.after.maximized === before.native.after.maximized, controlsRestored: applied(after, mode) }
            },
            checkConsumedArguments: async () => {
                const bytes = readFileSync(consumedArguments.counterFile)
                const expected = Buffer.from(consumedArguments.nonce + '\n', 'utf8')
                const snapshot = driver.helper(current.identity, 'Snapshot')
                if (!Array.isArray(snapshot.ownedDialogs)) throw new Error('owned modal inventory missing')
                if (snapshot.ownedDialogs.some(d => d.text.includes(consumedArguments.nonce))) throw new Error('consumed CLI run prompt replayed after restart')
                return { expectedLaunchCount: 1, observedLaunchCount: bytes.equals(expected) ? 1 : -1 }
            },
            capture: async ({ identity, mode }) => {
                const directory = path.join(outputDirectory, `${mode}-native-window`)
                mkdirSync(directory)
                return captureNativeWindow({ identity, directory })
            },
        })
    } catch (error) { report = { ok: false, identity: current.identity, modes: [], error: String(error) } }
    report.desktopEvidence = evidence
    report.inputCleanupFailed = driver?.getInputCleanupFailed() ?? false
    writeFileSync(path.join(outputDirectory, 'frame-restart-result.json'), JSON.stringify(report, null, 2) + '\n', { flag: 'wx' })
    return report
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    process.stdout.write('runWindowsFrameRestart(options): private owned-controller entry point. Requires an existing verified page/identity, fresh outputDirectory, deadline, role, PowerShell, portableDataRoot, actual replaceApplication boundary, snapshotProcessTree(pid), native capture callback, and one consumed CLI counter/nonce. Verifies Native/Thin/Full settings through real process replacement, two-window/session cleanup, fullscreen and PTY. No whole-platform acceptance.\n')
    if (!process.argv.includes('--help') && !process.argv.includes('-h')) process.exitCode = 2
}
