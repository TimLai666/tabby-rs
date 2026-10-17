import test from 'node:test';
import assert from 'node:assert/strict';
import { proveRestartOrigin } from './restart-process-origin.mjs';

const FILETIME_EPOCH = 116444736000000000n;

function fileTime (utc, offsetMs = 0n) {
    return (FILETIME_EPOCH + BigInt(Date.parse(utc)) * 10000n + offsetMs * 10000n).toString(10);
}

function startEvent (pid, parentPid, createdUtc, offsetMs = 0n) {
    return { pid, parentPid, fileTime: fileTime(createdUtc, offsetMs) };
}

function fixture () {
    const seed = { pid: 100, creationTimeUTC: '2026-10-17T09:00:00.000Z' };
    const candidate = {
        pid: 200,
        parentPid: seed.pid,
        creationTimeUTC: '2026-10-17T10:00:05.500Z',
        image: 'C:/Program Files/Tabby/tabby.exe',
    };
    const trace = {
        readyUtc: '2026-10-17T10:00:00.000Z',
        overflow: false,
        error: null,
        events: [startEvent(candidate.pid, seed.pid, candidate.creationTimeUTC, 100n)],
    };
    return {
        candidate,
        seeds: [seed],
        trace,
        triggeredUtc: '2026-10-17T10:00:05.000Z',
    };
}

function assertRejected (input) {
    const result = proveRestartOrigin(input);
    assert.equal(result?.owned, false);
    assert.equal(typeof result?.reason, 'string');
    assert.ok(result.reason.length > 0);
    assert.ok(Array.isArray(result.chain));
}

test('accepts a direct candidate start whose parent is an unchanged pre-observer seed', () => {
    const result = proveRestartOrigin(fixture());
    assert.equal(result.owned, true);
    assert.deepEqual(result.chain, [200, 100]);
});

test('accepts retained start events through a relauncher that has since exited', () => {
    const input = fixture();
    input.candidate.parentPid = 300;
    input.trace.events = [
        startEvent(300, 100, '2026-10-17T10:00:05.100Z', 100n),
        startEvent(200, 300, input.candidate.creationTimeUTC, 100n),
    ];
    const result = proveRestartOrigin(input);
    assert.equal(result.owned, true);
    assert.deepEqual(result.chain, [200, 300, 100]);
});

test('rejects a same-executable candidate with no ancestry to any owned seed', () => {
    const input = fixture();
    input.candidate.parentPid = 900;
    input.trace.events[0] = startEvent(200, 900, input.candidate.creationTimeUTC, 100n);
    assertRejected(input);
});

test('rejects a candidate with no matching process-start event', () => {
    const input = fixture();
    input.trace.events = [];
    assertRejected(input);
});

test('rejects a missing intermediate ancestor event', () => {
    const input = fixture();
    input.candidate.parentPid = 300;
    input.trace.events = [startEvent(200, 300, input.candidate.creationTimeUTC, 100n)];
    assertRejected(input);
});

test('rejects a CIM candidate parent that disagrees with the native start event', () => {
    const input = fixture();
    input.candidate.parentPid = 300;
    assertRejected(input);
});

test('rejects duplicate native starts for a PID in the candidate chain', () => {
    const input = fixture();
    input.trace.events.push({ ...input.trace.events[0] });
    assertRejected(input);
});

test('rejects a reused intermediate PID with multiple start records', () => {
    const input = fixture();
    input.candidate.parentPid = 300;
    input.trace.events = [
        startEvent(300, 100, '2026-10-17T10:00:05.100Z', 100n),
        startEvent(300, 100, '2026-10-17T10:00:05.200Z', 200n),
        startEvent(200, 300, input.candidate.creationTimeUTC, 100n),
    ];
    assertRejected(input);
});

test('rejects a seed PID that restarted during the observation window', () => {
    const input = fixture();
    input.trace.events.unshift(startEvent(100, 1, '2026-10-17T10:00:01.000Z', 100n));
    assertRejected(input);
});

test('rejects cyclic process-start ancestry', () => {
    const input = fixture();
    input.candidate.parentPid = 300;
    input.trace.events = [
        startEvent(200, 300, input.candidate.creationTimeUTC, 100n),
        startEvent(300, 200, '2026-10-17T10:00:05.600Z', 100n),
    ];
    assertRejected(input);
});

