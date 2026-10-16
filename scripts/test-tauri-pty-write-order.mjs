import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import vm from 'node:vm';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const project = path.resolve(here, '..');
const sourcePath = path.join(project, 'tabby-tauri/src/services/ptyInterface.service.ts');
const localRequire = createRequire(path.join(project, 'package.json'));
const ts = localRequire('typescript');

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
const localSetTimeout = globalThis.setTimeout;
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
        if (delay === 2000) return { unref () {} };
        return localSetTimeout(callback, delay, ...args);
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

function createBridge (name = 'session') {
    const pending = [];
    const applied = [];
    const bridge = {
        pending,
        applied,
        async listen () { return () => {} },
        invoke (command, payload) {
            if (command === 'pty.attach') return Promise.resolve();
            if (command !== 'pty.write') return Promise.resolve();
            const operation = deferred();
            const bytes = [...payload.data];
            pending.push({ id: payload.id, bytes, operation });
            return operation.promise.then(value => {
                applied.push({ id: payload.id, bytes, session: name });
                return value;
            });
        },
    };
    return bridge;
}

async function makeProxy (name) {
    const bridge = createBridge(name);
    const proxy = await TauriPTYProxy.create({ id: name, pid: 42 }, bridge);
    return { proxy, bridge };
}

async function flushMicrotasks () {
    for (let i = 0; i < 8; i++) await Promise.resolve();
}

async function settleInCallOrderWhenSerialized (bridge, writes) {
    await flushMicrotasks();
    if (bridge.pending.length === writes.length) {
        for (const operation of [...bridge.pending].reverse()) operation.operation.resolve();
        await Promise.all(writes);
        return;
    }
    for (let i = 0; i < writes.length; i++) {
        let operation = bridge.pending[i];
        if (!operation) {
            // A correct per-proxy queue may not dispatch the next bridge call
            // until the preceding native operation has acknowledged.
            bridge.pending[i - 1].operation.resolve();
            await flushMicrotasks();
            operation = bridge.pending[i];
        }
        assert.ok(operation, `native write ${i + 1} was dispatched`);
        operation.operation.resolve();
        await flushMicrotasks();
    }
    await Promise.all(writes);
}

test('two overlapping writes preserve their original byte order under reversed bridge latency', async () => {
    const { proxy, bridge } = await makeProxy('fifo-pair');
    const first = proxy.write(Buffer.from('first|'));
    const second = proxy.write(Buffer.from('second|'));
    await flushMicrotasks();

    if (bridge.pending.length > 1) {
        bridge.pending[1].operation.resolve();
        bridge.pending[0].operation.resolve();
    } else {
        bridge.pending[0].operation.resolve();
        await flushMicrotasks();
        bridge.pending[1].operation.resolve();
    }
    await Promise.all([first, second]);
    assert.deepEqual(bridge.applied.flatMap(item => item.bytes), [...Buffer.from('first|second|')]);
});

test('a long per-character command reaches the native bridge in exact input order', async () => {
    const { proxy, bridge } = await makeProxy('character-command');
    const command = 'powershell -NoProfile -Command "Write-Output ORDER-1234567890"\r\n';
    const writes = [...command].map(character => proxy.write(Buffer.from(character)));
    await settleInCallOrderWhenSerialized(bridge, writes);
    assert.deepEqual(bridge.applied.flatMap(item => item.bytes), [...Buffer.from(command)]);
});

test('a Buffer write snapshots bytes at call time while it waits behind an earlier write', async () => {
    const { proxy, bridge } = await makeProxy('buffer-snapshot');
    const first = proxy.write(Buffer.from('wait'));
    const mutable = Buffer.from([0x41, 0x00, 0x0d, 0x0a, 0x42]);
    const second = proxy.write(mutable);
    mutable.fill(0x7f);
    await flushMicrotasks();
    await settleInCallOrderWhenSerialized(bridge, [first, second]);
    assert.ok(bridge.applied.some(item => Buffer.from(item.bytes).equals(Buffer.from([0x41, 0x00, 0x0d, 0x0a, 0x42]))));
});

test('a rejected write rejects with its own error and a later queued write still completes', async () => {
    const { proxy, bridge } = await makeProxy('error-recovery');
    const expectedError = new Error('native write failed');
    const first = proxy.write(Buffer.from('bad'));
    const firstRejected = assert.rejects(first, error => error === expectedError);
    const second = proxy.write(Buffer.from('good'));
    await flushMicrotasks();
    bridge.pending[0].operation.reject(expectedError);
    await firstRejected;
    await flushMicrotasks();
    if (!bridge.pending[1]) await flushMicrotasks();
    bridge.pending[1].operation.resolve();
    await second;
    assert.deepEqual(bridge.applied.map(item => item.bytes), [[...Buffer.from('good')]]);
});

test('each write Promise acknowledges only its own native operation', async () => {
    const { proxy, bridge } = await makeProxy('ack-timing');
    const first = proxy.write(Buffer.from('one'));
    const second = proxy.write(Buffer.from('two'));
    let firstSettled = false;
    let secondSettled = false;
    first.then(() => { firstSettled = true; });
    second.then(() => { secondSettled = true; });
    await flushMicrotasks();
    bridge.pending[0].operation.resolve();
    await first;
    await flushMicrotasks();
    assert.equal(firstSettled, true);
    assert.equal(secondSettled, false);
    if (!bridge.pending[1]) await flushMicrotasks();
    bridge.pending[1].operation.resolve();
    await second;
    assert.equal(secondSettled, true);
});

test('writes on separate proxies progress independently', async () => {
    const a = await makeProxy('independent-a');
    const b = await makeProxy('independent-b');
    const blocked = a.proxy.write(Buffer.from('blocked'));
    const independent = b.proxy.write(Buffer.from('independent'));
    await flushMicrotasks();
    assert.equal(a.bridge.pending.length, 1);
    assert.equal(b.bridge.pending.length, 1);
    b.bridge.pending[0].operation.resolve();
    await independent;
    assert.deepEqual(b.bridge.applied[0].bytes, [...Buffer.from('independent')]);
    a.bridge.pending[0].operation.resolve();
    await blocked;
});

test('raw binary bytes and independently split UTF-8 writes are preserved exactly', async () => {
    const { proxy, bridge } = await makeProxy('binary-and-utf8');
    const raw = Buffer.from([0x00, 0x0d, 0x0a, 0xff, 0x80, 0x41]);
    const utf8 = Buffer.from('你');
    const parts = [raw, utf8.subarray(0, 1), utf8.subarray(1, 2), utf8.subarray(2)];
    const writes = parts.map(part => proxy.write(part));
    await settleInCallOrderWhenSerialized(bridge, writes);
    assert.deepEqual(bridge.applied.flatMap(item => item.bytes), [...Buffer.concat(parts)]);
});
