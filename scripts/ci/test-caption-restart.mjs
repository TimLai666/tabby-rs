import test from 'node:test';
import assert from 'node:assert/strict';
import { exerciseWindowsFrameRestart } from './exercise-windows-frame-restart.mjs';

const initial = Object.freeze({ pid: 4100, creationUtc: '2026-10-17T01:02:03.000Z', hwnd: 9001 });
const modes = ['native', 'thin', 'full'];
const boundaryNames = [
    'selectFrame', 'restart', 'verifyFrame', 'verifyTerminal:before-fullscreen',
    'exerciseFullscreen', 'verifyTerminal:after-fullscreen', 'checkConsumedArguments', 'capture'
];

function callbacks (overrides = {}, calls = []) {
    const defaults = {
        selectFrame: async () => ({ persisted: true, restartRequested: true }),
        restart: async ({ identity }) => ({
            identity: { pid: identity.pid + 1, creationUtc: '2026-10-17T01:02:04.000Z', hwnd: identity.hwnd },
            ownedReplacement: true, oldProcessGone: true, oldWindowsGone: true, oldSessionsClosed: true
        }),
        verifyFrame: async ({ mode }) => ({ selectedFrame: mode, configReadback: mode, frameApplied: true }),
        verifyTerminal: async () => ({ exactBytes: true, renderedMarkerCopied: true }),
        exerciseFullscreen: async () => ({ entered: true, controlsHidden: true, exited: true, controlsRestored: true }),
        checkConsumedArguments: async () => ({ expectedLaunchCount: 1, observedLaunchCount: 1 }),
        capture: async () => ({ ok: true })
    };
    const result = {};
    for (const name of Object.keys(defaults)) {
        result[name] = overrides[name] ?? (async args => {
            calls.push({ name, args });
            return defaults[name](args);
        });
        if (overrides[name]) {
            const original = overrides[name];
            result[name] = async args => { calls.push({ name, args }); return original(args); };
        }
    }
    return result;
}

function options (extra = {}) {
    return { identity: { ...initial }, deadline: Date.now() + 60_000, ...callbacks(), ...extra };
}

function assertStopped (result) {
    assert.equal(result.ok, false);
    assert.ok(typeof result.error === 'string' && result.error.length > 0);
}

test('runs all frame modes in order and propagates each replacement identity through every later boundary', async () => {
    const calls = [];
    const result = await exerciseWindowsFrameRestart({ identity: { ...initial }, deadline: Date.now() + 60_000, ...callbacks({}, calls) });
    assert.equal(result.ok, true);
    assert.equal(result.error, null);
    assert.deepEqual(result.modes.map(item => item.mode), modes);
    assert.equal(result.modes.length, 3);
    const expected = modes.flatMap(mode => boundaryNames.map(name => `${mode}:${name}`));
    assert.deepEqual(calls.map(({ name, args }) => `${args.mode}:${name === 'verifyTerminal' ? `${name}:${args.phase}` : name}`), expected);
    let current = initial;
    for (let modeIndex = 0; modeIndex < modes.length; modeIndex++) {
        const segment = calls.slice(modeIndex * boundaryNames.length, (modeIndex + 1) * boundaryNames.length);
        assert.deepEqual(segment[0].args.identity, current);
        for (const call of segment.slice(0, 2)) assert.deepEqual(call.args.identity, current);
        const replacement = result.modes[modeIndex].identity;
        assert.ok(replacement.pid !== current.pid || replacement.creationUtc !== current.creationUtc);
        for (const call of segment.slice(2)) assert.deepEqual(call.args.identity, replacement);
        assert.deepEqual(result.modes[modeIndex].identity, replacement);
        current = replacement;
    }
    assert.deepEqual(result.identity, current);
});

test('validates all required callbacks before invoking any action', async () => {
    const calls = [];
    const supplied = callbacks({}, calls);
    delete supplied.capture;
    const result = await exerciseWindowsFrameRestart({ identity: { ...initial }, deadline: Date.now() + 60_000, ...supplied });
    assertStopped(result);
    assert.equal(calls.length, 0);
});

for (const [label, identity] of [
    ['zero PID', { ...initial, pid: 0 }],
    ['unsafe HWND', { ...initial, hwnd: Number.MAX_SAFE_INTEGER + 1 }],
    ['empty creation time', { ...initial, creationUtc: '' }]
]) {
    test(`rejects invalid initial identity (${label}) before any callback`, async () => {
        const calls = [];
        const result = await exerciseWindowsFrameRestart({ identity, deadline: Date.now() + 60_000, ...callbacks({}, calls) });
        assertStopped(result);
        assert.equal(calls.length, 0);
    });
}

