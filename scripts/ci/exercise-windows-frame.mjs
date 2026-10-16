// QA-only input acceptance using actual Windows SendInput and the source-built PTY adapter.
import { randomUUID } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { createOwnedCaptionDriver } from './run-windows-caption-desktop.mjs'
const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
const literal = value => `'${String(value).replaceAll("'", "''")}'`

export async function exerciseWindowsFrame (options) {
    const driver = createOwnedCaptionDriver(options)
    const { primary, helper, activate, timeout, tree } = driver
    const { page, outputDirectory, captureNativeWindow } = options
    const result = { kind: 'actual-windows-pty-input', acceptanceScope: 'PTY input only; caption and platform parity are not accepted', ok: false, cases: [], nativeEvidence: driver.evidence }
    const capture = async name => {
        const directory = path.join(outputDirectory, name)
        mkdirSync(directory)
        const evidence = await captureNativeWindow(directory)
        if (!evidence.ok || evidence.data?.appProcessId !== primary.pid || Number.parseInt(String(evidence.data?.hwnd), 16) !== primary.hwnd) throw new Error('native screenshot ownership was not established')
        return evidence
    }
    const burst = async (name, args, interval) => {
        const marker = `PTY_ORDER_${randomUUID()}`
        const stem = path.join(outputDirectory, name)
        const file = stem + '.txt', metaFile = stem + '.json', scriptFile = stem + '.ps1'
        const expected = args.join('|')
        const expectedBase64 = Buffer.from(expected, 'utf8').toString('base64')
        const script = `$ErrorActionPreference='Stop'; $expected=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('${expectedBase64}')); if(($args -join '|') -cne $expected){throw 'input argument order/bytes mismatch'}; $self=Get-CimInstance Win32_Process -Filter "ProcessId = $PID"; $parent=Get-CimInstance Win32_Process -Filter "ProcessId = $($self.ParentProcessId)"; if(-not $self -or -not $parent){throw 'missing owned process'}; @{child=@{pid=[int]$self.ProcessId;creationUtc=$self.CreationDate.ToUniversalTime().ToString('o')};shell=@{pid=[int]$parent.ProcessId;creationUtc=$parent.CreationDate.ToUniversalTime().ToString('o')}}|ConvertTo-Json -Compress|Set-Content -LiteralPath ${literal(metaFile)} -Encoding UTF8; [IO.File]::WriteAllText(${literal(file)},$expected,[Text.UTF8Encoding]::new($false)); [Console]::WriteLine('${marker}')`
        if ([file, metaFile, scriptFile].some(existsSync)) throw new Error('burst evidence must be fresh')
        writeFileSync(scriptFile, script, { flag: 'wx' })
        const command = `powershell.exe -NoLogo -NoProfile -NonInteractive -File "${scriptFile}" ${args.join(' ')}`
        if (command.length > 8192 || args.some(arg => !/^[a-zA-Z0-9\u3400-\u9fff]+$/.test(arg))) throw new Error('unexpected burst command input')
        const row = { name, interval, commandCharacters: command.length, argumentCount: args.length, expectedUtf8Base64: expectedBase64, marker, file, exactBytes: false, renderedMarkerCopied: false }
        result.cases.push(row)
        await activate(primary)
        const before = helper(primary, 'Snapshot')
        if (before.after?.minimized) throw new Error('input window is minimized')
        const typed = helper(primary, 'TypeTerminal', ['-TextBase64', Buffer.from(command, 'utf16le').toString('base64'), '-CharacterIntervalMs', String(interval)])
        if (typed.sentCharacters !== command.length || typed.characterIntervalMs !== interval) throw new Error('actual SendInput character count/delay mismatch')
        const until = Date.now() + timeout(30000)
        while (Date.now() < until && !existsSync(file)) await delay(100)
        if (!existsSync(file) || !readFileSync(file).equals(Buffer.from(expected, 'utf8'))) throw new Error(name + ': exact terminal-created input bytes were not observed')
        row.exactBytes = true
        const meta = JSON.parse(readFileSync(metaFile, 'utf8').replace(/^\uFEFF/, ''))
        const processes = tree()
        const shell = driver.sessions.get(primary.hwnd)
        if (!shell || meta.shell.pid !== shell.pid || meta.shell.creationUtc !== shell.creationUtc || !processes.some(p => p.pid === shell.pid && p.creationTimeUTC === shell.creationUtc)) throw new Error('burst output was not from the original owned terminal shell')
        row.meta = meta
        const untilCopy = Date.now() + timeout(15000)
        while (Date.now() < untilCopy && !row.renderedMarkerCopied) {
            await page.keyboard.press('Control+Shift+A', { timeout: timeout(5000) })
            await page.keyboard.press('Control+Shift+C', { timeout: timeout(5000) })
            const text = driver.readClipboard()
            row.renderedMarkerCopied = text.replace(/[\r\n]/g, '').includes(marker)
            if (!row.renderedMarkerCopied) await delay(100)
        }
        if (!row.renderedMarkerCopied) throw new Error(name + ': rendered output was not read back through Windows clipboard')
        await page.screenshot({ path: stem + '.png', timeout: timeout(10000) })
        row.nativeCapture = await capture(name + '-native')
    }
    try {
        const welcome = page.getByRole('button', { name: 'Close and never show again', exact: true })
        const visible = []
        for (const button of await welcome.all()) if (await button.isVisible()) visible.push(button)
        if (visible.length > 1) throw new Error('ambiguous welcome controls')
        if (visible.length === 1) await visible[0].click({ timeout: timeout(10000) })
        await page.keyboard.press('Control+Shift+T', { timeout: timeout(5000) })
        await page.locator('.xterm-helper-textarea:visible').waitFor({ timeout: timeout(15000) })
        result.shortInput = await driver.commandProbe(primary, 'short-input', true)
        await burst('ascii-burst', Array.from({ length: 200 }, (_, i) => `a${String(i).padStart(4, '0')}`), 0)
        await burst('unicode-burst', Array.from({ length: 24 }, (_, i) => `字中${String(i).padStart(3, '0')}`), 0)
        result.finalInput = await driver.commandProbe(primary, 'after-bursts', true)
        result.ok = true
    } catch (error) {
        result.error = String(error)
        try { await page.screenshot({ path: path.join(outputDirectory, 'failure.png'), timeout: timeout(5000) }) } catch (error) { result.screenshotError = String(error) }
    }
    result.inputCleanupFailed = driver.getInputCleanupFailed()
    if (result.inputCleanupFailed) result.ok = false
    writeFileSync(path.join(outputDirectory, 'pty-input-result.json'), JSON.stringify(result, null, 2), { flag: 'wx' })
    return result
}
