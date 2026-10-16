import { randomUUID } from 'node:crypto'
import { spawn, spawnSync } from 'node:child_process'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const HELPER = path.join(HERE, 'exercise-windows-window-input.ps1')
const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
const literal = value => `'${String(value).replaceAll("'", "''")}'`

export async function exerciseWindowsFrame (options) {
    const result = { ok: false, scope: 'Actual primary Thin window input, resize, caption actions, PTY and settings visibility only', fullFrameAccepted: false, inputCleanupFailed: false, actions: [], error: null }
    const { page, powerShell, ownerPid, ownerCreationUtc, ownerHwnd, outputDirectory, deadline } = options
    let phase = 'validation'
    const timeout = cap => {
        const left = deadline - Date.now()
        if (left <= 0) throw new Error('full-app frame deadline expired')
        return Math.max(1, Math.min(cap, left))
    }
    const recoverMouse = (file, helperPid, record = () => {}) => {
        const journal = file + '.mouse-down'
        if (!existsSync(journal)) return null
        try {
            if (!Number.isInteger(helperPid) || helperPid <= 0) throw new Error('input journal has no known owned helper PID')
            const recoveryFile = file + '.recovery.json'
            const recovery = spawnSync(powerShell, ['-NoProfile', '-NonInteractive', '-File', HELPER,
                    '-AppProcessId', String(ownerPid),
                    '-WindowHandle', String(ownerHwnd), '-Action', 'RecoverMouse',
                    '-InputJournal', journal, '-InputHelperPid', String(helperPid),
                    '-OutputFile', recoveryFile],
                { encoding: 'utf8', timeout: 15000, maxBuffer: 1024 * 1024, windowsHide: true, shell: false })
            const evidence = { status: recovery.status, error: recovery.error?.message ?? null, stderr: String(recovery.stderr ?? '').slice(0, 4000), data: null }
            record(evidence)
            if (existsSync(recoveryFile)) evidence.data = JSON.parse(readFileSync(recoveryFile, 'utf8').replace(/^\uFEFF/, ''))
            if (recovery.status !== 0 || recovery.error || evidence.data?.ok !== true || evidence.data.action !== 'RecoverMouse' || evidence.data.appProcessId !== ownerPid || Number(evidence.data.hwnd) !== ownerHwnd || existsSync(journal)) {
                result.inputCleanupFailed = true
                throw new Error('independent recovery could not confirm that the own mouse press was released')
            }
            return evidence
        } catch (error) {
            result.inputCleanupFailed = true
            throw error
        }
    }
    const native = (action, extra = []) => {
        const file = path.join(outputDirectory, `${result.actions.length}-${action.toLowerCase()}.json`)
        const journal = file + '.mouse-down'
        if (existsSync(file) || existsSync(journal)) throw new Error('native evidence or input journal path already exists')
        if (deadline - Date.now() < 45000) throw new Error('insufficient time for native input and independent mouse recovery')
        const run = spawnSync(powerShell, ['-NoProfile', '-NonInteractive', '-File', HELPER,
            '-AppProcessId', String(ownerPid), '-CreationUtc', ownerCreationUtc,
            '-WindowHandle', String(ownerHwnd), '-Action', action, '-OutputFile', file, ...extra],
        { encoding: 'utf8', timeout: 25000, maxBuffer: 1024 * 1024, windowsHide: true, shell: false })
        const evidence = { action, status: run.status, error: run.error?.message ?? null, stderr: String(run.stderr ?? '').slice(0, 4000), data: null }
        result.actions.push(evidence)
        const journalRemained = existsSync(journal)
        if (journalRemained) {
            evidence.recovery = recoverMouse(file, run.pid, recovery => { evidence.recovery = recovery })
        }
        if (existsSync(file)) evidence.data = JSON.parse(readFileSync(file, 'utf8').replace(/^\uFEFF/, ''))
        if (journalRemained || run.status !== 0 || run.error || evidence.data?.ok !== true || evidence.data.appProcessId !== ownerPid || Number(evidence.data.hwnd) !== ownerHwnd || evidence.data.action !== action) {
            throw new Error(`${action} did not prove the requested owned action: ${evidence.data?.error ?? evidence.error ?? evidence.stderr}`)
        }
        return evidence.data.after ?? evidence.data.before
    }
    const interruptedInput = async extra => {
        if (deadline - Date.now() < 45000) throw new Error('insufficient time for interrupted mouse recovery probe')
        const file = path.join(outputDirectory, 'interrupted-drag.json')
        const journal = file + '.mouse-down'
        if (existsSync(file) || existsSync(journal)) throw new Error('interrupted input evidence already exists')
        let child
        let exit
        let ended = false
        const evidence = { journalObserved: false, killedWhileJournalPresent: false, helperPid: null, recovery: null }
        result.interruptedInput = evidence
        try {
            child = spawn(powerShell, ['-NoProfile', '-NonInteractive', '-File', HELPER,
                '-AppProcessId', String(ownerPid), '-CreationUtc', ownerCreationUtc,
                '-WindowHandle', String(ownerHwnd), '-Action', 'Drag', '-OutputFile', file, ...extra],
            { windowsHide: true, shell: false, stdio: 'ignore' })
            evidence.helperPid = child.pid ?? null
            exit = new Promise((resolve, reject) => {
                child.once('error', error => { ended = true; reject(error) })
                child.once('close', (code, signal) => { ended = true; resolve({ code, signal }) })
            })
            void exit.catch(() => {})
            const until = Date.now() + 15000
            let ready = false
            while (!ended && Date.now() < until) {
                try {
                    const lines = readFileSync(journal, 'utf8').trim().split(/\r?\n/)
                    ready = lines.length === 3 && lines[0] === String(ownerPid) && lines[1] === String(ownerHwnd) && lines[2] === String(child.pid)
                } catch (error) {
                    if (!['ENOENT', 'EBUSY', 'EACCES'].includes(error.code)) throw error
                }
                if (ready) break
                await delay(5)
            }
            if (!ready || ended) throw new Error('interrupted probe did not observe a complete active own mouse journal')
            evidence.journalObserved = true
            await delay(40)
            if (!existsSync(journal) || ended) throw new Error('interrupted probe finished before it could be terminated; no interrupted release is proven')
            evidence.killedWhileJournalPresent = child.kill()
        } finally {
            try {
                if (child && !ended) child.kill()
                if (exit) evidence.exit = await Promise.race([exit, delay(3000).then(() => { throw new Error('owned interrupted helper did not terminate') })])
            } catch (error) {
                result.inputCleanupFailed = true
                throw error
            } finally {
                if (existsSync(journal)) evidence.recovery = recoverMouse(file, child?.pid, recovery => { evidence.recovery = recovery })
            }
        }
        if (!evidence.killedWhileJournalPresent || evidence.recovery?.data?.recoveryButtonWasDown !== true || evidence.recovery?.data?.recoveryReleaseSent !== true) {
            throw new Error('interrupted probe did not prove a real independent LEFTUP after termination')
        }
    }
    const pty = async name => {
        phase = `PTY ${name}`
        const file = path.join(outputDirectory, `pty-${name}.txt`)
        if (existsSync(file)) throw new Error('PTY marker output already exists')
        const marker = `FRAME_PTY_${randomUUID()}`
        const command = `$ErrorActionPreference='Stop'; [IO.File]::WriteAllText(${literal(file)},${literal(marker)},[Text.UTF8Encoding]::new($false)); [Console]::WriteLine(${literal(marker)})`
        const encoded = Buffer.from(command, 'utf16le').toString('base64')
        const textarea = page.locator('.xterm-helper-textarea:visible')
        if (await textarea.count() !== 1) throw new Error('exactly one visible terminal is required')
        await textarea.focus({ timeout: timeout(5000) })
        await page.keyboard.type(`powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ${encoded}`, { timeout: timeout(15000) })
        await page.keyboard.press('Enter', { timeout: timeout(5000) })
        const until = Date.now() + timeout(20000)
        while (Date.now() < until && !existsSync(file)) await delay(100)
        if (!existsSync(file) || !readFileSync(file).equals(Buffer.from(marker, 'utf8'))) throw new Error('PTY did not produce the exact fresh output bytes')
        const copyUntil = Date.now() + timeout(20000)
        let attempts = 0
        let renderedMarkerCopied = false
        while (Date.now() < copyUntil && !renderedMarkerCopied) {
            attempts++
            await page.keyboard.press('Control+Shift+A', { timeout: timeout(5000) })
            await page.keyboard.press('Control+Shift+C', { timeout: timeout(5000) })
            const copy = spawnSync(powerShell, ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from("[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); [Console]::Write([string](Get-Clipboard -Raw))", 'utf16le').toString('base64')], { encoding: 'utf8', timeout: Math.max(1, Math.min(timeout(10000), copyUntil - Date.now())), maxBuffer: 1024 * 1024, windowsHide: true, shell: false })
            const copied = String(copy.stdout ?? '')
            renderedMarkerCopied = copy.status === 0 && !copy.error && copied.replace(/[\r\n]/g, '').includes(marker)
            if (!renderedMarkerCopied) await delay(100)
        }
        if (!renderedMarkerCopied) throw new Error('rendered terminal output was not read through the OS clipboard')
        result[`pty_${name}`] = { marker, file, exactBytes: true, renderedMarkerCopied, clipboardAttempts: attempts }
        await page.screenshot({ path: path.join(outputDirectory, `pty-${name}.png`), timeout: timeout(15000) })
    }
    try {
        if (!Number.isInteger(ownerHwnd) || ownerHwnd <= 0 || !Number.isInteger(ownerPid) || ownerPid <= 0 || !ownerCreationUtc) throw new Error('owned HWND and process creation identity are required')
        await page.bringToFront({ timeout: timeout(5000) })
        const welcome = page.getByRole('button', { name: 'Close and never show again', exact: true })
        if (await welcome.isVisible()) await welcome.click({ timeout: timeout(10000) })
        await page.keyboard.press('Control+Shift+T', { timeout: timeout(5000) })
        await page.locator('.xterm-helper-textarea:visible').waitFor({ timeout: timeout(20000) })
        await pty('before')

        phase = 'drag'
        const before = native('Snapshot')
        result.dragDom = await page.evaluate(() => {
            const candidates = [...document.querySelectorAll('.btn-space.drag, title-bar .title')]
            for (const element of candidates) {
                const rect = element.getBoundingClientRect()
                const style = getComputedStyle(element)
                if (rect.width < 20 || rect.height < 10 || style.visibility === 'hidden' || style.display === 'none') continue
                const x = rect.left + rect.width / 2
                const y = rect.top + rect.height / 2
                const hit = document.elementFromPoint(x, y)
                if (!hit || !(hit === element || element.contains(hit))) continue
                return { x, y, rect: rect.toJSON(), dpr: devicePixelRatio, innerWidth, innerHeight, region: style.getPropertyValue('app-region'), webkitRegion: style.getPropertyValue('-webkit-app-region') }
            }
            return null
        })
        if (!result.dragDom) throw new Error('the actual shared layout has no visible unambiguous drag region')
        const dom = result.dragDom
        if (Math.abs(dom.innerWidth * dom.dpr - before.client.width) > 3 || Math.abs(dom.innerHeight * dom.dpr - before.client.height) > 3) throw new Error('DOM and native client coordinate scales do not match')
        const dragArguments = ['-PointX', String(Math.round(before.client.screenX + dom.x * dom.dpr)), '-PointY', String(Math.round(before.client.screenY + dom.y * dom.dpr)), '-DeltaX', '-60', '-DeltaY', '0']
        if (options.role === 'native') {
            await interruptedInput(dragArguments)
            // The interrupted gesture can move this owned window before release.
            // Re-read the client origin for the independently required full drag.
            const recovered = native('Snapshot')
            dragArguments[1] = String(Math.round(recovered.client.screenX + dom.x * dom.dpr))
            dragArguments[3] = String(Math.round(recovered.client.screenY + dom.y * dom.dpr))
        }
        native('Drag', dragArguments)
        await pty('after-drag')
        phase = 'resize'
        const resized = native('Resize', ['-DeltaX', '-60', '-DeltaY', '0'])
        await pty('after-resize')
        phase = 'caption max/restore'
        native('Maximize')
        await pty('maximized')
        const restored = native('Restore')
        for (const edge of ['left', 'top', 'right', 'bottom']) {
            if (restored.outer[edge] !== resized.outer[edge]) throw new Error(`restore changed the ordinary outer ${edge}`)
        }
        await pty('restored')
        phase = 'settings visibility'
        await page.keyboard.press('Control+,', { timeout: timeout(5000) })
        const navigation = page.locator('settings-tab .nav-pills a')
        result.settingsNavigation = await navigation.allTextContents()
        await navigation.filter({ hasText: /^\s*Window\s*$/ }).click({ timeout: timeout(10000) })
        const settings = page.locator('window-settings-tab')
        for (const frame of ['Native', 'Thin', 'Full']) await settings.locator(`label[for="frame${frame}"]`).waitFor({ state: 'visible', timeout: timeout(10000) })
        result.settings = { allFrameChoicesVisible: true, thinChecked: await settings.locator('#frameThin').isChecked() }
        if (!result.settings.thinChecked) throw new Error('fresh process did not expose Thin as its selected startup frame')
        await page.screenshot({ path: path.join(outputDirectory, 'window-settings.png'), timeout: timeout(15000) })
        result.ok = true
    } catch (error) {
        result.error = `${phase}: ${error instanceof Error ? error.message : String(error)}`
        if (!result.inputCleanupFailed) {
            try { await page.screenshot({ path: path.join(outputDirectory, 'failure.png'), timeout: timeout(5000) }) } catch (captureError) { result.screenshotError = String(captureError) }
        }
    }
    writeFileSync(path.join(outputDirectory, 'frame-result.json'), `${JSON.stringify(result, null, 2)}\n`, { flag: 'wx' })
    return result
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    process.stdout.write('exercise-windows-frame.mjs exports exerciseWindowsFrame(options) for the owned full-app capture controller. It requires an existing page, PID/creation identity, HWND, deadline and fresh output directory. Exercises actual native input and PTY output; does not accept Full/Native restart, secondary windows or platform parity.\n')
    if (!process.argv.includes('--help') && !process.argv.includes('-h')) process.exitCode = 2
}