test('rejects a restart that returns the same process identity even when HWND changed', async () => {
    const result = await exerciseWindowsFrameRestart(options({ restart: async ({ identity }) => ({
        identity: { ...identity, hwnd: identity.hwnd + 1 }, ownedReplacement: true,
        oldProcessGone: true, oldWindowsGone: true, oldSessionsClosed: true
    }) }));
    assertStopped(result);
    assert.equal(result.modes.length, 0);
});

test('rejects an invalid replacement identity', async () => {
    const result = await exerciseWindowsFrameRestart(options({ restart: async () => ({
        identity: { pid: -1, creationUtc: 'new', hwnd: 2 }, ownedReplacement: true,
        oldProcessGone: true, oldWindowsGone: true, oldSessionsClosed: true
    }) }));
    assertStopped(result);
    assert.equal(result.modes.length, 0);
});

for (const flag of ['ownedReplacement', 'oldProcessGone', 'oldWindowsGone', 'oldSessionsClosed']) {
    test(`requires restart cleanup flag ${flag}`, async () => {
        const result = await exerciseWindowsFrameRestart(options({ restart: async ({ identity }) => ({
            identity: { ...identity, pid: identity.pid + 1 }, ownedReplacement: true,
            oldProcessGone: true, oldWindowsGone: true, oldSessionsClosed: true, [flag]: false
        }) }));
        assertStopped(result);
        assert.equal(result.modes.length, 0);
    });
}

for (const flag of ['persisted', 'restartRequested']) {
    test(`requires selection flag ${flag}`, async () => {
        const result = await exerciseWindowsFrameRestart(options({ selectFrame: async () => ({ persisted: true, restartRequested: true, [flag]: false }) }));
        assertStopped(result);
        assert.equal(result.modes.length, 0);
    });
}

for (const [field, value] of [['selectedFrame', 'unexpected'], ['configReadback', 'unexpected'], ['frameApplied', false]]) {
    test(`rejects mismatched or unapplied frame result (${field})`, async () => {
        const result = await exerciseWindowsFrameRestart(options({ verifyFrame: async ({ mode }) => ({
            selectedFrame: mode, configReadback: mode, frameApplied: true, [field]: value
        }) }));
        assertStopped(result);
        assert.equal(result.modes.length, 0);
    });
}

for (const phase of ['before-fullscreen', 'after-fullscreen']) {
    for (const flag of ['exactBytes', 'renderedMarkerCopied']) {
        test(`requires terminal ${flag} during ${phase}`, async () => {
            const result = await exerciseWindowsFrameRestart(options({ verifyTerminal: async args => ({ exactBytes: true, renderedMarkerCopied: true, [flag]: args.phase !== phase }) }));
            assertStopped(result);
            assert.equal(result.modes.length, 0);
        });
    }
}

for (const flag of ['entered', 'controlsHidden', 'exited', 'controlsRestored']) {
    test(`rejects fullscreen failure at ${flag}`, async () => {
        const result = await exerciseWindowsFrameRestart(options({ exerciseFullscreen: async () => ({
            entered: true, controlsHidden: true, exited: true, controlsRestored: true, [flag]: false
        }) }));
        assertStopped(result);
        assert.equal(result.modes.length, 0);
    });
}

test('rejects replayed or missing consumed-argument counts', async () => {
    const result = await exerciseWindowsFrameRestart(options({ checkConsumedArguments: async () => ({ expectedLaunchCount: 1, observedLaunchCount: 2 }) }));
    assertStopped(result);
    assert.equal(result.modes.length, 0);
});

test('rejects failed capture and does not append an incomplete mode', async () => {
    const result = await exerciseWindowsFrameRestart(options({ capture: async () => ({ ok: false }) }));
    assertStopped(result);
    assert.equal(result.modes.length, 0);
});

test('stops immediately when a boundary rejects', async () => {
    const calls = [];
    const result = await exerciseWindowsFrameRestart({ ...options(), ...callbacks({ verifyFrame: async () => { throw new Error('synthetic boundary rejection'); } }, calls) });
    assertStopped(result);
    assert.equal(calls.at(-1)?.name, 'verifyFrame');
    assert.equal(calls.length, 3);
    assert.equal(result.modes.length, 0);
});

test('honors deadline expiry during a callback and restores Date.now', async () => {
    const originalNow = Date.now;
    let now = 1000;
    Date.now = () => now;
    try {
        const opts = { identity: { ...initial }, deadline: 1010, ...callbacks({ selectFrame: async () => {
            now = 1011;
            return { persisted: true, restartRequested: true };
        } }) };
        const result = await exerciseWindowsFrameRestart(opts);
        assertStopped(result);
        assert.equal(result.modes.length, 0);
    } finally {
        Date.now = originalNow;
    }
    assert.equal(Date.now, originalNow);
});
