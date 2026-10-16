#!/usr/bin/env node
import { createHash } from 'node:crypto'
import { spawn, spawnSync } from 'node:child_process'
import { createReadStream, createWriteStream, existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const WINDOW_CAPTURE_SCRIPT = path.join(SCRIPT_DIR, 'capture-windows-app-window.ps1')

const KIND = 'actual-windows-app-startup-capture'
const TOTAL_TIMEOUT_MS = 5 * 60 * 1000
const TERMINAL_TIMEOUT_MS = 14 * 60 * 1000
const APP_TIMEOUT_MS = 90 * 1000
const MAX_DOM_HTML = 200000
const MAX_VISIBLE_TEXT = 20000

const FLAGS = {
    '--reference-exe': 'referenceExe',
    '--rs-exe': 'rsExe',
    '--output': 'output',
    '--source-revision': 'sourceRevision',
    '--reference-port': 'referencePort',
    '--rs-port': 'rsPort',
    '--flow': 'flow',
}

const REQUIRED_FLAGS = [
    ['referenceExe', '--reference-exe'],
    ['rsExe', '--rs-exe'],
    ['output', '--output'],
    ['sourceRevision', '--source-revision'],
]

const logStreams = []

function usage () {
    return [
        'Capture the actual Windows Tabby (reference) and Tabby RS (native) applications.',
        '',
        'Usage:',
        '  node scripts/ci/capture-windows-apps.mjs --reference-exe ABS --rs-exe ABS \\',
        '       --output FRESH_ABS_DIR --source-revision SHA [--reference-port 9222] [--rs-port 9223]',
        '',
        'Optional --flow startup, terminal (keyboard and CLI paste), or frame (configured caption color, native drag/resize and PTY).',
        'CDP ports must be distinct integers from 1 through 65535; defaults are 9222 and 9223.',
        'Both executable paths must name actual application copies with empty adjacent data directories.',
        'The reference receives PORTABLE_EXECUTABLE_FILE to disable its automatic updater.',
        'The native process receives common diagnostic WebView2 arguments and an explicit isolated portable WebView profile.',
        'Terminal flow executes bounded marker commands, replaces the Windows clipboard, and confirms',
        'only the owned native Paste modal containing its unique test token. It saves failure screenshots.',
        'All flows forcibly stop the owned app processes afterward; application Quit remains untested.',
        'Runs on Windows only. Produces actual application diagnostics under --output.',
        'Exit codes: 0 required checks passed (or help), 1 capture failed, 2 invalid arguments.',
        'This capture supplies diagnostic evidence; it does not establish full parity.',
    ].join('\n')
}

function message (error) {
    return error instanceof Error ? error.message : String(error)
}

function bounded (text, limit) {
    const value = text === null || text === undefined ? '' : String(text)
    if (value.length <= limit) return value
    return `${value.slice(0, limit)}\n...[truncated ${value.length - limit} characters]`
}

function pushBounded (list, value, limit) {
    if (list.length < limit) list.push(value)
}

function parseCliArgs (argv) {
    const values = {}
    const errors = []
    for (let i = 0; i < argv.length; i++) {
        const token = argv[i]
        if (!token.startsWith('--')) {
            errors.push(`unexpected argument: ${token}`)
            continue
        }
        let name = token
        let inlineValue = null
        const equals = token.indexOf('=')
        if (equals !== -1) {
            name = token.slice(0, equals)
            inlineValue = token.slice(equals + 1)
        }
        const key = FLAGS[name]
        if (!key) {
            errors.push(`unknown flag: ${name}`)
            continue
        }
        if (Object.prototype.hasOwnProperty.call(values, key)) {
            errors.push(`duplicate flag: ${name}`)
            continue
        }
        let value = inlineValue
        if (value === null) {
            const next = argv[i + 1]
            if (next === undefined || next.startsWith('--')) {
                errors.push(`missing value for ${name}`)
                continue
            }
            value = next
            i++
        }
        if (value === '') {
            errors.push(`missing value for ${name}`)
            continue
        }
        values[key] = value
    }
    return { values, errors }
}

function isWindowsExePath (label, value, errors) {
    if (!path.isAbsolute(value)) {
        errors.push(`${label}: must be an absolute path`)
        return false
    }
    if (path.extname(value).toLowerCase() !== '.exe') {
        errors.push(`${label}: must be a Windows .exe executable`)
        return false
    }
    let stat
    try {
        stat = statSync(value)
    } catch {
        errors.push(`${label}: executable not found: ${value}`)
        return false
    }
    if (!stat.isFile()) {
        errors.push(`${label}: is not a regular file: ${value}`)
        return false
    }
    return true
}

function parsePort (label, raw, fallback, errors) {
    if (raw === undefined) return fallback
    if (!/^\d+$/.test(raw)) {
        errors.push(`${label}: must be an integer`)
        return fallback
    }
    const value = Number(raw)
    if (value < 1 || value > 65535) {
        errors.push(`${label}: must be between 1 and 65535`)
        return fallback
    }
    return value
}

function validateOptions (values) {
    const errors = []
    for (const [key, flag] of REQUIRED_FLAGS) {
        if (values[key] === undefined) errors.push(`missing required flag: ${flag}`)
    }

    let referenceExeValid = false
    let rsExeValid = false
    if (values.referenceExe !== undefined) referenceExeValid = isWindowsExePath('--reference-exe', values.referenceExe, errors)
    if (values.rsExe !== undefined) rsExeValid = isWindowsExePath('--rs-exe', values.rsExe, errors)

    if (referenceExeValid && rsExeValid) {
        try {
            const referenceResolved = path.resolve(values.referenceExe).toLowerCase()
            const rsResolved = path.resolve(values.rsExe).toLowerCase()
            if (referenceResolved === rsResolved) {
                errors.push('--reference-exe and --rs-exe must be different executables')
            }
        } catch (error) {
            errors.push(`could not compare executables: ${message(error)}`)
        }
    }

    if (values.output !== undefined) {
        if (!path.isAbsolute(values.output)) {
            errors.push('--output: must be an absolute path')
        } else if (existsSync(values.output)) {
            errors.push(`--output: refuses to reuse existing output: ${values.output}`)
        }
    }

    if (values.sourceRevision !== undefined && !/^[0-9a-fA-F]{7,64}$/.test(values.sourceRevision)) {
        errors.push('--source-revision: must be a hexadecimal revision identifier')
    }

    const referencePort = parsePort('--reference-port', values.referencePort, 9222, errors)
    const rsPort = parsePort('--rs-port', values.rsPort, 9223, errors)
    if (referencePort === rsPort) {
        errors.push('--reference-port and --rs-port must differ')
    }

    if (process.platform !== 'win32') {
        errors.push(`this capture controller requires Windows (win32); current platform is ${process.platform}`)
    }

    const flow = values.flow ?? 'startup'
    if (!['startup', 'terminal', 'frame'].includes(flow)) errors.push('--flow: must be startup, terminal or frame')
    return { errors, referencePort, rsPort, flow }
}

function sha256File (file) {
    return new Promise((resolve, reject) => {
        const hash = createHash('sha256')
        const stream = createReadStream(file)
        stream.on('data', chunk => hash.update(chunk))
        stream.on('error', reject)
        stream.on('end', () => resolve(hash.digest('hex')))
    })
}

function ensurePortableRoot (exePath, captionColor = false) {
    const dataRoot = path.join(path.dirname(exePath), 'data')
    if (!existsSync(dataRoot)) mkdirSync(dataRoot)
    if (readdirSync(dataRoot).length) throw new Error(`refuses to launch with pre-existing portable data: ${dataRoot}`)
    if (!statSync(dataRoot).isDirectory()) {
        throw new Error(`portable data path is not a directory: ${dataRoot}`)
    }
    if (captionColor) writeFileSync(path.join(dataRoot, 'config.yaml'), "version: 1\nappearance:\n  frame: native\nterminal:\n  colorScheme:\n    name: 'Tabby Default'\n    foreground: '#b050d0'\n    background: '#171717'\n    cursor: '#bbbbbb'\n    colors:\n      - '#000000'\n      - '#ff615a'\n      - '#b1e969'\n      - '#ebd99c'\n      - '#5da9f6'\n      - '#e86aff'\n      - '#82fff7'\n      - '#dedacf'\n      - '#313131'\n      - '#f58c80'\n      - '#ddf88f'\n      - '#eee5b2'\n      - '#a5c7ff'\n      - '#ddaaff'\n      - '#b7fff9'\n      - '#ffffff'\n", { flag: 'wx' })
    return dataRoot
}

function delay (milliseconds) {
    return new Promise(resolve => setTimeout(resolve, milliseconds))
}

function withTimeout (promise, milliseconds, label) {
    let timer
    const timeout = new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out after ${milliseconds}ms`)), milliseconds)
    })
    return Promise.race([promise, timeout]).finally(() => clearTimeout(timer))
}

function resolvePowerShell () {
    for (const candidate of ['pwsh', 'powershell']) {
        const probe = spawnSync(candidate, ['-NoProfile', '-Command', '(Get-Process -Id $PID).Path'], { encoding: 'utf8', windowsHide: true, timeout: 15000 })
        const executable = probe.stdout?.trim()
        if (!probe.error && probe.status === 0 && executable && path.isAbsolute(executable) && existsSync(executable)) return executable
    }
    throw new Error('No usable Windows PowerShell executable was found')
}

function runPowerShell (powerShell, script, timeoutMs) {
    const result = spawnSync(powerShell, ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script], {
        encoding: 'utf8',
        windowsHide: true,
        timeout: timeoutMs,
        maxBuffer: 16 * 1024 * 1024,
    })
    return {
        ok: !result.error && result.status === 0,
        status: result.status,
        stdout: (result.stdout || '').trim(),
        stderr: (result.stderr || '').trim(),
        error: result.error ? message(result.error) : null,
    }
}

const SNAPSHOT_SCRIPT = `$ErrorActionPreference = 'Stop'
$root = __ROOT__
$procs = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, CreationDate, Name, CommandLine
$children = @{}
foreach ($p in $procs) {
  $parent = [int]$p.ParentProcessId
  if (-not $children.ContainsKey($parent)) { $children[$parent] = New-Object System.Collections.Generic.List[int] }
  $children[$parent].Add([int]$p.ProcessId)
}
$result = New-Object System.Collections.Generic.List[object]
$stack = New-Object System.Collections.Generic.Stack[int]
$stack.Push([int]$root)
$seen = @{}
while ($stack.Count -gt 0) {
  $id = $stack.Pop()
  if ($seen.ContainsKey($id)) { continue }
  $seen[$id] = $true
  $match = $procs | Where-Object { $_.ProcessId -eq $id } | Select-Object -First 1
  if ($match) {
    $result.Add([ordered]@{ pid = [int]$match.ProcessId; parentPid = [int]$match.ParentProcessId; creationTimeUTC = $match.CreationDate.ToUniversalTime().ToString('o'); executableBasename = [string]$match.Name; browserCommandLine = $(if($match.Name -ieq "msedgewebview2.exe"){$match.CommandLine}else{$null}) })
  }
  if ($match -and $children.ContainsKey($id)) {
    foreach ($child in $children[$id]) {
      $candidate = $procs | Where-Object { $_.ProcessId -eq $child } | Select-Object -First 1
      if ($candidate -and $candidate.CreationDate -ge $match.CreationDate) { $stack.Push([int]$child) }
    }
  }
}
ConvertTo-Json -Compress -Depth 6 -InputObject @{ processes = $result.ToArray() }`

function snapshotProcessTree (powerShell, rootPid, deadline) {
    const remaining = deadline - Date.now()
    if (remaining <= 0) return { processes: [], error: 'process snapshot deadline exhausted' }
    const output = runPowerShell(powerShell, SNAPSHOT_SCRIPT.replace('__ROOT__', String(rootPid)), Math.min(10000, remaining))
    if (!output.ok) {
        return { processes: [], error: output.error || output.stderr || `process snapshot exited ${output.status}` }
    }
    try {
        const parsed = JSON.parse(output.stdout)
        const processes = parsed && Array.isArray(parsed.processes) ? parsed.processes : []
        return { processes, error: null }
    } catch (error) {
        return { processes: [], error: `could not parse process snapshot: ${message(error)}` }
    }
}

function cdpListeners (context, port) {
    if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error('invalid CDP identity port')
    const script = `$ErrorActionPreference='Stop'
$entries = @(Get-NetTCPConnection -State Listen -ErrorAction Stop | Where-Object LocalPort -eq ${port} | ForEach-Object {
    $p = Get-CimInstance Win32_Process -Filter "ProcessId = $($_.OwningProcess)" -ErrorAction Stop
    if (-not $p) { throw 'CDP listener process disappeared' }
    @{ pid=[int]$p.ProcessId; creationTimeUTC=$p.CreationDate.ToUniversalTime().ToString('o') }
})
ConvertTo-Json -Compress -InputObject $entries`
    const left = context.deadline - Date.now()
    if (left <= 0) throw new Error('CDP identity deadline expired')
    const query = runPowerShell(context.powerShell, script, Math.min(10000, left))
    if (!query.ok) throw new Error('CDP listener identity query failed: ' + (query.error || query.stderr))
    const entries = JSON.parse(query.stdout)
    if (!Array.isArray(entries)) throw new Error('CDP listener identity is not an array')
    return entries
}

function verifyCdpOwnership (context, info, port) {
    const tree = snapshotProcessTree(context.powerShell, info.pid, context.deadline)
    const root = tree.processes.find(entry => entry.pid === info.pid)
    if (tree.error || !root || root.creationTimeUTC !== info.rootIdentity.creationTimeUTC) throw new Error('owned CDP process tree identity changed')
    const listeners = cdpListeners(context, port)
    if (!listeners.length || !listeners.every(listener => tree.processes.some(process => process.pid === listener.pid && process.creationTimeUTC === listener.creationTimeUTC))) {
        throw new Error('CDP port is not exclusively owned by the launched application process tree')
    }
    info.cdp.ownership = { port, listeners, rootIdentity: root, verified: true }
}

function pickPage (browser) {
    const pages = []
    for (const context of browser.contexts()) pages.push(...context.pages())
    const loaded = pages.filter(page => {
        try {
            return page.url() && page.url() !== 'about:blank'
        } catch {
            return false
        }
    })
    return loaded[0] || pages[0] || null
}

async function connectOverCdp (chromium, endpoint, deadline, deathReport) {
    let lastError = null
    while (Date.now() < deadline) {
        const dead = deathReport()
        if (dead) throw new Error(`application ended before CDP connected: ${dead}`)
        const remaining = Math.max(1000, Math.min(15000, deadline - Date.now()))
        try {
            return await chromium.connectOverCDP(endpoint, { timeout: remaining })
        } catch (error) {
            lastError = error
        }
        await delay(500)
    }
    throw new Error(`CDP connect timed out: ${lastError ? message(lastError) : 'no detail'}`)
}

function runNativeWindowCapture (context, pid, appDir, windowHandle = 0) {
    const result = { ok: false, jsonPath: null, screenshotPath: null, data: null, error: null, invocation: null }
    const jsonPath = path.join(appDir, 'native-window.json')
    const pngPath = path.join(appDir, 'native-window.png')
    if (existsSync(jsonPath) || existsSync(pngPath)) {
        result.error = 'native window output targets already exist'
        return result
    }
    const invocation = spawnSync(context.powerShell, [
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', WINDOW_CAPTURE_SCRIPT,
        '-AppProcessId', String(pid), '-OutputDirectory', appDir, '-WindowHandle', String(windowHandle),
    ], { encoding: 'utf8', windowsHide: true, timeout: Math.max(1, Math.min(context.nativeCaptureTimeoutMs ?? 90000, context.deadline - Date.now())), maxBuffer: 16 * 1024 * 1024 })
    result.invocation = { status: invocation.status, error: invocation.error ? message(invocation.error) : null }
    if (existsSync(jsonPath)) {
        result.jsonPath = path.relative(context.output, jsonPath)
        try {
            result.data = JSON.parse(readFileSync(jsonPath, 'utf8'))
            result.ok = invocation.status === 0 && !invocation.error && Boolean(result.data && result.data.ok) && existsSync(pngPath) && statSync(pngPath).size > 0
            if (!result.ok) result.error = (result.data && result.data.error) || 'native window capture reported failure'
        } catch (error) {
            result.error = `invalid native-window.json: ${message(error)}`
        }
    } else {
        result.error = invocation.error ? message(invocation.error)
            : (invocation.stderr || '').trim() || `capture-windows-app-window.ps1 exited ${invocation.status} without native-window.json`
    }
    if (existsSync(pngPath)) result.screenshotPath = path.relative(context.output, pngPath)
    return result
}

function failureInfo (app, error) {
    return {
        role: app.role,
        launched: false,
        executablePath: app.exe,
        executableSha256: null,
        pid: null,
        browserArguments: app.browserArguments,
        environmentOverrides: app.environmentOverrides ?? {},
        portableDataRoot: null,
        cdp: { ok: false, endpoint: `http://127.0.0.1:${app.port}`, engineVersion: null, error: null },
        page: null,
        dom: null,
        console: [],
        pageErrors: [],
        tauriBridge: null,
        blankPage: null,
        clientScreenshot: null,
        nativeWindow: { ok: false, jsonPath: null, screenshotPath: null, data: null, error: null, invocation: null },
        aliveAfterCapture: null,
        cleanup: null,
        error,
    }
}

