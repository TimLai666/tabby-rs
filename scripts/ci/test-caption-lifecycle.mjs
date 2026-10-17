import assert from 'node:assert/strict'
import test from 'node:test'
import { exerciseWindowsCaptionLifecycle } from './exercise-windows-caption-lifecycle.mjs'

const primary = Object.freeze({ pid: 4101, creationUtc: '2026-10-17T00:00:00.000Z', hwnd: 501 })
const secondary = Object.freeze({ pid: primary.pid, creationUtc: primary.creationUtc, hwnd: 502 })
const bounds = Object.freeze({ x: 12, y: 24, width: 900, height: 640 })
const state = (extra = {}) => ({ bounds: { ...bounds }, minimized: false, maximized: false, ...extra })

function harness(overrides = {}) {
  const events = []
  const invokes = new Map()
  const terminalFocus = []
  const nativeCalls = []
  const nativeAction = async ({ identity, action }) => {
    events.push(`native:${action}:${identity.hwnd}`)
    nativeCalls.push({ identity: { ...identity }, action })
    const evidence = { ok: true, identity: { ...identity }, action, before: state(), after: state() }
    if (action.startsWith('Invoke')) invokes.set(`${identity.hwnd}:${action}`, (invokes.get(`${identity.hwnd}:${action}`) ?? 0) + 1)
    switch (action) {
      case 'PressOutside': return { ...evidence, physicalMouseDown: true, releasedOutside: true, captureCleared: true, mouseButtonUp: true }
      case 'PressCancel': return { ...evidence, physicalMouseDown: true, cancelled: true, captureCleared: true, mouseButtonUp: true }
      case 'InvokeMaximize': return { ...evidence, invocationCount: 1, after: state({ maximized: true }) }
      case 'InvokeRestore': return { ...evidence, invocationCount: 1, after: state() }
      case 'InvokeMinimize': return { ...evidence, invocationCount: 1, after: state({ minimized: true }) }
      case 'RestoreWindow': return { ...evidence, after: state() }
      case 'StartBusyTerminal': return { ...evidence, busyChildReady: true }
      case 'InvokeClose': return { ...evidence, invocationCount: 1, closeWarningPending: true, forcedDestroy: false }
      case 'CancelClose': return { ...evidence, cancelled: true, windowRetained: true }
      case 'StopBusyTerminal': return { ...evidence, stopped: true, sessionsClosed: true }
      case 'CloseSecondary': return { ...evidence, windowDestroyed: true, sessionsClosed: true }
      default: return evidence
    }
  }
  const verifyTerminal = async ({ identity, focus, name }) => {
    events.push(`pty:${name}:${identity.hwnd}:${focus}`)
    terminalFocus.push({ identity: { ...identity }, focus, name })
    return { exactBytes: true, renderedMarkerCopied: true }
  }
  const createSecondary = async () => {
    events.push('create-secondary')
    return { ...secondary }
  }
  const capture = async ({ identity, name }) => {
    events.push(`capture:${name}:${identity.hwnd}`)
    return { ok: true }
  }
  return {
    events, invokes, terminalFocus, nativeCalls, nativeAction,
    options: {
      primary: { ...primary },
      nativeAction: overrides.nativeAction ?? nativeAction,
      verifyTerminal: overrides.verifyTerminal ?? verifyTerminal,
      createSecondary: overrides.createSecondary ?? createSecondary,
      capture: overrides.capture ?? capture,
      deadline: overrides.deadline ?? Date.now() + 10_000,
    },
  }
}

