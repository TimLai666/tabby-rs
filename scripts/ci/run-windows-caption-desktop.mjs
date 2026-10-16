/** QA adapter for the existing owned controller. Real Windows input is mandatory. */
import { randomUUID } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { exerciseWindowsCaptionLifecycle } from './exercise-windows-caption-lifecycle.mjs'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const literal = value => `'${String(value).replaceAll("'", "''")}'`
const delay = ms => new Promise(resolve => setTimeout(resolve, ms))

export async function runWindowsCaptionDesktop (options) {
    const { page, ownerPid, ownerCreationUtc, ownerHwnd, powerShell, outputDirectory, deadline, captureNativeWindow, snapshotProcessTree } = options
    const primary = { pid: ownerPid, creationUtc: ownerCreationUtc, hwnd: ownerHwnd }
    const pages = new Map([[ownerHwnd, page]])
    const sessions = new Map()
    const busy = new Map()
    const warnings = new Map()
    const evidence = { native: [], terminal: [], secondary: null }
    let sequence = 0
    let inputCleanupFailed = false
    const timeout = cap => {
        const remaining = deadline - Date.now()
        if (remaining <= 0) throw new Error('caption desktop deadline expired')
        return Math.min(cap, remaining)
    }
    const readJson = file => JSON.parse(readFileSync(file, 'utf8').replace(/^\uFEFF/, ''))
    const ps = script => {
        const run = spawnSync(powerShell, ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(script, 'utf16le').toString('base64')],
            { encoding: 'utf8', timeout: timeout(15000), windowsHide: true, shell: false, maxBuffer: 1024 * 1024 })
        if (run.status !== 0 || run.error) throw new Error(run.error?.message ?? String(run.stderr).slice(0, 2000))
        return run.stdout
    }
    const helper = (identity, action, extra = [], legacy = false) => {
        if (inputCleanupFailed) throw new Error('input cleanup unverified; no subsequent GUI actions allowed')
        if (deadline - Date.now() < 45000) throw new Error('insufficient time for native action and mouse recovery')
        const file = path.join(outputDirectory, `${sequence++}-${action}.json`)
        const journal = file + '.mouse-down'
        if (existsSync(file) || existsSync(journal)) throw new Error('native output/journal must be fresh')
        const script = path.join(HERE, legacy ? 'exercise-windows-window-input.ps1' : 'invoke-windows-caption.ps1')
        const run = spawnSync(powerShell, ['-NoProfile', '-NonInteractive', '-File', script,
            '-AppProcessId', String(identity.pid), '-CreationUtc', identity.creationUtc,
            '-WindowHandle', String(identity.hwnd), '-Action', action, '-OutputFile', file, ...extra],
            { encoding: 'utf8', timeout: timeout(25000), windowsHide: true, shell: false, maxBuffer: 1024 * 1024 })
        const record = { action, file, status: run.status, error: run.error?.message ?? null, stderr: String(run.stderr ?? '').slice(0, 4000), data: null }
        evidence.native.push(record)
        if (existsSync(journal)) {
            const recoveryFile = file + '.recovery.json'
            const recovery = spawnSync(powerShell, ['-NoProfile', '-NonInteractive', '-File', path.join(HERE, 'exercise-windows-window-input.ps1'),
                '-AppProcessId', String(identity.pid), '-WindowHandle', String(identity.hwnd), '-Action', 'RecoverMouse',
                '-InputJournal', journal, '-InputHelperPid', String(run.pid), '-OutputFile', recoveryFile],
                { encoding: 'utf8', timeout: timeout(15000), windowsHide: true, shell: false, maxBuffer: 1024 * 1024 })
            record.recovery = { status: recovery.status, error: recovery.error?.message ?? null, data: existsSync(recoveryFile) ? readJson(recoveryFile) : null }
            inputCleanupFailed = recovery.status !== 0 || Boolean(recovery.error) || record.recovery.data?.ok !== true || existsSync(journal)
            throw new Error('native input left a journal; recovery recorded, action acceptance refused')
        }
        if (existsSync(file)) record.data = readJson(file)
        if (record.data?.inputCleanupFailed) inputCleanupFailed = true
        if (run.status !== 0 || run.error || !record.data?.ok) throw new Error(`${action}: ${record.data?.error ?? record.error ?? record.stderr}`)
        const data = record.data
        if (legacy) {
            if (data.appProcessId !== identity.pid || Number(data.hwnd) !== identity.hwnd || data.action !== action) throw new Error('legacy helper returned mismatched identity')
        } else if (data.identity?.pid !== identity.pid || data.identity?.creationUtc !== identity.creationUtc || data.identity?.hwnd !== identity.hwnd || data.action !== action) {
            throw new Error('caption helper returned mismatched identity')
        }
        return data
    }
    const tree = () => {
        const observation = snapshotProcessTree()
        const root = observation.processes?.find(p => p.pid === ownerPid)
        if (observation.error || root?.creationTimeUTC !== ownerCreationUtc) throw new Error('application process tree identity changed')
        return observation.processes
    }
    const sameProcess = (entries, identity) => entries.some(p => p.pid === identity.pid && p.creationTimeUTC === identity.creationUtc)
    const waitGone = async identity => {
        const until = Date.now() + timeout(10000)
        while (Date.now() < until) {
            const rows = tree()
            if (!sameProcess(rows, identity)) {
                const query = JSON.parse(ps(`$ErrorActionPreference='Stop'; $p=Get-CimInstance Win32_Process -Filter "ProcessId = ${identity.pid}"; ConvertTo-Json -Compress -InputObject @{present=($null -ne $p);creationUtc=$(if($p){$p.CreationDate.ToUniversalTime().ToString('o')}else{$null})}`))
                if (!query.present || query.creationUtc !== identity.creationUtc) return
            }
            await delay(100)
        }
        throw new Error('exact owned terminal child remained alive')
    }
    const activate = async identity => {
        const target = pages.get(identity.hwnd)
        if (!target) throw new Error('no page mapped to owned HWND')
        const geometry = helper(identity, 'Snapshot', [], true).after
        const dom = await target.evaluate(() => {
            const terminals = [...document.querySelectorAll('.xterm-screen')].filter(e => e.getBoundingClientRect().width > 100 && e.getBoundingClientRect().height > 100)
            if (terminals.length !== 1) return null
            const r = terminals[0].getBoundingClientRect(), x = r.left + r.width / 2, y = r.top + r.height * 0.75
            return document.elementFromPoint(x, y)?.closest('.xterm') ? { x, y, dpr: devicePixelRatio, innerWidth, innerHeight } : null
        })
        if (!dom || Math.abs(dom.innerWidth * dom.dpr - geometry.client.width) > 3 || Math.abs(dom.innerHeight * dom.dpr - geometry.client.height) > 3) throw new Error('terminal/native client geometry mismatch')
        helper(identity, 'Activate', ['-PointX', String(Math.round(geometry.client.screenX + dom.x * dom.dpr)), '-PointY', String(Math.round(geometry.client.screenY + dom.y * dom.dpr))], true)
    }
    const commandProbe = async (identity, name, focus, hold = false) => {
        if (focus) await activate(identity)
        const marker = `CAPTION_PTY_${randomUUID()}`
        const stem = path.join(outputDirectory, `${sequence++}-${name}`), file = stem + '.txt', metadata = stem + '.json'
        if (existsSync(file) || existsSync(metadata)) throw new Error('terminal probe targets must be fresh')
        const script = `$ErrorActionPreference='Stop'; $self=Get-CimInstance Win32_Process -Filter "ProcessId = $PID"; $parent=Get-CimInstance Win32_Process -Filter "ProcessId = $($self.ParentProcessId)"; if(-not $self -or -not $parent){throw 'terminal process identity unavailable'}; $meta=@{child=@{pid=[int]$self.ProcessId;creationUtc=$self.CreationDate.ToUniversalTime().ToString('o')};shell=@{pid=[int]$parent.ProcessId;creationUtc=$parent.CreationDate.ToUniversalTime().ToString('o')}}|ConvertTo-Json -Compress; [IO.File]::WriteAllText(${literal(metadata)},$meta,[Text.UTF8Encoding]::new($false)); [IO.File]::WriteAllText(${literal(file)},${literal(marker)},[Text.UTF8Encoding]::new($false)); [Console]::WriteLine(${literal(marker)}); ${hold ? 'Start-Sleep -Seconds 120' : ''}`
        const command = `powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ${Buffer.from(script, 'utf16le').toString('base64')}`
        helper(identity, 'TypeTerminal', ['-TextBase64', Buffer.from(command, 'utf16le').toString('base64')])
        const until = Date.now() + timeout(20000)
        while (Date.now() < until && (!existsSync(file) || !readFileSync(file).equals(Buffer.from(marker, 'utf8')))) await delay(100)
        if (!existsSync(file) || !readFileSync(file).equals(Buffer.from(marker, 'utf8'))) throw new Error('native terminal input did not produce exact fresh bytes')
        const meta = readJson(metadata), rows = tree()
        if (!sameProcess(rows, meta.shell) || (hold && !sameProcess(rows, meta.child))) throw new Error('terminal process is not an exact owned descendant')
        const previous = sessions.get(identity.hwnd)
        if (previous && (previous.pid !== meta.shell.pid || previous.creationUtc !== meta.shell.creationUtc)) throw new Error('terminal shell changed unexpectedly')
        sessions.set(identity.hwnd, meta.shell)
        const record = { identity, name, focus, marker, file, meta, exactBytes: true, renderedMarkerCopied: false }
        evidence.terminal.push(record)
        if (hold) { busy.set(identity.hwnd, meta.child); return record }
        const target = pages.get(identity.hwnd)
        const copyUntil = Date.now() + timeout(15000)
        while (Date.now() < copyUntil && !record.renderedMarkerCopied) {
            await target.keyboard.press('Control+Shift+A', { timeout: timeout(5000) })
            await target.keyboard.press('Control+Shift+C', { timeout: timeout(5000) })
            const text = ps("[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); [Console]::Write([string](Get-Clipboard -Raw))")
            record.renderedMarkerCopied = text.replace(/[\r\n]/g, '').includes(marker)
            if (!record.renderedMarkerCopied) await delay(100)
        }
        if (!record.renderedMarkerCopied) throw new Error('terminal marker not read back through OS clipboard')
        await target.screenshot({ path: stem + '.png', timeout: timeout(10000) })
        return record
    }
    let report
    try {
        if (typeof snapshotProcessTree !== 'function' || typeof captureNativeWindow !== 'function') throw new Error('owned process/capture callbacks required')
        mkdirSync(outputDirectory)
        await page.locator('.xterm-helper-textarea:visible').waitFor({ timeout: timeout(10000) })
        report = await exerciseWindowsCaptionLifecycle({
            primary, deadline,
            nativeAction: async ({ identity, action }) => {
                if (action === 'StartBusyTerminal') {
                    await commandProbe(identity, 'busy', false, true)
                    return { ok: true, identity, action, busyChildReady: true }
                }
                if (action === 'StopBusyTerminal') {
                    const child = busy.get(identity.hwnd)
                    if (!child) throw new Error('no recorded busy child')
                    await pages.get(identity.hwnd).keyboard.press('Control+C', { timeout: timeout(5000) })
                    await waitGone(child)
                    return { ok: true, identity, action, stopped: true, sessionsClosed: true }
                }
                const warning = warnings.get(identity.hwnd)
                const data = helper(identity, action, action === 'CancelClose' ? ['-DialogHandle', String(warning?.dialogHwnd), '-ExpectedWarningText', warning?.warningText ?? ''] : [])
                if (action === 'InvokeClose') warnings.set(identity.hwnd, data)
                if (action === 'CloseSecondary') {
                    const shell = sessions.get(identity.hwnd)
                    if (!shell) throw new Error('secondary terminal shell was not recorded')
                    await waitGone(shell)
                    data.sessionsClosed = true
                }
                return data
            },
            verifyTerminal: ({ identity, focus, name }) => commandProbe(identity, name, focus),
            createSecondary: async () => {
                await activate(primary)
                const before = helper(primary, 'EnumerateWindows').ownedWindows
                if (!Array.isArray(before)) throw new Error('owned window inventory missing')
                const context = page.context(), previousPages = context.pages()
                const created = context.waitForEvent('page', { timeout: timeout(15000) })
                void created.catch(() => {})
                await page.keyboard.press('Control+Shift+N', { timeout: timeout(5000) })
                const target = await created
                await target.waitForLoadState('domcontentloaded', { timeout: timeout(15000) })
                const after = helper(primary, 'EnumerateWindows').ownedWindows
                const newWindows = after.filter(w => !before.some(old => old.hwnd === w.hwnd))
                const newPages = context.pages().filter(p => !previousPages.includes(p))
                if (newWindows.length !== 1 || newPages.length !== 1 || newPages[0] !== target) throw new Error('secondary HWND/page mapping is ambiguous')
                const identity = { pid: ownerPid, creationUtc: ownerCreationUtc, hwnd: Number(newWindows[0].hwnd) }
                if (!Number.isSafeInteger(identity.hwnd) || identity.hwnd <= 0 || identity.hwnd === ownerHwnd) throw new Error('invalid secondary HWND')
                const url = target.url()
                if (options.role === 'reference' ? !/^file:/i.test(url) : !/^(?:https?:\/\/tauri\.localhost(?:[/:]|$)|tauri:\/\/localhost(?:[/:]|$))/i.test(url)) throw new Error('unexpected secondary application URL')
                pages.set(identity.hwnd, target)
                await target.keyboard.press('Control+Shift+T', { timeout: timeout(5000) })
                await target.locator('.xterm-helper-textarea:visible').waitFor({ timeout: timeout(15000) })
                evidence.secondary = { identity, before, after, url }
                return identity
            },
            capture: async ({ identity, name }) => {
                if (identity.hwnd !== ownerHwnd) throw new Error('capture callback is scoped to primary HWND')
                const directory = path.join(outputDirectory, name)
                mkdirSync(directory)
                return captureNativeWindow(directory)
            },
        })
    } catch (error) {
        report = { ok: false, primary, error: String(error), inputCleanupFailed }
    }
    report.inputCleanupFailed ||= inputCleanupFailed
    report.desktopEvidence = evidence
    if (existsSync(outputDirectory)) writeFileSync(path.join(outputDirectory, 'caption-desktop-result.json'), JSON.stringify(report, null, 2) + '\n', { flag: 'wx' })
    return report
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    process.stdout.write('runWindowsCaptionDesktop(options): QA-only owned-controller adapter. Requires page, role, exact ownerPid/ownerCreationUtc/ownerHwnd, PowerShell path, fresh outputDirectory, deadline, captureNativeWindow(directory), snapshotProcessTree(). Runs real native input, UIA Invoke, PTY and secondary cleanup. Whole-platform parity and restart remain separate.\n')
    if (!process.argv.includes('--help') && !process.argv.includes('-h')) process.exitCode = 2
}