async function captureApp (context, app) {
    const appDir = path.join(context.output, app.role)
    mkdirSync(appDir)
    const stdoutStream = createWriteStream(path.join(appDir, 'stdout.log'))
    const stderrStream = createWriteStream(path.join(appDir, 'stderr.log'))
    logStreams.push(stdoutStream, stderrStream)

    const info = {
        role: app.role,
        launched: false,
        executablePath: app.exe,
        executableSha256: null,
        pid: null,
        browserArguments: app.browserArguments,
        environmentOverrides: app.environmentOverrides ?? {},
        portableDataRoot: null,
        cdp: { ok: false, endpoint: `http://127.0.0.1:${app.port}`, engineVersion: null, error: null },
        page: null,
        dom: null,
        console: [],
        pageErrors: [],
        tauriBridge: null,
        blankPage: null,
        clientScreenshot: null,
        nativeWindow: { ok: false, jsonPath: null, screenshotPath: null, data: null, error: null, invocation: null },
        aliveAfterCapture: null,
        cleanup: null,
        error: null,
    }

    try {
        info.executableSha256 = await sha256File(app.exe)
        info.portableDataRoot = ensurePortableRoot(app.exe, context.captionColor)
        if (cdpListeners(context, app.port).length) throw new Error('requested CDP port is already occupied; application was not launched')
    } catch (error) {
        info.error = `preparation failed: ${message(error)}`
        return info
    }

    context.result[app.role] = info
    checkpoint(context, `${app.role}-preparing`)
    const appDeadline = Math.min(Date.now() + APP_TIMEOUT_MS, context.deadline)

    let child
    try {
        child = spawn(app.exe, app.args, {
            cwd: path.dirname(app.exe),
            env: app.env,
            shell: false,
            windowsHide: false,
            stdio: ['ignore', 'pipe', 'pipe'],
        })
    } catch (error) {
        info.error = `launch failed: ${message(error)}`
        return info
    }

    info.launched = true
    info.pid = typeof child.pid === 'number' ? child.pid : null

    let exitReport = null
    let spawnError = null
    child.on('exit', (code, signal) => { exitReport = { code, signal } })
    child.on('error', error => { spawnError = message(error) })
    child.stdout.on('data', chunk => stdoutStream.write(chunk))
    child.stderr.on('data', chunk => stderrStream.write(chunk))
    context.running.set(app.role, { child, info, state: () => ({ exitReport, spawnError }) })
    const initialTree = snapshotProcessTree(context.powerShell, info.pid, context.deadline)
    info.initialProcessTree = initialTree
    const root = initialTree.processes.find(entry => entry.pid === info.pid && entry.parentPid === process.pid && entry.executableBasename.toLowerCase() === path.basename(app.exe).toLowerCase())
    if (!root) { info.error = 'could not establish owned root process identity'; return info }
    info.rootIdentity = root
    checkpoint(context, `${app.role}-started`)

    let browser = null
    try {
        browser = await connectOverCdp(context.chromium, info.cdp.endpoint, appDeadline, () => {
            if (spawnError) return `spawn error: ${spawnError}`
            if (exitReport) return `exit code ${exitReport.code} signal ${exitReport.signal}`
            return null
        })
        context.running.get(app.role).browser = browser
        verifyCdpOwnership(context, info, app.port)
        info.cdp.ok = true
        try {
            info.cdp.engineVersion = browser.version()
        } catch (error) {
            info.cdp.engineVersion = `unavailable: ${message(error)}`
        }

        const observedPages = new WeakSet()
        const observePage = candidate => {
            if (!candidate || observedPages.has(candidate)) return
            observedPages.add(candidate)
            candidate.on('console', entry => pushBounded(info.console, { type: entry.type(), text: bounded(entry.text(), 2000) }, 200))
            candidate.on('pageerror', error => pushBounded(info.pageErrors, bounded(message(error), 4000), 100))
        }
        let page = null
        while (Date.now() < appDeadline) {
            page = pickPage(browser)
            observePage(page)
            if (page && await withTimeout(page.evaluate(() => document.readyState !== 'loading' && Boolean(document.body?.innerText.trim()) && Boolean(document.querySelector('welcome-page, start-page, .xterm'))), Math.max(1, Math.min(3000, appDeadline - Date.now())), 'meaningful UI readiness').catch(() => false)) break
            if (spawnError || exitReport) throw new Error('application ended before meaningful UI rendered')
            await delay(250)
        }
        if (!page || Date.now() >= appDeadline) throw new Error('meaningful application UI did not render within the startup deadline')

        context.running.get(app.role).page = page
        page.setDefaultTimeout(Math.max(1000, appDeadline - Date.now()))

        const dom = await withTimeout(page.evaluate(() => ({
            url: location.href,
            title: document.title,
            innerWidth: window.innerWidth,
            innerHeight: window.innerHeight,
            outerWidth: window.outerWidth,
            outerHeight: window.outerHeight,
            visibleText: document.body ? document.body.innerText : '',
            html: document.documentElement ? document.documentElement.outerHTML : '',
            tauriBridge: typeof globalThis.__TAURI_INTERNALS__ !== 'undefined',
        })), Math.max(1000, appDeadline - Date.now()), 'DOM read')

        info.page = {
            url: dom.url,
            title: dom.title,
            innerWidth: dom.innerWidth,
            innerHeight: dom.innerHeight,
            outerWidth: dom.outerWidth,
            outerHeight: dom.outerHeight,
        }
        const expectedUrl = app.role === 'reference' ? /^file:/i.test(dom.url) : /^(?:https?:\/\/tauri\.localhost(?:[/:]|$)|tauri:\/\/localhost(?:[/:]|$))/i.test(dom.url)
        if (!expectedUrl) throw new Error(`unexpected actual application page URL: ${dom.url}`)
        info.tauriBridge = Boolean(dom.tauriBridge)
        info.blankPage = String(dom.visibleText || '').trim().length === 0
        info.dom = {
            visibleText: bounded(dom.visibleText, MAX_VISIBLE_TEXT),
            html: bounded(dom.html, MAX_DOM_HTML),
            visibleTextTruncated: String(dom.visibleText || '').length > MAX_VISIBLE_TEXT,
            htmlTruncated: String(dom.html || '').length > MAX_DOM_HTML,
        }

        const clientPng = path.join(appDir, 'client.png')
        await withTimeout(page.screenshot({ path: clientPng, type: 'png' }), Math.max(1000, appDeadline - Date.now()), 'client screenshot')
        info.clientScreenshot = path.relative(context.output, clientPng)
    } catch (error) {
        info.cdp.error = message(error)
    }

    checkpoint(context, `${app.role}-native-window`)
    if (info.pid) info.nativeWindow = runNativeWindowCapture(context, info.pid, appDir)

    info.aliveAfterCapture = exitReport === null && spawnError === null
    return info
}

