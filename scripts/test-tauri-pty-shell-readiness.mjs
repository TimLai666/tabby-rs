import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import vm from 'node:vm';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const project = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const sourcePath = path.join(project, 'tabby-tauri/src/services/ptyInterface.service.ts');
const ts = createRequire(path.join(project, 'package.json'))('typescript');
const source = await readFile(sourcePath, 'utf8');
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
        experimentalDecorators: true,
        emitDecoratorMetadata: false,
    },
    fileName: sourcePath,
});
assert.equal(compiled.diagnostics?.some(diagnostic => diagnostic.category === ts.DiagnosticCategory.Error), false);

class FakePTYProxy {}
class FakePTYInterface {}
const moduleObject = { exports: {} };
const startupTimers = [];
const sandbox = {
    exports: moduleObject.exports,
    module: moduleObject,
    Buffer,
    console,
    Promise,
    Map,
    Set,
    Uint8Array,
    Array,
    Object,
    Error,
    setTimeout (callback, delay, ...args) {
        if (delay === 2000) {
            const timer = { callback: () => callback(...args) };
            startupTimers.push(timer);
            return timer;
        }
        return globalThis.setTimeout(callback, delay, ...args);
    },
    clearTimeout,
    require (specifier) {
        if (specifier === '@angular/core') return { Injectable: () => target => target };
        if (specifier === '../../../tabby-local/src/api') return {
            PTYInterface: FakePTYInterface,
            PTYProxy: FakePTYProxy,
        };
        if (specifier === '../api/hostBridge' || specifier === '../api/pty' || specifier === './shellProvider.service') return {};
        throw new Error(`Unexpected source dependency: ${specifier}`);
    },
};
vm.runInNewContext(`(function (exports, require, module) { ${compiled.outputText}\n})`, sandbox)(
    moduleObject.exports,
    sandbox.require,
    moduleObject,
);
const { TauriPTYProxy } = moduleObject.exports;

function deferred () {
    let resolve;
    let reject;
    const promise = new Promise((resolvePromise, rejectPromise) => {
        resolve = resolvePromise;
        reject = rejectPromise;
    });
    return { promise, resolve, reject };
}

function bridgeFor (id) {
    const calls = [];
    const startup = deferred();
    const children = [];
    const cwds = [];
    const bridge = {
        id,
        calls,
        startup,
        children,
        cwds,
        async listen () { return () => {} },
        invoke (command, payload) {
            calls.push({ command, payload });
            if (command === 'pty.attach') return Promise.resolve();
            if (command === 'pty.getTruePid') return startup.promise;
            if (command === 'pty.getChildren') {
                const pending = deferred();
                children.push(pending);
                return pending.promise;
            }
            if (command === 'pty.getCwd') {
                const pending = deferred();
                cwds.push(pending);
                return pending.promise;
            }
            return Promise.resolve();
        },
    };
    return bridge;
}

async function createProxy (id) {
    const bridge = bridgeFor(id);
    const pid = 400 + startupTimers.length;
    const proxy = await TauriPTYProxy.create({ id, pid }, bridge);
    const timer = startupTimers.at(-1);
    return { bridge, pid, proxy, timer };
}

async function flushMicrotasks () {
    for (let i = 0; i < 8; i++) await Promise.resolve();
}

async function completeStartup (fixture) {
    fixture.timer.callback();
    await flushMicrotasks();
    fixture.bridge.startup.resolve(900 + startupTimers.indexOf(fixture.timer));
    await flushMicrotasks();
}

function readCallCount (bridge) {
    return bridge.calls.filter(call => call.command === 'pty.getChildren' || call.command === 'pty.getCwd').length;
}

async function releaseReads (bridge) {
    for (const pending of bridge.children) pending.resolve([]);
    for (const pending of bridge.cwds) pending.resolve('C:/work');
    await flushMicrotasks();
}

test('child and working-directory native reads wait until the existing startup identity is acknowledged', async () => {
    const fixture = await createProxy('readiness-gate');
    const childrenResult = fixture.proxy.getChildProcesses().then(value => ({ value }), error => ({ error }));
    const cwdResult = fixture.proxy.getWorkingDirectory().then(value => ({ value }), error => ({ error }));
    try {
        await flushMicrotasks();
        assert.equal(readCallCount(fixture.bridge), 0);
        fixture.timer.callback();
        await flushMicrotasks();
        assert.equal(fixture.bridge.calls.filter(call => call.command === 'pty.getTruePid').length, 1);
        assert.equal(readCallCount(fixture.bridge), 0);
        fixture.bridge.startup.resolve(901);
        await flushMicrotasks();
        assert.equal(readCallCount(fixture.bridge), 2);
        await releaseReads(fixture.bridge);
        await Promise.all([childrenResult, cwdResult]);
    } finally {
        if (!fixture.bridge.calls.some(call => call.command === 'pty.getTruePid')) fixture.timer.callback();
        fixture.bridge.startup.resolve(901);
        await flushMicrotasks();
        await releaseReads(fixture.bridge);
    }
});