async function execute(options) {
  let timer
  try {
    return await Promise.race([
      exerciseWindowsCaptionLifecycle(options),
      new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('test harness timeout')), 1500) }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

async function expectNotOk(options, reason) {
  const report = await execute(options)
  assert.equal(report?.ok, false, `${reason} must not return ok`)
  assert.ok(typeof report.error === 'string' && report.error.length > 0, `${reason} must be reported`)
  return report
}

test('positive guard executes complete 12-step lifecycle and single UIA Invokes in order', async () => {
  const h = harness()
  const report = await execute(h.options)
  assert.equal(report.ok, true)
  assert.equal(report.inputCleanupFailed, false)
  assert.deepEqual(report.primary, primary)
  assert.deepEqual(report.secondary, secondary)
  assert.ok(Array.isArray(report.actions) && report.actions.length > 0, 'successful report must retain action evidence')
  assert.deepEqual(h.events, [
    'native:Snapshot:501', 'pty:initial:501:true',
    'native:PressOutside:501', 'native:PressCancel:501',
    'native:InvokeMaximize:501', 'native:InvokeRestore:501',
    'native:InvokeMinimize:501', 'native:RestoreWindow:501',
    'pty:caption-focus-check:501:false', 'native:StartBusyTerminal:501',
    'native:InvokeClose:501', 'native:CancelClose:501', 'native:StopBusyTerminal:501',
    'pty:after-close-cancel:501:false', 'create-secondary',
    'native:Snapshot:501', 'native:Snapshot:502',
    'native:InvokeMaximize:502', 'native:InvokeRestore:502', 'native:Snapshot:501',
    'pty:secondary:502:true', 'native:CloseSecondary:502',
    'pty:primary-restored:501:true', 'capture:primary-after:501',
  ])
  assert.deepEqual([...h.invokes.entries()].sort(), [
    '501:InvokeClose', '501:InvokeMaximize', '501:InvokeMinimize', '501:InvokeRestore',
    '502:InvokeMaximize', '502:InvokeRestore',
  ].map((key) => [key, 1]).sort())
  assert.deepEqual(h.terminalFocus.map(({ focus }) => focus), [true, false, false, true, true])
  assert.equal(h.nativeCalls.some(({ action }) => action === 'RefocusTerminal'), false)
})

test('wrong primary snapshot identity stops before PTY or later GUI actions', async () => {
  const h = harness()
  h.options.nativeAction = async (input) => ({ ...await h.nativeAction(input), identity: { ...primary, hwnd: 999 } })
  const report = await expectNotOk(h.options, 'wrong primary identity')
  assert.equal(report.primary?.hwnd, primary.hwnd)
  assert.deepEqual(h.events, ['native:Snapshot:501'])
})

test('unconfirmed input cleanup stops all subsequent GUI actions', async () => {
  const h = harness()
  h.options.nativeAction = async (input) => {
    const result = await h.nativeAction(input)
    if (input.action === 'PressOutside') return { ...result, mouseButtonUp: false, captureCleared: false }
    return result
  }
  const report = await expectNotOk(h.options, 'unconfirmed input cleanup')
  assert.equal(report.inputCleanupFailed, true)
  assert.deepEqual(h.events, ['native:Snapshot:501', 'pty:initial:501:true', 'native:PressOutside:501'])
})

test('incorrect secondary process identity is rejected before native work on that window', async () => {
  const h = harness({ createSecondary: async () => { h.events.push('create-secondary'); return { ...secondary, pid: secondary.pid + 1 } } })
  await expectNotOk(h.options, 'wrong secondary identity')
  assert.equal(h.nativeCalls.some(({ identity }) => identity.hwnd === secondary.hwnd), false)
  assert.equal(h.events.some((event) => event.startsWith('native:Snapshot:502')), false)
})

test('PTY mismatch cannot produce ok and stops before caption input', async () => {
  const h = harness({ verifyTerminal: async ({ identity, focus, name }) => {
    h.events.push(`pty:${name}:${identity.hwnd}:${focus}`)
    return { exactBytes: name !== 'initial', renderedMarkerCopied: true }
  } })
  await expectNotOk(h.options, 'PTY verification failure')
  assert.deepEqual(h.events, ['native:Snapshot:501', 'pty:initial:501:true'])
})

test('ambiguous UIA Invoke is not retried or followed by another caption action', async () => {
  const h = harness()
  h.options.nativeAction = async (input) => {
    const result = await h.nativeAction(input)
    if (input.action === 'InvokeMaximize') return { ...result, invocationCount: 2 }
    return result
  }
  await expectNotOk(h.options, 'duplicate UIA invocation')
  assert.equal(h.events.filter((event) => event === 'native:InvokeMaximize:501').length, 1)
  assert.equal(h.events.includes('native:InvokeRestore:501'), false)
})

test('busy-terminal close cleanup failure stops before secondary creation', async () => {
  const h = harness()
  h.options.nativeAction = async (input) => {
    const result = await h.nativeAction(input)
    if (input.action === 'StopBusyTerminal') return { ...result, sessionsClosed: false }
    return result
  }
  await expectNotOk(h.options, 'close cleanup failure')
  assert.equal(h.events.includes('create-secondary'), false)
  assert.equal(h.events.some((event) => event.startsWith('capture:')), false)
})

test('uncancelled close warning cannot continue into stop, secondary, or capture', async () => {
  const h = harness()
  h.options.nativeAction = async (input) => {
    const result = await h.nativeAction(input)
    if (input.action === 'CancelClose') return { ...result, windowRetained: false }
    return result
  }
  await expectNotOk(h.options, 'close warning cancellation failure')
  assert.equal(h.events.includes('native:StopBusyTerminal:501'), false)
  assert.equal(h.events.includes('create-secondary'), false)
  assert.equal(h.events.some((event) => event.startsWith('capture:')), false)
})

test('secondary window actions must leave primary bounds and state unchanged', async () => {
  const h = harness()
  h.options.nativeAction = async (input) => {
    const result = await h.nativeAction(input)
    if (input.action === 'Snapshot' && input.identity.hwnd === primary.hwnd && h.events.filter((event) => event === 'native:Snapshot:501').length > 2) {
      return { ...result, after: state({ maximized: true }) }
    }
    return result
  }
  await expectNotOk(h.options, 'primary changed by secondary action')
  assert.equal(h.events.includes('native:CloseSecondary:502'), false)
})

test('secondary close/session cleanup failure prevents final capture and ok', async () => {
  const h = harness()
  h.options.nativeAction = async (input) => {
    const result = await h.nativeAction(input)
    if (input.action === 'CloseSecondary') return { ...result, sessionsClosed: false }
    return result
  }
  await expectNotOk(h.options, 'secondary close cleanup failure')
  assert.equal(h.events.includes('capture:primary-after:501'), false)
})

test('failed final capture cannot produce ok', async () => {
  const h = harness({ capture: async ({ identity, name }) => {
    h.events.push(`capture:${name}:${identity.hwnd}`)
    return { ok: false, error: 'capture unavailable' }
  } })
  await expectNotOk(h.options, 'capture failure')
  assert.equal(h.events.at(-1), 'capture:primary-after:501')
})

test('expired deadline prevents the first native action', async () => {
  const h = harness({ deadline: Date.now() - 1 })
  await expectNotOk(h.options, 'expired deadline')
  assert.deepEqual(h.events, [])
})

test('deadline expiry during a native action prevents the next GUI action', async (context) => {
  let now = Date.now()
  context.mock.method(Date, 'now', () => now)
  const h = harness({ deadline: now + 100 })
  h.options.nativeAction = async (input) => {
    const result = await h.nativeAction(input)
    if (input.action === 'Snapshot') now += 101
    return result
  }
  await expectNotOk(h.options, 'mid-flow deadline expiry')
  assert.deepEqual(h.events, ['native:Snapshot:501'])
})


test('native boundary rejection during physical input makes cleanup unverified and stops', async () => {
  const h = harness()
  h.options.nativeAction = async (input) => {
    if (input.action === 'PressOutside') {
      h.events.push(`native:${input.action}:${input.identity.hwnd}`)
      throw new Error('native helper timed out after mouse down')
    }
    return h.nativeAction(input)
  }
  const report = await expectNotOk(h.options, 'native helper rejection')
  assert.equal(report.inputCleanupFailed, true)
  assert.equal(h.events.at(-1), 'native:PressOutside:501')
})

for (const [action, invalidAfter] of [
  ['InvokeMaximize', state()],
  ['InvokeRestore', state({ maximized: true })],
  ['InvokeMinimize', state()],
  ['RestoreWindow', state({ minimized: true })],
]) {
  test(`${action} must prove its resulting window state before proceeding`, async () => {
    const h = harness()
    h.options.nativeAction = async (input) => {
      const evidence = await h.nativeAction(input)
      return input.action === action ? { ...evidence, after: invalidAfter } : evidence
    }
    await expectNotOk(h.options, `${action} wrong resulting state`)
    assert.equal(h.events.at(-1), `native:${action}:501`)
  })
}