async function teardownApp (context, info) {
    const cleanup = { role: info.role, forcedTeardown: true, quitFlowTested: false, rootPid: info.pid, tracked: [], survivors: [], unverifiedDescendants: [], error: null, observation: 'owned process snapshots; this does not verify the application Quit flow or descendants that escape between snapshots' }
    if (!info.rootIdentity) {
        cleanup.error = 'no verified owned root identity; automatic termination refused'
        return cleanup
    }
    const fresh = snapshotProcessTree(context.powerShell, info.pid, context.cleanupDeadline)
    const currentRoot = fresh.processes.find(entry => entry.pid === info.pid)
    if (currentRoot && currentRoot.creationTimeUTC !== info.rootIdentity.creationTimeUTC) {
        cleanup.error = 'root PID was reused; automatic termination refused'
        return cleanup
    }
    const tracked = new Map()
    for (const entry of [...info.initialProcessTree.processes, ...(info.finalProcessObservation?.processes ?? []), ...fresh.processes]) tracked.set(`${entry.pid}:${entry.creationTimeUTC}`, entry)
    cleanup.tracked = [...tracked.values()]
    if (fresh.error) cleanup.error = fresh.error
    const encoded = Buffer.from(JSON.stringify(cleanup.tracked), 'utf8').toString('base64')
    const script = `$ErrorActionPreference = 'Stop'
$entries = @([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('${encoded}')) | ConvertFrom-Json)
$stopped = @(); $survivors = @(); $reused = @(); $exited = @(); $errors = @()
foreach ($entry in $entries) {
    $current = Get-CimInstance Win32_Process -Filter "ProcessId = $($entry.pid)"
    if (-not $current) { continue }
    if ($current.CreationDate.ToUniversalTime().Ticks -ne ([DateTime]$entry.creationTimeUTC).ToUniversalTime().Ticks) { $reused += $entry.pid; continue }
    try {
        $owned = Get-Process -Id $entry.pid -ErrorAction Stop
        if ([Math]::Abs(($owned.StartTime.ToUniversalTime() - $current.CreationDate.ToUniversalTime()).TotalMilliseconds) -gt 1) { throw 'process identity changed before termination' }
        $owned.Kill(); $owned.WaitForExit(5000) | Out-Null; $stopped += $entry.pid
    } catch {
        $terminationError = $_.Exception.Message
        $after = Get-CimInstance Win32_Process -Filter "ProcessId = $($entry.pid)" -ErrorAction Stop
        if (-not $after) { $exited += $entry.pid }
        elseif ($after.CreationDate.ToUniversalTime().Ticks -ne ([DateTime]$entry.creationTimeUTC).ToUniversalTime().Ticks) { $reused += $entry.pid }
        else { $errors += "PID $($entry.pid): $terminationError" }
    }
}
foreach ($entry in $entries) {
    $current = Get-CimInstance Win32_Process -Filter "ProcessId = $($entry.pid)"
    if ($current -and $current.CreationDate.ToUniversalTime().Ticks -eq ([DateTime]$entry.creationTimeUTC).ToUniversalTime().Ticks) { $survivors += $entry.pid }
}
@{ stopped = @($stopped); survivors = @($survivors); reused = @($reused); exited = @($exited); errors = @($errors) } | ConvertTo-Json -Compress -Depth 4`
    const output = runPowerShell(context.powerShell, script, Math.max(1000, Math.min(30000, context.cleanupDeadline - Date.now())))
    try {
        if (!output.ok) throw new Error(output.error || output.stderr || `process termination exited ${output.status}`)
        const report = JSON.parse(output.stdout)
        cleanup.termination = report
        cleanup.survivors = report.survivors
        cleanup.unverifiedDescendants = report.survivors
        if (report.survivors.length || report.errors.length) cleanup.error = JSON.stringify(report)
    } catch (error) { cleanup.error = message(error) }
    return cleanup
}

