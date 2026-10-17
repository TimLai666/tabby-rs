/** Actual Windows caption lifecycle orchestration. Native/PTY/CDP boundaries are supplied by the owned runner. */
export async function exerciseWindowsCaptionLifecycle(options) {
  const report = { ok: false, actions: [], primary: options?.primary ?? null, secondary: undefined, error: null, inputCleanupFailed: false }
  const fail = (message) => { report.error = message; return report }
  const sameIdentity = (a, b) => Boolean(a && b && a.pid === b.pid && a.creationUtc === b.creationUtc && a.hwnd === b.hwnd)
  const sameState = (a, b) => Boolean(a && b && a.minimized === b.minimized && a.maximized === b.maximized &&
    a.bounds && b.bounds && a.bounds.x === b.bounds.x && a.bounds.y === b.bounds.y && a.bounds.width === b.bounds.width && a.bounds.height === b.bounds.height)
  const alive = () => Number.isFinite(options?.deadline) && Date.now() < options.deadline

  if (!options || !options.primary || typeof options.nativeAction !== 'function' || typeof options.verifyTerminal !== 'function' ||
      typeof options.createSecondary !== 'function' || typeof options.capture !== 'function') return fail('invalid lifecycle options')
  if (!alive()) return fail('deadline expired')

  const native = async (identity, action, extra = {}) => {
    if (!alive()) throw new Error('deadline expired')
    let evidence
    try { evidence = await options.nativeAction({ identity, action, ...extra }) }
    catch (error) {
      if (action === 'PressOutside' || action === 'PressCancel') report.inputCleanupFailed = true
      throw error
    }
    report.actions.push({ action, identity, evidence })
    if ((action === 'PressOutside' || action === 'PressCancel') &&
        (!evidence?.physicalMouseDown || !evidence?.captureCleared || !evidence?.mouseButtonUp)) report.inputCleanupFailed = true
    if (!alive()) throw new Error('deadline expired')
    if (!evidence?.ok || evidence.action !== action || !sameIdentity(evidence.identity, identity)) throw new Error(`${action} failed or returned mismatched identity`)
    return evidence
  }
  const terminal = async (identity, focus, name) => {
    if (!alive()) throw new Error('deadline expired')
    const evidence = await options.verifyTerminal({ identity, focus, name })
    if (!alive()) throw new Error('deadline expired')
    if (!evidence?.exactBytes || !evidence?.renderedMarkerCopied) throw new Error(`${name} terminal verification failed`)
  }
  const expectState = (e, predicate, action) => { if (!predicate(e.after)) throw new Error(`${action} resulting state mismatch`) }
  const isBaseline = (s) => sameState(s, baseline)

  let baseline
  try {
    baseline = (await native(options.primary, 'Snapshot')).after
    if (!baseline || !Number.isFinite(baseline.bounds?.x) || !Number.isFinite(baseline.bounds?.y) ||
        !Number.isFinite(baseline.bounds?.width) || !Number.isFinite(baseline.bounds?.height) || baseline.minimized !== false || baseline.maximized !== false)
      throw new Error('primary baseline snapshot invalid')
    await terminal(options.primary, true, 'initial')

    const outside = await native(options.primary, 'PressOutside')
    if (!outside.physicalMouseDown || !outside.releasedOutside || !outside.captureCleared || !outside.mouseButtonUp) {
      report.inputCleanupFailed = !outside.captureCleared || !outside.mouseButtonUp
      throw new Error('PressOutside input or cleanup evidence incomplete')
    }
    expectState(outside, isBaseline, 'PressOutside')
    const cancel = await native(options.primary, 'PressCancel')
    if (!cancel.physicalMouseDown || !cancel.cancelled || !cancel.captureCleared || !cancel.mouseButtonUp) {
      report.inputCleanupFailed = !cancel.captureCleared || !cancel.mouseButtonUp
      throw new Error('PressCancel input or cleanup evidence incomplete')
    }
    expectState(cancel, isBaseline, 'PressCancel')

    let e = await native(options.primary, 'InvokeMaximize')
    if (e.invocationCount !== 1) throw new Error('InvokeMaximize invocation count was not one')
    expectState(e, (s) => s?.maximized === true && s.minimized === false, 'InvokeMaximize')
    e = await native(options.primary, 'InvokeRestore')
    if (e.invocationCount !== 1) throw new Error('InvokeRestore invocation count was not one')
    expectState(e, isBaseline, 'InvokeRestore')
    e = await native(options.primary, 'InvokeMinimize')
    if (e.invocationCount !== 1) throw new Error('InvokeMinimize invocation count was not one')
    expectState(e, (s) => s?.minimized === true, 'InvokeMinimize')
    e = await native(options.primary, 'RestoreWindow')
    expectState(e, isBaseline, 'RestoreWindow')
    await terminal(options.primary, false, 'caption-focus-check')

    e = await native(options.primary, 'StartBusyTerminal')
    if (!e.busyChildReady) throw new Error('busy terminal child is not ready')
    e = await native(options.primary, 'InvokeClose')
    if (e.invocationCount !== 1 || !e.closeWarningPending || e.forcedDestroy) throw new Error('InvokeClose warning evidence invalid')
    e = await native(options.primary, 'CancelClose')
    if (!e.cancelled || !e.windowRetained) throw new Error('CancelClose did not retain window')
    e = await native(options.primary, 'StopBusyTerminal')
    if (!e.stopped || !e.sessionsClosed) throw new Error('busy terminal cleanup failed')
    await terminal(options.primary, false, 'after-close-cancel')

    const secondary = await options.createSecondary()
    if (!alive()) throw new Error('deadline expired')
    if (!secondary || secondary.hwnd === options.primary.hwnd || secondary.pid !== options.primary.pid || secondary.creationUtc !== options.primary.creationUtc)
      throw new Error('secondary window identity invalid')
    report.secondary = secondary
    const primaryBefore = (await native(options.primary, 'Snapshot')).after
    const secondaryBefore = (await native(secondary, 'Snapshot')).after
    if (!sameState(primaryBefore, baseline) || !secondaryBefore) throw new Error('pre-secondary snapshot invalid')
    e = await native(secondary, 'InvokeMaximize')
    if (e.invocationCount !== 1) throw new Error('secondary InvokeMaximize invocation count was not one')
    expectState(e, (s) => s?.maximized === true && s.minimized === false, 'secondary InvokeMaximize')
    e = await native(secondary, 'InvokeRestore')
    if (e.invocationCount !== 1) throw new Error('secondary InvokeRestore invocation count was not one')
    expectState(e, (s) => sameState(s, secondaryBefore), 'secondary InvokeRestore')
    e = await native(options.primary, 'Snapshot')
    if (!sameState(e.after, primaryBefore)) throw new Error('secondary actions changed primary window')
    await terminal(secondary, true, 'secondary')
    e = await native(secondary, 'CloseSecondary')
    if (!e.windowDestroyed || !e.sessionsClosed) throw new Error('secondary close/session cleanup failed')
    await terminal(options.primary, true, 'primary-restored')
    if (!alive()) throw new Error('deadline expired')
    const capture = await options.capture({ identity: options.primary, name: 'primary-after' })
    if (!alive()) throw new Error('deadline expired')
    if (!capture?.ok) throw new Error(capture?.error || 'final capture failed')
    report.actions.push({ action: 'Capture', identity: options.primary, evidence: capture })
    report.ok = true
    return report
  } catch (error) {
    report.error = error instanceof Error ? error.message : String(error)
    return report
  }
}
