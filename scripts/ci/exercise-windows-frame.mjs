import { randomUUID } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const HELPER = path.join(HERE, 'exercise-windows-window-input.ps1')
const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
const literal = value => `'${String(value).replaceAll("'", "''")}'`

export async function exerciseWindowsFrame (options) {
    const result = { ok: false, scope: 'Actual primary Thin window input, resize, caption actions, PTY and settings visibility only', fullFrameAccepted: false, actions: [], error: null }
    const { page, powerShell, ownerPid, ownerCreationUtc, ownerHwnd, outputDirectory, deadline } = options
    let phase = 'validation'
    const timeout = cap => {
        const left = deadline - Date.now()
        if (left <= 0) throw new Error('full-app frame deadline expired')
        return Math.max(1, Math.min(cap, left))
    }
    const native = (action, extra = []) => {
        const file = path.join(outputDirectory, `${result.actions.length}-${action.toLowerCase()}.json`)
        if (existsSync(file)) throw new Error('native evidence path already exists')
        const run = spawnSync(powerShell, ['-NoProfile', '-NonInteractive', '-File', HELPER,
            '-AppProcessId', String(ownerPid), '-CreationUtc', ownerCreationUtc,
            '-WindowHandle', String(ownerHwnd), '-Action', action, '-OutputFile', file, ...extra],
        { encoding: 'utf8', timeout: timeout(25000), maxBuffer: 1024 * 1024, windowsHide: true, shell: false })
        const evidence = { action, status: run.status, error: run.error?.message ?? null, stderr: String(run.stderr ?? '').slice(0, 4000), data: null }
        result.actions.push(evidence)
        if (existsSync(file)) evidence.data = JSON.parse(readFileSync(file, 'utf8').replace(/^\uFEFF/, ''))
        if (run.status !== 0 || run.error || evidence.data?.ok !== true || evidence.data.appProcessId !== ownerPid || Number(evidence.data.hwnd) !== ownerHwnd || evidence.data.action !== action) {
            throw new Error(`${action} did not prove the requested owned action: ${evidence.data?.error ?? evidence.error ?? evidence.stderr}`)
        }
        return evidence.data.after ?? evidence.data.before
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
        await page.keyboard.press('Control+Shift+A', { timeout: timeout(5000) })
        await page.keyboard.press('Control+Shift+C', { timeout: timeout(5000) })
        const copy = spawnSync(powerShell, ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from("[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); [Console]::Write([string](Get-Clipboard -Raw))", 'utf16le').toString('base64')], { encoding: 'utf8', timeout: timeout(10000), maxBuffer: 1024 * 1024, windowsHide: true, shell: false })
        const copied = String(copy.stdout ?? '')
        if (copy.status !== 0 || copy.error || !copied.replace(/[\r\n]/g, '').includes(marker)) throw new Error('rendered terminal output was not read through the OS clipboard')
        result[`pty_${name}`] = { marker, file, exactBytes: true, renderedMarkerCopied: true }
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
        native('Drag', ['-PointX', String(Math.round(before.client.screenX + dom.x * dom.dpr)), '-PointY', String(Math.round(before.client.screenY + dom.y * dom.dpr)), '-DeltaX', '-60', '-DeltaY', '0'])
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
        await page.getByRole('tab', { name: 'Window', exact: true }).click({ timeout: timeout(10000) })
        const settings = page.locator('window-settings-tab')
        for (const frame of ['Native', 'Thin', 'Full']) await settings.locator(`label[for="frame${frame}"]`).waitFor({ state: 'visible', timeout: timeout(10000) })
        result.settings = { allFrameChoicesVisible: true, thinChecked: await settings.locator('#frameThin').isChecked() }
        if (!result.settings.thinChecked) throw new Error('fresh process did not expose Thin as its selected startup frame')
        await page.screenshot({ path: path.join(outputDirectory, 'window-settings.png'), timeout: timeout(15000) })
        result.ok = true
    } catch (error) {
        result.error = `${phase}: ${error instanceof Error ? error.message : String(error)}`
        try { await page.screenshot({ path: path.join(outputDirectory, 'failure.png'), timeout: timeout(5000) }) } catch (captureError) { result.screenshotError = String(captureError) }
    }
    writeFileSync(path.join(outputDirectory, 'frame-result.json'), `${JSON.stringify(result, null, 2)}\n`, { flag: 'wx' })
    return result
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    process.stdout.write('exercise-windows-frame.mjs exports exerciseWindowsFrame(options) for the owned full-app capture controller. It requires an existing page, PID/creation identity, HWND, deadline and fresh output directory. Exercises actual native input and PTY output; does not accept Full/Native restart, secondary windows or platform parity.\n')
    if (!process.argv.includes('--help') && !process.argv.includes('-h')) process.exitCode = 2
}