function cleanupComplete (cleanup) {
    if (!cleanup) return false
    return cleanup.survivors.length === 0 && !cleanup.error
}

function computeOutcome (result) {
    const reference = result.reference || {}
    const native = result.native || {}
    result.substitutionDetected = reference.tauriBridge === true
    result.checks = {
        referenceLaunched: Boolean(reference.launched),
        nativeLaunched: Boolean(native.launched),
        referenceCdp: Boolean(reference.cdp && reference.cdp.ok),
        nativeCdp: Boolean(native.cdp && native.cdp.ok),
        referenceTauriBridgeAbsent: reference.tauriBridge === false,
        nativeTauriBridgePresent: native.tauriBridge === true,
        referencePageRendered: reference.blankPage === false,
        nativePageRendered: native.blankPage === false,
        referenceAliveThroughCapture: reference.aliveAfterCapture === true,
        nativeAliveThroughCapture: native.aliveAfterCapture === true,
        bothAliveBeforeTeardown: result.bothAliveBeforeTeardown === true,
        referenceObservedPageErrorsAbsent: reference.pageErrors?.length === 0,
        nativeObservedPageErrorsAbsent: native.pageErrors?.length === 0,
        referenceClientScreenshot: Boolean(reference.clientScreenshot),
        nativeClientScreenshot: Boolean(native.clientScreenshot),
        referenceNativeWindow: Boolean(reference.nativeWindow && reference.nativeWindow.ok),
        nativeNativeWindow: Boolean(native.nativeWindow && native.nativeWindow.ok),
        referenceCleanupComplete: cleanupComplete(reference.cleanup),
        nativeCleanupComplete: cleanupComplete(native.cleanup),
    }
    result.captured = Boolean(reference.clientScreenshot || (reference.nativeWindow && reference.nativeWindow.screenshotPath))
        && Boolean(native.clientScreenshot || (native.nativeWindow && native.nativeWindow.screenshotPath))
    if (result.flow === 'terminal') {
        result.checks.referenceTerminalInteraction = reference.terminalInteraction?.ok === true
        result.checks.nativeTerminalInteraction = native.terminalInteraction?.ok === true
    }
    if (result.flow === 'frame') {
        result.checks.referenceFrameInteraction = reference.frameInteraction?.ok === true
        result.checks.nativeFrameInteraction = native.frameInteraction?.ok === true
    }
    result.ok = Object.values(result.checks).every(Boolean)
    return result
}

