/** QA only. The owned Windows controller supplies all real desktop boundaries. */
export async function exerciseWindowsFrameRestart (options) {
    const initialCommandCount = options?.initialCommandCount === undefined ? 1 : options.initialCommandCount
    const report = { ok: false, identity: options?.identity ?? null, modes: [], error: null, initialCommandCount, commandReplayTested: false }
    const validIdentity = value => value && Number.isSafeInteger(value.pid) && value.pid > 0 &&
        Number.isSafeInteger(value.hwnd) && value.hwnd > 0 && typeof value.creationUtc === 'string' && value.creationUtc.trim().length > 0
    const required = ['selectFrame', 'restart', 'verifyFrame', 'verifyTerminal', 'exerciseFullscreen', 'checkConsumedArguments', 'capture']
    const checkDeadline = () => {
        if (!Number.isFinite(options?.deadline) || Date.now() >= options.deadline) throw new Error('restart acceptance deadline expired')
    }
    const require = (ok, detail) => { if (!ok) throw new Error(detail) }
    try {
        require(initialCommandCount === 0 || initialCommandCount === 1, 'initial command count must be zero or one')
        require(validIdentity(options?.identity), 'invalid initial process/window identity')
        require(required.every(name => typeof options[name] === 'function'), 'all restart boundaries are required')
        checkDeadline()
        let identity = { ...options.identity }
        for (const mode of ['native', 'thin', 'full']) {
            const events = []
            const call = async (boundary, extra = {}) => {
                checkDeadline()
                const value = await options[boundary]({ identity: { ...identity }, mode, ...extra })
                events.push({ boundary, ...extra, value })
                checkDeadline()
                return value
            }
            const selected = await call('selectFrame')
            require(selected?.persisted === true && selected.restartRequested === true, `${mode}: selected frame persistence/restart request failed`)
            const replacement = await call('restart')
            require(validIdentity(replacement?.identity), `${mode}: invalid replacement identity`)
            require(replacement.identity.pid !== identity.pid || replacement.identity.creationUtc !== identity.creationUtc, `${mode}: process was not replaced`)
            for (const flag of ['ownedReplacement', 'oldProcessGone', 'oldWindowsGone', 'oldSessionsClosed']) {
                require(replacement[flag] === true, `${mode}: ${flag} is unverified`)
            }
            identity = { ...replacement.identity }
            report.identity = identity
            const frame = await call('verifyFrame')
            require(frame?.selectedFrame === mode && frame.configReadback === mode && frame.frameApplied === true, `${mode}: persisted/native frame mismatch`)
            const before = await call('verifyTerminal', { phase: 'before-fullscreen' })
            require(before?.exactBytes === true && before.renderedMarkerCopied === true, `${mode}: initial terminal verification failed`)
            const full = await call('exerciseFullscreen')
            for (const flag of ['entered', 'controlsHidden', 'exited', 'controlsRestored']) {
                require(full?.[flag] === true, `${mode}: fullscreen ${flag} failed`)
            }
            const after = await call('verifyTerminal', { phase: 'after-fullscreen' })
            require(after?.exactBytes === true && after.renderedMarkerCopied === true, `${mode}: post-fullscreen terminal verification failed`)
            const args = await call('checkConsumedArguments')
            require(args?.expectedLaunchCount === initialCommandCount && args.observedLaunchCount === initialCommandCount, `${mode}: consumed launch arguments replayed or unverified`)
            const capture = await call('capture')
            require(capture?.ok === true, `${mode}: capture failed`)
            report.modes.push({ mode, identity: { ...identity }, events })
        }
        report.commandReplayTested = initialCommandCount === 1
        report.ok = true
    } catch (error) {
        report.error = error instanceof Error ? error.message : String(error)
    }
    return report
}