test('one proxy reuses its existing startup identity Promise for both native reads', async () => {
    const fixture = await createProxy('single-startup-promise');
    await completeStartup(fixture);
    const childrenResult = fixture.proxy.getChildProcesses();
    const cwdResult = fixture.proxy.getWorkingDirectory();
    await flushMicrotasks();
    assert.equal(fixture.bridge.calls.filter(call => call.command === 'pty.getTruePid').length, 1);
    assert.equal(readCallCount(fixture.bridge), 2);
    await releaseReads(fixture.bridge);
    await Promise.all([childrenResult, cwdResult]);
});

test('a waiting proxy does not block another proxy whose startup identity is ready', async () => {
    const waiting = await createProxy('waiting-session');
    const ready = await createProxy('ready-session');
    const waitingResult = waiting.proxy.getChildProcesses().then(value => ({ value }), error => ({ error }));
    const readyResult = ready.proxy.getChildProcesses();
    try {
        await flushMicrotasks();
        assert.equal(readCallCount(waiting.bridge), 0);
        await completeStartup(ready);
        assert.equal(readCallCount(ready.bridge), 1);
        assert.equal(readCallCount(waiting.bridge), 0);
        ready.bridge.children[0].resolve([{ pid: 901 }]);
        assert.deepEqual(await readyResult, [{ pid: 901 }]);
    } finally {
        if (!waiting.bridge.calls.some(call => call.command === 'pty.getTruePid')) waiting.timer.callback();
        waiting.bridge.startup.resolve(902);
        await flushMicrotasks();
        await releaseReads(waiting.bridge);
        await releaseReads(ready.bridge);
        await waitingResult;
    }
});

test('native child and working-directory read errors propagate unchanged after startup readiness', async () => {
    const fixture = await createProxy('native-read-errors');
    await completeStartup(fixture);
    const childrenError = new Error('children read failed');
    const cwdError = new Error('cwd read failed');
    const childrenResult = assert.rejects(fixture.proxy.getChildProcesses(), error => error === childrenError);
    const cwdResult = assert.rejects(fixture.proxy.getWorkingDirectory(), error => error === cwdError);
    await flushMicrotasks();
    fixture.bridge.children[0].reject(childrenError);
    fixture.bridge.cwds[0].reject(cwdError);
    await Promise.all([childrenResult, cwdResult]);
});

test('a rejected startup resolver falls back to getPID, releases both reads, and leaves another proxy independent', async () => {
    const waiting = await createProxy('fallback-waiting-proxy');
    const fallback = await createProxy('fallback-ready-proxy');
    const waitingResult = waiting.proxy.getWorkingDirectory().then(value => ({ value }), error => ({ error }));
    const startupError = new Error('true PID lookup failed');
    const childrenError = new Error('child lookup failed after PID fallback');
    const cwdError = new Error('cwd lookup failed after PID fallback');
    const childrenResult = fallback.proxy.getChildProcesses();
    const cwdResult = fallback.proxy.getWorkingDirectory();
    const childrenRejected = assert.rejects(childrenResult, error => error === childrenError);
    const cwdRejected = assert.rejects(cwdResult, error => error === cwdError);

    try {
        await flushMicrotasks();
        assert.equal(readCallCount(waiting.bridge), 0);
        assert.equal(readCallCount(fallback.bridge), 0);

        fallback.timer.callback();
        await flushMicrotasks();
        assert.equal(fallback.bridge.calls.filter(call => call.command === 'pty.getTruePid').length, 1);
        fallback.bridge.startup.reject(startupError);
        await flushMicrotasks();

        assert.equal(await fallback.proxy.getTruePID(), fallback.pid);
        assert.equal(fallback.bridge.calls.filter(call => call.command === 'pty.getTruePid').length, 1);
        assert.equal(readCallCount(fallback.bridge), 2);
        assert.equal(readCallCount(waiting.bridge), 0);

        fallback.bridge.children[0].reject(childrenError);
        fallback.bridge.cwds[0].reject(cwdError);
        await Promise.all([childrenRejected, cwdRejected]);
        assert.equal(readCallCount(waiting.bridge), 0);
    } finally {
        if (!fallback.bridge.calls.some(call => call.command === 'pty.getTruePid')) fallback.timer.callback();
        fallback.bridge.startup.resolve(fallback.pid);
        await flushMicrotasks();
        await releaseReads(fallback.bridge);

        if (!waiting.bridge.calls.some(call => call.command === 'pty.getTruePid')) waiting.timer.callback();
        waiting.bridge.startup.resolve(waiting.pid);
        await flushMicrotasks();
        await releaseReads(waiting.bridge);
        await waitingResult;
    }
});