function writeResult (output, result) {
    const target = path.join(output, 'result.json')
    writeFileSync(target, `${JSON.stringify(result, null, 2)}\n`)
    return target
}

function printSummary (result) {
    const failed = Object.entries(result.checks).filter(([, value]) => !value).map(([key]) => key)
    process.stdout.write(`kind: ${result.kind}\n`)
    process.stdout.write(`sourceRevision: ${result.sourceRevision}\n`)
    process.stdout.write(`captured (diagnostics only): ${result.captured}\n`)
    process.stdout.write(`startup capture ok: ${result.ok}\n`)
    process.stdout.write(`fullParityPassed: ${result.fullParityPassed}\n`)
    process.stdout.write(`manualVisualReview: ${result.manualVisualReview}\n`)
    if (failed.length) process.stdout.write(`failed checks: ${failed.join(', ')}\n`)
    for (const error of result.errors) process.stdout.write(`error: ${error}\n`)
}

function checkpoint (context, stage) {
    context.result.stage = stage
    writeResult(context.output, context.result)
}

function releaseControllerHandles (context) {
    for (const { child } of context.running.values()) {
        child.stdout?.destroy()
        child.stderr?.destroy()
        child.unref()
    }
}

async function main () {
    const argv = process.argv.slice(2)
    if (argv.length === 0 || argv.includes('--help') || argv.includes('-h')) {
        process.stdout.write(`${usage()}\n`)
        return 0
    }

    const { values, errors: parseErrors } = parseCliArgs(argv)
    const { errors: validationErrors, referencePort, rsPort, flow } = validateOptions(values)
    const errors = [...parseErrors, ...validationErrors]
    if (errors.length) {
        for (const error of errors) process.stderr.write(`error: ${error}\n`)
        process.stderr.write(`\n${usage()}\n`)
        return 2
    }

    const output = values.output
    const result = {
        kind: KIND,
        flow,
        stage: 'startup-capture',
        sourceRevision: values.sourceRevision,
        platform: process.platform,
        referenceExe: values.referenceExe,
        rsExe: values.rsExe,
        referencePort,
        rsPort,
        startedAt: new Date().toISOString(),
        finishedAt: null,
        durationMs: null,
        captured: false,
        ok: false,
        fullParityPassed: false,
        manualVisualReview: false,
        parityStatus: 'not-evaluated',
        substitutionDetected: false,
        checks: {},
        reference: null,
        native: null,
        cleanup: { forcedTeardown: false, survivors: [], unverifiedDescendants: [] },
        errors: [],
        unverified: [
            'local terminal input through the real PTY was not exercised',
            'second-invocation paste with a literal pipe or Unicode was not exercised',
            'actual UI close and child cleanup were not exercised (teardown is forced process termination)',
            'installer, per-user, and other supported-platform scopes were not exercised',
            'manual visual review of the captured screenshots was not performed',
            'console and page errors emitted before listeners attach after the CDP connection are not replayed',
            'delivery of the diagnostic remote-debugging browser arguments is not independently verified',
        ],
    }

    try {
        mkdirSync(output)
    } catch (error) {
        process.stderr.write(`error: --output could not be created fresh: ${message(error)}\n`)
        return 2
    }

    let chromium
    try {
        ;({ chromium } = await import('playwright-core'))
    } catch (error) {
        result.errors.push(`could not import playwright-core: ${message(error)}`)
        result.finishedAt = new Date().toISOString()
        writeResult(output, result)
        process.stderr.write(`error: could not import playwright-core: ${message(error)}\n`)
        return 1
    }

    const powerShell = resolvePowerShell()
    const cleanupDeadline = Date.parse(result.startedAt) + (flow !== 'startup' ? TERMINAL_TIMEOUT_MS : TOTAL_TIMEOUT_MS)
    const context = { output, result, deadline: cleanupDeadline - 45000, cleanupDeadline, powerShell, chromium, captionColor: flow === 'frame', running: new Map() }
    checkpoint(context, 'starting')
    const watchdog = setTimeout(() => {
        result.ok = false
        result.errors.push('controller hard deadline exceeded; app cleanup is unverified')
        result.cleanup.hardDeadlineExceeded = true
        result.finishedAt = new Date().toISOString()
        try { checkpoint(context, 'deadline-exceeded') } catch (error) { process.stderr.write(`checkpoint failed: ${message(error)}\n`) }
        releaseControllerHandles(context)
        process.exit(1)
    }, Math.max(1, cleanupDeadline - Date.now()))
    watchdog.unref()

    const cleanEnvironment = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(?:TABBY_CONFIG_DIRECTORY|TABBY_RS_(?:BENCHMARK|INSTALLER_SMOKE)_.*|WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS|WEBVIEW2_USER_DATA_FOLDER)$/.test(key)))
    const referenceArgs = [`--remote-debugging-port=${referencePort}`]
    const apps = [
        { role: 'reference', exe: values.referenceExe, port: referencePort, args: referenceArgs, env: { ...cleanEnvironment, PORTABLE_EXECUTABLE_FILE: values.referenceExe }, environmentOverrides: { PORTABLE_EXECUTABLE_FILE: values.referenceExe }, browserArguments: referenceArgs },
        { role: 'native', exe: values.rsExe, port: rsPort, args: [], env: { ...cleanEnvironment, WEBVIEW2_USER_DATA_FOLDER: path.join(path.dirname(values.rsExe), 'data', 'webview'), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required --remote-debugging-port=' + rsPort }, environmentOverrides: { WEBVIEW2_USER_DATA_FOLDER: path.join(path.dirname(values.rsExe), 'data', 'webview'), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required --remote-debugging-port=' + rsPort }, browserArguments: 'unchanged compiled product; process-wide diagnostic WebView2 arguments and an explicit isolated portable WebView profile' },
    ]
    try {
        for (const app of apps) {
            const info = await captureApp(context, app)
            result[app.role] = info
            if (info.error) result.errors.push(`${app.role}: ${info.error}`)
            if (info.cdp.error) result.errors.push(`${app.role} CDP: ${info.cdp.error}`)
            if (info.nativeWindow.error) result.errors.push(`${app.role} native window: ${info.nativeWindow.error}`)
            if (flow === 'frame') {
                const { exerciseWindowsFrame } = await import('./exercise-windows-frame.mjs')
                const run = context.running.get(app.role)
                const info = result[app.role]
                if (!run?.page || !info.rootIdentity || !info.nativeWindow?.ok || info.cdp.error || info.error) {
                    info.frameInteraction = { ok: false, error: 'actual startup did not establish an owned HWND, usable page and process identity' }
                    continue
                }
                const directory = path.join(output, app.role, 'frame-interaction')
                mkdirSync(directory)
                checkpoint(context, app.role + '-frame-interaction')
                verifyCdpOwnership(context, info, app.port)
                info.frameInteraction = await exerciseWindowsFrame({
                    page: run.page, role: app.role, ownerPid: info.pid,
                    snapshotProcessTree: () => snapshotProcessTree(powerShell, info.pid, context.deadline),
                    ownerCreationUtc: info.rootIdentity.creationTimeUTC,
                    ownerHwnd: Number.parseInt(String(info.nativeWindow.data.hwnd), 16),
                    outputDirectory: directory, deadline: context.deadline, powerShell,
                    captureNativeWindow: captureDirectory => runNativeWindowCapture(
                        { ...context, nativeCaptureTimeoutMs: 30000 }, info.pid, captureDirectory,
                        Number.parseInt(String(info.nativeWindow.data.hwnd), 16)),
                })
                if (!info.frameInteraction.ok) result.errors.push(app.role + ' frame interaction: ' + info.frameInteraction.error)
                checkpoint(context, app.role + '-frame-interaction-complete')
                if (info.frameInteraction.inputCleanupFailed) {
                    result.errors.push('Stopped all GUI actions because own input cleanup is unverified')
                    break
                }
            }
        }
        if (flow === 'terminal') {
            const { exerciseWindowsTerminal } = await import('./exercise-windows-terminal.mjs')
            for (const app of apps) {
                const run = context.running.get(app.role)
                const info = result[app.role]
                if (!run?.page || !info.rootIdentity || info.cdp.error || info.error) {
                    info.terminalInteraction = { ok: false, error: 'actual startup did not establish a usable page and process identity' }
                    continue
                }
                const directory = path.join(output, app.role, 'interaction')
                mkdirSync(directory)
                checkpoint(context, `${app.role}-terminal-interaction`)
                info.terminalInteraction = await exerciseWindowsTerminal({
                    page: run.page, role: app.role, executable: app.exe, environment: app.env,
                    ownerPid: info.pid, ownerCreationUtc: info.rootIdentity.creationTimeUTC,
                    outputDirectory: directory, deadline: context.deadline, powerShell,
                    captureNativeWindow: ({ pid, windowHandle, directory: captureDirectory }) => {
                        const capture = runNativeWindowCapture({ ...context, nativeCaptureTimeoutMs: 30000 }, pid, captureDirectory, windowHandle)
                        return { ...capture, screenshotPath: capture.data?.screenshot?.path ?? capture.screenshotPath }
                    },
                })
                if (!info.terminalInteraction.ok) result.errors.push(`${app.role} terminal interaction: ${info.terminalInteraction.error}`)
                checkpoint(context, `${app.role}-terminal-interaction-complete`)
            }
            result.unverified = result.unverified.filter(item => !item.startsWith('local terminal input') && !item.startsWith('second-invocation paste'))
            result.unverified.push('terminal interaction acceptance requires reviewing the actual per-app results and screenshots')
        }
        if (flow === 'frame') {
            result.unverified.push('Full/Native frame restart, secondary windows, cross-DPI and geometry persistence were not exercised')
        }
        result.bothAliveBeforeTeardown = apps.every(app => {
            const run = context.running.get(app.role)
            if (!run?.info.rootIdentity) return false
            const observation = snapshotProcessTree(context.powerShell, run.info.pid, context.deadline)
            run.info.finalProcessObservation = observation
            const root = observation.processes.find(entry => entry.pid === run.info.pid)
            return !observation.error && root?.creationTimeUTC === run.info.rootIdentity.creationTimeUTC && !run.state().exitReport && !run.state().spawnError
        })
    } finally {
        for (const run of context.running.values()) {
            const info = run.info
            result[info.role] ??= info
            checkpoint(context, `${info.role}-teardown`)
            if (run.browser) await withTimeout(run.browser.close(), Math.max(1, Math.min(5000, context.cleanupDeadline - Date.now())), 'CDP disconnect').catch(error => { info.cdp.disconnectError = message(error) })
            info.cleanup = await teardownApp(context, info).catch(error => ({ survivors: [], error: message(error) }))
            if (info.cleanup.error) result.errors.push(`${info.role} cleanup: ${info.cleanup.error}`)
            checkpoint(context, `${info.role}-teardown-complete`)
        }
        releaseControllerHandles(context)
    }

    result.cleanup.forcedTeardown = apps.every(app => result[app.role] && result[app.role].cleanup && result[app.role].cleanup.forcedTeardown)
    result.cleanup.survivors = apps.flatMap(app => (result[app.role] && result[app.role].cleanup && result[app.role].cleanup.survivors) || [])
    result.cleanup.unverifiedDescendants = result.cleanup.survivors

    computeOutcome(result)
    result.finishedAt = new Date().toISOString()
    result.durationMs = Date.parse(result.finishedAt) - Date.parse(result.startedAt)

    checkpoint(context, 'complete')
    clearTimeout(watchdog)
    printSummary(result)
    return result.ok ? 0 : 1
}

let exitCode = 1
try {
    exitCode = await main()
} catch (error) {
    process.stderr.write(`error: ${message(error)}\n`)
    exitCode = 1
} finally {
    await withTimeout(Promise.all(logStreams.map(stream => new Promise(resolve => {
        if (stream.writableFinished || stream.destroyed) { resolve(); return }
        stream.once('finish', resolve)
        stream.once('error', resolve)
        stream.end()
    }))), 2000, 'log flush').catch(() => {})
}
process.exit(exitCode)