test('rejects a parent whose process start is later than its child', () => {
    const input = fixture();
    input.candidate.parentPid = 300;
    input.trace.events = [
        startEvent(300, 100, '2026-10-17T10:00:05.800Z', 100n),
        startEvent(200, 300, input.candidate.creationTimeUTC, 100n),
    ];
    assertRejected(input);
});

test('rejects observer readiness later than the triggering action', () => {
    const input = fixture();
    input.trace.readyUtc = '2026-10-17T10:00:06.000Z';
    assertRejected(input);
});

test('rejects malformed observer readiness time', () => {
    const input = fixture();
    input.trace.readyUtc = 'not-an-ISO-date';
    assertRejected(input);
});

test('rejects event history marked overflowed', () => {
    const input = fixture();
    input.trace.overflow = true;
    assertRejected(input);
});

test('rejects event observer errors', () => {
    const input = fixture();
    input.trace.error = 'observer terminated unexpectedly';
    assertRejected(input);
});

test('rejects incomplete event evidence without throwing', () => {
    const input = fixture();
    delete input.trace.events;
    assertRejected(input);
});

for (const [label, field, value] of [
    ['candidate creation time', 'candidateCreation', 'October 17'],
    ['seed creation time', 'seedCreation', '2026-13-50T00:00:00Z'],
    ['trigger time', 'triggeredUtc', 'yesterday'],
]) {
    test(`rejects invalid ISO ${label}`, () => {
        const input = fixture();
        if (field === 'candidateCreation') input.candidate.creationTimeUTC = value;
        else if (field === 'seedCreation') input.seeds[0].creationTimeUTC = value;
        else input.triggeredUtc = value;
        assertRejected(input);
    });
}

for (const [label, edit] of [
    ['candidate PID zero', input => { input.candidate.pid = 0; }],
    ['candidate parent PID zero', input => { input.candidate.parentPid = 0; }],
    ['candidate PID above the safe integer range', input => { input.candidate.pid = Number.MAX_SAFE_INTEGER + 1; }],
    ['candidate parent PID above the safe integer range', input => { input.candidate.parentPid = Number.MAX_SAFE_INTEGER + 1; }],
    ['seed PID zero', input => { input.seeds[0].pid = 0; }],
    ['seed PID above the safe integer range', input => { input.seeds[0].pid = Number.MAX_SAFE_INTEGER + 1; }],
    ['event PID negative', input => { input.trace.events[0].pid = -1; }],
    ['event PID above the safe integer range', input => { input.trace.events[0].pid = Number.MAX_SAFE_INTEGER + 1; }],
    ['event parent PID negative', input => { input.trace.events[0].parentPid = -1; }],
    ['event parent PID above the safe integer range', input => { input.trace.events[0].parentPid = Number.MAX_SAFE_INTEGER + 1; }],
]) {
    test(`rejects ${label}`, () => {
        const input = fixture();
        edit(input);
        assertRejected(input);
    });
}

test('rejects a candidate created before the restart trigger', () => {
    const input = fixture();
    input.candidate.creationTimeUTC = '2026-10-17T10:00:04.000Z';
    input.trace.events[0] = startEvent(200, 100, input.candidate.creationTimeUTC, 100n);
    assertRejected(input);
});

test('rejects candidate notification more than one second after candidate creation', () => {
    const input = fixture();
    input.trace.events[0] = startEvent(200, 100, input.candidate.creationTimeUTC, 1001n);
    assertRejected(input);
});

test('rejects events that predate observer readiness', () => {
    const input = fixture();
    input.candidate.parentPid = 300;
    input.trace.events = [
        startEvent(300, 100, '2026-10-17T09:59:59.000Z', 100n),
        startEvent(200, 300, input.candidate.creationTimeUTC, 100n),
    ];
    assertRejected(input);
});

test('rejects a seed that did not predate observer readiness', () => {
    const input = fixture();
    input.seeds[0].creationTimeUTC = '2026-10-17T10:00:00.500Z';
    assertRejected(input);
});

test('rejects ancestry deeper than the bounded traversal limit', () => {
    const input = fixture();
    const events = [];
    let parentPid = 100;
    for (let pid = 300; pid < 370; pid++) {
        events.push(startEvent(pid, parentPid, '2026-10-17T10:00:05.100Z', 100n));
        parentPid = pid;
    }
    input.candidate.parentPid = parentPid;
    events.push(startEvent(200, parentPid, input.candidate.creationTimeUTC, 100n));
    input.trace.events = events;
    assertRejected(input);
});
