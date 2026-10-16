// QA-only Windows native close/cancel acceptance for source-built PTY shell identity.
import path from 'node:path'
import { runWindowsCaptionDesktop } from './run-windows-caption-desktop.mjs'

export async function exerciseWindowsFrame (options) {
    const { page } = options
    const welcome = page.getByRole('button', { name: 'Close and never show again', exact: true })
    const visible = []
    for (const button of await welcome.all()) if (await button.isVisible()) visible.push(button)
    if (visible.length > 1) throw new Error('ambiguous welcome controls')
    if (visible.length === 1) await visible[0].click({ timeout: 10000 })
    await page.keyboard.press('Control+Shift+T', { timeout: 5000 })
    await page.locator('.xterm-helper-textarea:visible').waitFor({ timeout: 15000 })
    const result = await runWindowsCaptionDesktop({ ...options, outputDirectory: path.join(options.outputDirectory, 'shell-close') })
    result.acceptanceScope = 'Native-frame close warnings, cancellation, working PTY and secondary cleanup only; caption overlay, restart, installers and platform parity are pending'
    return result
}
