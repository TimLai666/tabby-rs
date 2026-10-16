// Exercise actual keyboard/PTY input and second-instance native paste on an existing full-app page.
import { randomUUID } from 'node:crypto'
import { spawn, spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, statSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const DIALOG_HELPER = path.join(SCRIPT_DIR, 'press-windows-app-dialog.ps1')
const MAX_OUTPUT = 12000

function message (error) { return error instanceof Error ? error.message : String(error) }
function bounded (value, limit = MAX_OUTPUT) {
    const text = String(value ?? '')
    return { text: text.slice(0, limit), truncated: text.length > limit, totalCharacters: text.length }
}
function remaining (deadline) { const ms = deadline - Date.now(); if (ms <= 0) throw new Error('overall terminal-interaction deadline expired'); return ms }
function delay (ms) { return new Promise(resolve => setTimeout(resolve, ms)) }
async function within (promise, ms, label) {
    let timer
    try {
        return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${label} timed out`)), Math.max(1, ms)) })])
    } finally { clearTimeout(timer) }
}
function psLiteral (value) { return `'${String(value).replaceAll("'", "''")}'` }
function encodedPowerShell (script) { return Buffer.from(script, 'utf16le').toString('base64') }
function makeDir (root, name) { const dir = path.join(root, name); mkdirSync(dir, { recursive: false }); return dir }
function readIfCreated (file) {
    try { return readFileSync(file) } catch (error) { if (error?.code === 'ENOENT') return null; throw error }
}
function pageCount (page) {
    const browser = page.context().browser()
    if (!browser) throw new Error('existing page has no connected browser')
    return browser.contexts().reduce((total, context) => total + context.pages().filter(candidate => !candidate.isClosed()).length, 0)
}
function timeoutFor (deadline, cap) { return Math.max(1, Math.min(cap, remaining(deadline))) }
function runPowerShell (powerShell, script, deadline, cap = 30000) {
    const timeout = timeoutFor(deadline, cap)
    const child = spawnSync(powerShell, ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', encodedPowerShell(script)], { encoding: 'utf8', timeout, maxBuffer: 1024 * 1024, windowsHide: true, shell: false })
    return {
        ok: child.status === 0 && !child.error,
        status: child.status, signal: child.signal, timeoutMs: timeout,
        error: child.error ? message(child.error) : null, stdout: bounded(child.stdout), stderr: bounded(child.stderr),
    }
}
function verifyPrimary (options, result, phase) {
    const script = `$ErrorActionPreference='Stop'; $p=Get-CimInstance Win32_Process -Filter 'ProcessId = ${options.ownerPid}' -ErrorAction Stop; if (-not $p) { throw 'primary process no longer exists' }; $actual=$p.CreationDate.ToUniversalTime().ToString('o'); if ($actual -cne ${psLiteral(options.ownerCreationUtc)}) { throw 'primary process creation identity mismatch' }; [Console]::Write($actual)`; const check = runPowerShell(options.powerShell, script, options.deadline)
    result.primary[phase] = check
    if (!check.ok || check.stdout.text.trim() !== options.ownerCreationUtc) throw new Error(`primary ${phase} identity check failed: ${check.error || check.stderr.text || check.stdout.text}`)
}
async function visibleTextarea (page, deadline) {
    const until = Math.min(deadline, Date.now() + 20000)
    let lastCount = 0
    while (Date.now() < until) {
        remaining(deadline)
        const nodes = await within(page.locator('.xterm-helper-textarea').all(), timeoutFor(deadline, 3000), 'terminal textarea query')
        const visible = []
        for (const node of nodes) if (await within(node.isVisible(), timeoutFor(deadline, 2000), 'terminal visibility query')) visible.push(node)
        lastCount = visible.length
        if (visible.length > 1) throw new Error(`expected one visible .xterm-helper-textarea, found ${visible.length}`)
        if (visible.length === 1) return visible[0]
        await delay(Math.min(100, Math.max(1, until - Date.now())))
    }
    throw new Error(`no visible .xterm-helper-textarea appeared within 20 seconds (found ${lastCount})`)
}
async function focusTextarea (page, deadline) {
    const textarea = await visibleTextarea(page, deadline)
    await within(textarea.focus(), timeoutFor(deadline, 5000), 'terminal focus')
    const focused = await within(textarea.evaluate(element => document.activeElement === element), timeoutFor(deadline, 3000), 'terminal focus verification')
    if (!focused) throw new Error('the visible terminal textarea did not receive focus')
    return textarea
}
async function rowsText (page, deadline) {
    const rows = page.locator('.xterm-rows')
    const count = await within(rows.count(), timeoutFor(deadline, 3000), 'terminal rows query')
    if (count === 0) return ''
    return await within(rows.first().innerText({ timeout: timeoutFor(deadline, 3000) }), timeoutFor(deadline, 4000), 'terminal rows read')
}
async function waitForPtyMarker (page, markerFile, expectedBytes, marker, deadline, phase, updateEvidence) {
    const until = Math.min(deadline, Date.now() + 20000)
    let bytes = null
    let text = ''
    while (Date.now() < until) {
        remaining(deadline)
        bytes = readIfCreated(markerFile)
        if (bytes && !bytes.equals(expectedBytes) && !(bytes.length <= expectedBytes.length && expectedBytes.subarray(0, bytes.length).equals(bytes))) throw new Error(`${phase} PTY-created file bytes did not match the expected UTF-8 payload`)
        text = await rowsText(page, deadline)
        updateEvidence?.({ fileExists: Boolean(bytes), bytesHex: bytes ? bytes.toString('hex') : null, byteLength: bytes?.length ?? 0, visibleMarkerFound: text.includes(marker), visibleRowsDiagnostic: bounded(text, 20000) })
        if (bytes?.equals(expectedBytes)) return { bytes: bytes.toString('hex'), byteLength: bytes.length, ptyBytesMatch: true, visibleRowsDiagnostic: bounded(text, 20000) }
        await delay(Math.min(100, Math.max(1, until - Date.now())))
    }
    throw new Error(`${phase} exact PTY file bytes were not observed within 20 seconds`)
}
function readWindowsClipboard (options) {
    const script = `$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); $text=Get-Clipboard -Raw; if ($null -ne $text) { [Console]::Write([string]$text) }`
    return runPowerShell(options.powerShell, script, options.deadline, 5000)
}
async function proveRenderedClipboardMarker (options, marker, updateEvidence) {
    const until = Math.min(options.deadline, Date.now() + 20000)
    const evidence = { renderedTextSource: 'Windows OS clipboard via Get-Clipboard -Raw', selectAction: 'Control+Shift+A', copyAction: 'Control+Shift+C', readbackAttempts: 0, markerFound: false, lastReadback: null }
    await within(options.page.keyboard.press('Control+Shift+A'), timeoutFor(options.deadline, 5000), 'terminal select-all action')
    evidence.selectActionCompleted = true
    await within(options.page.keyboard.press('Control+Shift+C'), timeoutFor(options.deadline, 5000), 'terminal copy action')
    evidence.copyActionCompleted = true
    updateEvidence?.(evidence)
    while (Date.now() < until) {
        const remainingMs = Math.min(5000, until - Date.now(), remaining(options.deadline))
        const readback = readWindowsClipboard({ ...options, deadline: Date.now() + remainingMs })
        evidence.readbackAttempts++
        evidence.lastReadback = { ok: readback.ok, status: readback.status, error: readback.error, stderr: readback.stderr, text: readback.stdout }
        if (readback.ok && readback.stdout.text.includes(marker)) {
            evidence.markerFound = true
            evidence.copiedText = readback.stdout
            updateEvidence?.(evidence)
            return evidence
        }
        updateEvidence?.(evidence)
        await delay(Math.min(150, Math.max(1, until - Date.now())))
    }
    throw new Error('the unique rendered marker was not read from the Windows OS clipboard within 20 seconds')
}
function runSecondary (options, argv) {
    const timeoutMs = timeoutFor(options.deadline, 30000)
    return new Promise(resolve => {
        let child
        let settled = false
        let stdout = ''; let stderr = ''; let stdoutSize = 0; let stderrSize = 0
        const finish = value => {
            if (settled) return
            settled = true
            clearTimeout(timer)
            resolve({ ...value, pid: child?.pid ?? null, timeoutMs, argv: [...argv], cwd: path.dirname(options.executable), shell: false, stdout: bounded(stdout), stderr: bounded(stderr), stdoutCharacters: stdoutSize, stderrCharacters: stderrSize, descendantsUnverified: true })
        }
        const collect = (target, chunk, isStdout) => {
            const text = chunk.toString('utf8')
            if (isStdout) stdoutSize += text.length
            else stderrSize += text.length
            if (target.length < MAX_OUTPUT) return target + text.slice(0, MAX_OUTPUT - target.length)
            return target
        }
        const timer = setTimeout(() => {
            try { child.kill() } catch { }
            finish({ status: null, signal: 'timeout', error: `secondary process exceeded ${timeoutMs}ms; only its spawned process received kill()` })
        }, timeoutMs)
        try {
            child = spawn(options.executable, argv, {
                cwd: path.dirname(options.executable), env: options.environment, shell: false, windowsHide: false,
                stdio: ['ignore', 'pipe', 'pipe'],
            })
            child.stdout.on('data', chunk => { stdout = collect(stdout, chunk, true) })
            child.stderr.on('data', chunk => { stderr = collect(stderr, chunk, false) })
            child.once('error', error => finish({ status: null, signal: null, error: message(error) }))
            child.once('close', (status, signal) => finish({ status, signal, error: null }))
        } catch (error) { finish({ status: null, signal: null, error: message(error) }) }
    })
}
function invokeDialogHelper (options, action, nonce, directory) {
    const file = path.join(directory, `dialog-${action.toLowerCase()}.json`)
    if (existsSync(file)) throw new Error(`refusing existing dialog helper output: ${file}`)
    const args = ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', DIALOG_HELPER,
        '-AppProcessId', String(options.ownerPid), '-ExpectedTextToken', nonce, '-ButtonText', 'Paste', '-Action', action, '-OutputDirectory', directory]
    const timeoutMs = timeoutFor(options.deadline, 30000)
    const child = spawnSync(options.powerShell, args, {
        encoding: 'utf8', timeout: timeoutMs, maxBuffer: 1024 * 1024, windowsHide: true, shell: false,
    })
    let data = null
    let readError = null
    if (existsSync(file)) {
        try { data = JSON.parse(readFileSync(file, 'utf8')) } catch (error) { readError = message(error) }
    }
    const report = {
        invocation: { status: child.status, signal: child.signal, timeoutMs, error: child.error ? message(child.error) : null, stdout: bounded(child.stdout), stderr: bounded(child.stderr) },
        jsonPath: file, data, readError,
    }
    return report
}
async function captureNative (options, result, key, pid, hwnd, directory) {
    const capture = await within(Promise.resolve().then(() => options.captureNativeWindow({ pid, windowHandle: hwnd, directory })), timeoutFor(options.deadline, 30000), `${key} native screenshot`)
    result.screenshots[key] = capture
    const data = capture?.data || capture
    const imagePath = capture?.screenshotPath || data?.screenshot?.path || data?.screenshotPath
    const capturedHandle = typeof data?.hwnd === 'number' ? data.hwnd : Number.parseInt(String(data?.hwnd ?? ''), 16)
    if (capture?.ok !== true || Number(data?.appProcessId) !== pid || Number(data?.requestedWindowHandle) !== hwnd || !Number.isFinite(capturedHandle) || capturedHandle <= 0 || (hwnd > 0 && capturedHandle !== hwnd) || !imagePath) {
        throw new Error(`${key} native screenshot callback did not prove the requested owned HWND`)
    }
    const candidates = path.isAbsolute(imagePath) ? [imagePath] : [path.resolve(directory, imagePath), path.resolve(options.outputDirectory, imagePath)]
    const resolved = candidates.find(candidate => existsSync(candidate))
    const relative = resolved ? path.relative(options.outputDirectory, resolved) : '..'
    if (!resolved || relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) throw new Error(`${key} native screenshot is outside the fresh evidence directory`)
    if (!existsSync(resolved) || !statSync(resolved).isFile() || statSync(resolved).size === 0) throw new Error(`${key} native screenshot file is missing or empty: ${resolved}`)
    result.screenshots[`${key}Path`] = resolved
}

export async function exerciseWindowsTerminal (options) {
    const result = {
        kind: 'actual-windows-terminal-and-secondary-paste', ok: false, error: null,
        keyboard: { welcome: { found: false, closed: false }, command: null, markerFile: null, pty: null, renderedTextSource: 'Windows OS clipboard via Get-Clipboard -Raw', copyEvidence: null },
        secondary: { argv: null, status: null, error: null, stdout: null, stderr: null, descendantsUnverified: true },
        dialog: { nonce: null, inspect: null, capture: null, confirm: null },
        primary: { role: options?.role ?? null, pid: options?.ownerPid ?? null, creationUtc: options?.ownerCreationUtc ?? null, before: null, after: null, rendererPagesBefore: null, rendererPagesAfter: null, rendererPageCountUnchanged: null },
        screenshots: {},
        unverified: ['Installer behavior, actual application Quit, and supported-platform acceptance require separate evidence.', 'Secondary process descendants were not inspected or terminated.'],
    }
    try {
        if (!options || !options.page || typeof options.page.keyboard?.press !== 'function') throw new Error('an existing connected Playwright page is required')
        if (!Number.isInteger(options.ownerPid) || options.ownerPid <= 0 || !options.ownerCreationUtc) throw new Error('positive ownerPid and ownerCreationUtc are required')
        if (!path.isAbsolute(options.executable) || !/\.exe$/i.test(options.executable) || !statSync(options.executable).isFile()) throw new Error('executable must be an existing absolute .exe path')
        if (!options.environment || typeof options.environment !== 'object') throw new Error('the isolated application environment is required')
        if (!Number.isFinite(options.deadline) || options.deadline <= Date.now()) throw new Error('a future overall deadline is required')
        if (!path.isAbsolute(options.outputDirectory) || !statSync(options.outputDirectory).isDirectory()) throw new Error('outputDirectory must be an existing absolute directory')
        if (!path.isAbsolute(options.powerShell) || !statSync(options.powerShell).isFile()) throw new Error('powerShell must be an existing absolute executable path')
        if (typeof options.captureNativeWindow !== 'function') throw new Error('captureNativeWindow callback is required')

        const root = options.outputDirectory
        const keyboardDir = makeDir(root, 'keyboard')
        const keyboardNativeDir = makeDir(root, 'keyboard-native')
        const dialogDir = makeDir(root, 'paste-dialog')
        const dialogNativeDir = makeDir(root, 'paste-dialog-native')
        const pasteNativeDir = makeDir(root, 'paste-native')
        const keyboardFile = path.join(root, 'keyboard.txt')
        const pasteFile = path.join(root, 'paste.txt')
        if (existsSync(keyboardFile) || existsSync(pasteFile)) throw new Error('marker output file already exists in fresh evidence directory')
        result.keyboard.markerFile = keyboardFile
        result.primary.rendererPagesBefore = pageCount(options.page)
        verifyPrimary(options, result, 'before')

        const welcome = options.page.getByRole('button', { name: 'Close and never show again', exact: true })
        const welcomeMatches = await within(welcome.all(), timeoutFor(options.deadline, 5000), 'welcome button query')
        const visibleWelcome = []
        for (const button of welcomeMatches) if (await within(button.isVisible(), timeoutFor(options.deadline, 2000), 'welcome button visibility')) visibleWelcome.push(button)
        result.keyboard.welcome.found = visibleWelcome.length > 0
        if (visibleWelcome.length > 1) throw new Error('multiple visible exact welcome close buttons are ambiguous')
        if (visibleWelcome.length === 1) {
            await within(visibleWelcome[0].click({ timeout: timeoutFor(options.deadline, 10000) }), timeoutFor(options.deadline, 11000), 'welcome close click')
            result.keyboard.welcome.closed = true
        }

        await within(options.page.bringToFront(), timeoutFor(options.deadline, 5000), 'existing page activation')
        await within(options.page.keyboard.press('Control+Shift+T'), timeoutFor(options.deadline, 5000), 'new local terminal hotkey')
        await focusTextarea(options.page, options.deadline)
        const keyboardMarker = `KEYBOARD_OK_${randomUUID()}`
        const keyboardScript = `$ErrorActionPreference = 'Stop'; [System.IO.File]::WriteAllText(${psLiteral(keyboardFile)}, ${psLiteral(keyboardMarker)}, [System.Text.UTF8Encoding]::new($false)); [Console]::WriteLine(${psLiteral(keyboardMarker)})`
        const keyboardCommand = `powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ${encodedPowerShell(keyboardScript)}`
        if (!/^[\x00-\x7f]+$/.test(keyboardCommand) || keyboardCommand.includes(keyboardMarker)) throw new Error('keyboard command was not ASCII-encoded or exposed its raw marker')
        result.keyboard.command = keyboardCommand
        await within(options.page.keyboard.type(keyboardCommand), timeoutFor(options.deadline, 10000), 'keyboard command typing')
        await within(options.page.keyboard.press('Enter'), timeoutFor(options.deadline, 5000), 'keyboard command submission')
        const keyboardExpected = Buffer.from(keyboardMarker, 'utf8')
        result.keyboard.pty = await waitForPtyMarker(options.page, keyboardFile, keyboardExpected, keyboardMarker, options.deadline, 'keyboard', evidence => { result.keyboard.pty = evidence })
        result.keyboard.copyEvidence = await proveRenderedClipboardMarker(options, keyboardMarker, evidence => { result.keyboard.copyEvidence = evidence })
        const keyboardPng = path.join(keyboardDir, 'client.png')
        await within(options.page.screenshot({ path: keyboardPng, type: 'png' }), timeoutFor(options.deadline, 15000), 'keyboard client screenshot')
        if (!existsSync(keyboardPng) || statSync(keyboardPng).size === 0) throw new Error('keyboard client screenshot was not saved')
        result.screenshots.keyboardClient = keyboardPng
        await captureNative(options, result, 'keyboardNative', options.ownerPid, 0, keyboardNativeDir)

        const pasteMarker = `PASTE_OK_${randomUUID()}`
        const dialogNonce = `PASTE_DIALOG_${randomUUID()}`
        const pasteContent = '貼上驗證|成功'
        const pastePathBase64 = Buffer.from(pasteFile, 'utf8').toString('base64')
        const pasteScript = `try { 'nonce:${dialogNonce}' | Out-Null; [System.IO.File]::WriteAllText([System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String('${pastePathBase64}')), ${psLiteral(pasteContent)}, [System.Text.UTF8Encoding]::new([bool]0)); [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String(${psLiteral(Buffer.from(pasteMarker, 'utf8').toString('base64'))})) | Out-String } catch { exit 1 }`
        const pasteCommand = `powershell.exe -NoLogo -NoProfile -NonInteractive -Command "${pasteScript}"`
        if (!pasteCommand.includes('|') || !/[\u3400-\u9fff]/.test(pasteCommand) || pasteCommand.includes(pasteMarker)) throw new Error('paste command lacks a literal pipe/Chinese string or exposes its raw marker')
        result.dialog.nonce = dialogNonce
        result.secondary.expectedExitCode = 0
        result.secondary.argv = ['paste', pasteCommand]
        const secondary = await runSecondary(options, result.secondary.argv)
        Object.assign(result.secondary, secondary)
        if (secondary.status !== 0 || secondary.error) {
            result.secondary.descendantsUnverified = true
            throw new Error(`secondary paste invocation failed: ${secondary.error || secondary.stderr.text || `exit ${secondary.status}`}`)
        }

        const inspect = invokeDialogHelper(options, 'Inspect', dialogNonce, dialogDir)
        result.dialog.inspect = inspect
        if (!inspect.invocation || inspect.invocation.status !== 0 || inspect.invocation.error || inspect.readError || !inspect.data?.ok || inspect.data.actualAction !== false || inspect.data.processId !== options.ownerPid || inspect.data.nonce !== dialogNonce || Number(inspect.data.hwnd) <= 0 || String(inspect.data.button?.name ?? '').replaceAll('&', '') !== 'Paste') {
            throw new Error(`native Paste dialog Inspect did not prove the exact owned modal: ${inspect.data?.error || inspect.invocation?.stderr?.text || inspect.readError || 'result predicates failed'}`)
        }
        await captureNative(options, result, 'pasteDialogNative', options.ownerPid, Number(inspect.data.hwnd), dialogNativeDir)
        result.dialog.capture = result.screenshots.pasteDialogNative
        const confirm = invokeDialogHelper(options, 'Confirm', dialogNonce, dialogDir)
        result.dialog.confirm = confirm
        if (!confirm.invocation || confirm.invocation.status !== 0 || confirm.invocation.error || confirm.readError || !confirm.data?.ok || confirm.data.actualAction !== true || confirm.data.disappeared !== true || confirm.data.processId !== options.ownerPid || confirm.data.nonce !== dialogNonce || Number(confirm.data.hwnd) !== Number(inspect.data.hwnd)) {
            throw new Error(`native Paste dialog Confirm did not prove one invocation and disappearance: ${confirm.data?.error || confirm.invocation?.stderr?.text || confirm.readError || 'result predicates failed'}`)
        }

        await focusTextarea(options.page, options.deadline)
        await within(options.page.keyboard.press('Enter'), timeoutFor(options.deadline, 5000), 'pasted command submission')
        result.secondary.pty = {}
        result.secondary.pty = await waitForPtyMarker(options.page, pasteFile, Buffer.from(pasteContent, 'utf8'), pasteMarker, options.deadline, 'paste', evidence => { result.secondary.pty = evidence })
        result.secondary.copyEvidence = await proveRenderedClipboardMarker(options, pasteMarker, evidence => { result.secondary.copyEvidence = evidence })
        const pastePng = path.join(root, 'paste-client.png')
        await within(options.page.screenshot({ path: pastePng, type: 'png' }), timeoutFor(options.deadline, 15000), 'paste client screenshot')
        if (!existsSync(pastePng) || statSync(pastePng).size === 0) throw new Error('paste client screenshot was not saved')
        result.screenshots.pasteClient = pastePng
        await captureNative(options, result, 'pasteNative', options.ownerPid, 0, pasteNativeDir)
        result.ok = true
    } catch (error) {
        result.error = message(error)
    }
    if (result.primary.before && options?.powerShell && options?.ownerPid && options?.ownerCreationUtc) {
        try { verifyPrimary(options, result, 'after') } catch (error) {
            result.ok = false
            if (!result.error) result.error = message(error)
            result.unverified.push(`primary after-identity check failed: ${message(error)}`)
        }
    }
    if (options?.page) {
        try {
            result.primary.rendererPagesAfter = pageCount(options.page)
            if (result.primary.rendererPagesBefore !== null) {
                result.primary.rendererPageCountUnchanged = result.primary.rendererPagesBefore === result.primary.rendererPagesAfter
                if (!result.primary.rendererPageCountUnchanged) {
                    result.ok = false
                    if (!result.error) result.error = `renderer page count changed from ${result.primary.rendererPagesBefore} to ${result.primary.rendererPagesAfter}`
                }
            }
        } catch (error) {
            result.ok = false
            if (!result.error) result.error = `final renderer page count unavailable: ${message(error)}`
            result.unverified.push(`final renderer page count unavailable: ${message(error)}`)
        }
    }
    return result
}
