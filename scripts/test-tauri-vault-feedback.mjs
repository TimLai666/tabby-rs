import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const json = value => JSON.parse(JSON.stringify(value))
const compilerOptions = {
    target: ts.ScriptTarget.ES2020,
    module: ts.ModuleKind.CommonJS,
    experimentalDecorators: true,
    esModuleInterop: true,
}
const BASELINE = '14e2d60b9b6dee84a53c37f05eefeb803787de04'
const mutations = new Set([
    'vault.setEnabled', 'vault.replace', 'vault.setConfig',
    'vault.putSecret', 'vault.updateSecret', 'vault.removeSecret', 'vault.import',
])
const storage = { version: 1, contents: 'ciphertext', keySalt: 'aa', iv: 'bb' }
const window = { localStorage: {} }

function fakeTimers () {
    const pending = new Map()
    let seq = 0
    return {
        pending,
        setTimeout: (fn, ms) => { const id = ++seq; pending.set(id, { fn, ms }); return id },
        clearTimeout: id => pending.delete(id),
    }
}

function loadModule (file, modules, globals = {}) {
    const source = fs.readFileSync(file, 'utf8')
    const compiled = ts.transpileModule(source, { compilerOptions }).outputText
    const exports = {}
    Object.defineProperty(exports, '__esModule', { value: true, enumerable: false })
    vm.runInNewContext(compiled, {
        exports,
        require: specifier => {
            assert.ok(specifier in modules, `unprovided module ${specifier}`)
            return modules[specifier]
        },
        console,
        ...globals,
    }, { filename: file })
    return { exports, source }
}

function notificationRecorder () {
    const calls = []
    return {
        calls,
        notice (text) { calls.push({ method: 'notice', text }) },
        info (text, details) { calls.push({ method: 'info', text, details }) },
        error (text, details) { calls.push({ method: 'error', text, details }) },
    }
}

function toastrRecorder () {
    const calls = []
    return { calls, info (...args) { calls.push(['info', ...args]) }, error (...args) { calls.push(['error', ...args]) } }
}

function bridgeStub (handlers) {
    const calls = []
    return {
        calls,
        async invoke (command, request) {
            calls.push({ command, request })
            const handler = handlers[command]
            if (handler === undefined) throw new Error(`unexpected command ${command}`)
            return typeof handler === 'function' ? handler(request) : handler
        },
    }
}

const vaultModule = loadModule(
    path.join(root, 'tabby-tauri/src/services/vault.service.ts'),
    {
        '@angular/core': { Injectable: () => target => target },
        '@ng-bootstrap/ng-bootstrap': { NgbModal: class {} },
        rxjs: require('rxjs'),
        'tabby-core': { UnlockVaultModalComponent: class {} },
        '../api/hostBridge': {},
    },
    { window, ...fakeTimers() },
)
const TauriVaultService = vaultModule.exports.TauriVaultService

function buildVaultService (boundaries) {
    return new TauriVaultService(boundaries.bridge, boundaries.modal, boundaries.notifications)
}

const sharedNotificationsModule = loadModule(
    path.join(root, 'tabby-core/src/services/notifications.service.ts'),
    { '@angular/core': { Injectable: () => target => target }, 'ngx-toastr': { ToastrService: class {} } },
)
const SharedNotifications = sharedNotificationsModule.exports.NotificationsService
const nativeNotificationsPath = path.join(root, 'tabby-tauri/src/services/notifications.service.ts')
const nativeNotificationsModule = fs.existsSync(nativeNotificationsPath) ? loadModule(
    nativeNotificationsPath,
    { '@angular/core': { Injectable: () => target => target }, '../api/hostBridge': {} },
) : null
const NativeNotifications = nativeNotificationsModule?.exports.TauriNotificationsService

const baselineSource = execFileSync('git', ['show', `${BASELINE}:tabby-core/src/services/vault.service.ts`], { cwd: root, encoding: 'utf8' })
const baselineFile = ts.createSourceFile('baselineVault.ts', baselineSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
let baselineClass
baselineFile.forEachChild(node => {
    if (ts.isClassDeclaration(node) && node.name?.getText(baselineFile) === 'VaultService') baselineClass = node
})
assert.ok(baselineClass, 'baseline VaultService must exist')
let baselineDecrypt
baselineClass.members.forEach(member => {
    if (ts.isMethodDeclaration(member) && member.name.getText(baselineFile) === 'decrypt') baselineDecrypt = member
})
assert.ok(baselineDecrypt, 'baseline VaultService.decrypt must exist')
const baselineCompiled = ts.transpileModule(
    `class BaselineVaultService { ${baselineDecrypt.getText(baselineFile)} }`,
    { compilerOptions },
).outputText
const baselineExports = {}
vm.runInNewContext(`${baselineCompiled}\nexports.BaselineVaultService = BaselineVaultService`, {
    exports: baselineExports,
    wrapPromise: (_zone, promise) => promise,
    decryptVault: () => { throw new Error('BAD_DECRYPT: incorrect passphrase') },
}, { filename: 'baseline-vault.js' })

async function fixedOriginalNotifications () {
    const recorder = notificationRecorder()
    const receiver = Object.create(baselineExports.BaselineVaultService.prototype)
    receiver.notifications = recorder
    receiver.zone = {}
    receiver.forgetPassphrase = () => {}
    await receiver.decrypt({ ...storage }, 'wrong-passphrase').catch(() => null)
    return recorder.calls
}

const expectedPasswordNotice = json((await fixedOriginalNotifications()).map(call => ({ method: call.method, text: call.text })))
assert.deepEqual(
    expectedPasswordNotice, [{ method: 'error', text: 'Incorrect passphrase' }],
    'the fixed original VaultService.decrypt must report exactly one Incorrect passphrase notice',
)

function assertPasswordNotice (recorder) {
    assert.deepEqual(
        json(recorder.calls.map(call => ({ method: call.method, text: call.text }))),
        expectedPasswordNotice,
        'notification must equal the fixed original VaultService.decrypt result',
    )
}

const decryptionFailure = { code: 'permissionDenied', details: 'vault unlock failed' }
const checks = []
async function check (name, fn) {
    try {
        await fn()
        checks.push({ name, ok: true })
        console.log(`PASS ${name}`)
    } catch (error) {
        checks.push({ name, ok: false })
        console.log(`FAIL ${name}: ${error.message}`)
    }
}

await check('provided-passphrase unlock failure mirrors the original notice and rethrows the same object', async () => {
    window.localStorage.vaultRememberPassphraseFor = '1'
    const notifications = notificationRecorder()
    const bridge = bridgeStub({ 'vault.unlock': () => { throw decryptionFailure }, 'vault.lock': () => null })
    const modalCalls = []
    const modal = { open: component => { modalCalls.push(component); return { result: Promise.resolve(null) } } }
    const service = buildVaultService({ bridge, modal, notifications })
    service.setStore(storage)
    let caught = null
    try { await service.decrypt(storage, 'wrong-passphrase') } catch (error) { caught = error }
    assert.equal(caught, decryptionFailure, 'the exact native failure object must be rethrown')
    assert.equal(modalCalls.length, 0, 'a provided passphrase must not open the unlock modal')
    assert.equal(bridge.calls.some(call => call.command === 'vault.snapshot'), false, 'no snapshot may run on failure')
    assert.equal(bridge.calls.some(call => mutations.has(call.command)), false, 'no store mutation may run on failure')
    assert.equal(service.isOpen(), false, 'a failed unlock must not mark the vault open')
    assert.equal(service.store, storage, 'the stored vault must be retained')
    const unlock = bridge.calls.find(call => call.command === 'vault.unlock')
    assert.equal(unlock.request.passphrase, 'wrong-passphrase')
    assert.equal(unlock.request.rememberForSeconds, 60)
    assertPasswordNotice(notifications)
})

await check('modal-passphrase unlock failure mirrors the original notice and rethrows the same object', async () => {
    const notifications = notificationRecorder()
    const bridge = bridgeStub({ 'vault.unlock': () => { throw decryptionFailure }, 'vault.lock': () => null })
    const opened = []
    const modal = { open: component => { opened.push(component); return { result: Promise.resolve({ passphrase: 'typed-wrong', rememberFor: 2 }) } } }
    const service = buildVaultService({ bridge, modal, notifications })
    service.setStore(storage)
    let caught = null
    try { await service.decrypt(storage) } catch (error) { caught = error }
    assert.equal(caught, decryptionFailure, 'the exact native failure object must be rethrown')
    assert.equal(opened.length, 1, 'the unlock modal must open once when no passphrase is supplied')
    const unlock = bridge.calls.find(call => call.command === 'vault.unlock')
    assert.equal(unlock.request.passphrase, 'typed-wrong')
    assert.equal(unlock.request.rememberForSeconds, 120)
    assert.equal(bridge.calls.some(call => call.command === 'vault.snapshot'), false, 'no snapshot may run on failure')
    assert.equal(bridge.calls.some(call => mutations.has(call.command)), false, 'no store mutation may run on failure')
    assert.equal(service.isOpen(), false)
    assert.equal(service.store, storage)
    assertPasswordNotice(notifications)
})

await check('the same instance retries successfully, retaining the store and remember seconds', async () => {
    window.localStorage.vaultRememberPassphraseFor = '3'
    const notifications = notificationRecorder()
    let attempts = 0
    const bridge = bridgeStub({
        'vault.unlock': () => { attempts++; if (attempts === 1) throw decryptionFailure; return null },
        'vault.snapshot': () => ({ config: { theme: 'x' }, secrets: [] }),
        'vault.lock': () => null,
    })
    const service = buildVaultService({ bridge, modal: { open: () => ({ result: Promise.resolve(null) }) }, notifications })
    service.setStore(storage)
    const preference = window.localStorage.vaultRememberPassphraseFor
    let caught = null
    try { await service.decrypt(storage, 'wrong-password') } catch (error) { caught = error }
    assert.equal(caught, decryptionFailure)
    assert.equal(service.isOpen(), false)
    const vault = await service.decrypt(storage, 'right-password')
    assert.equal(attempts, 2, 'the retry must reach native unlock again')
    assert.equal(service.isOpen(), true, 'the successful retry must open the vault')
    assert.equal(service.store, storage, 'the store must be retained across failure and retry')
    assert.equal(window.localStorage.vaultRememberPassphraseFor, preference, 'the remember preference must be untouched')
    const unlocks = bridge.calls.filter(call => call.command === 'vault.unlock')
    assert.deepEqual(unlocks.map(call => call.request.rememberForSeconds), [180, 180], 'remember seconds unchanged across failure and retry')
    assert.equal(bridge.calls.filter(call => call.command === 'vault.snapshot').length, 1)
    assert.deepEqual(json(vault), { config: { theme: 'x' }, secrets: [] })
    assertPasswordNotice(notifications)
})

for (const [label, modal] of [
    ['null result', { open: () => ({ result: Promise.resolve(null) }) }],
    ['rejected result', { open: () => ({ result: Promise.reject(new Error('dismissed')) }) }],
    ['setup throw', { open: () => { throw new Error('no modal available') } }],
]) {
    await check(`modal ${label} must not unlock or report a password error`, async () => {
        const notifications = notificationRecorder()
        const bridge = bridgeStub({ 'vault.unlock': () => { throw new Error('must not unlock') }, 'vault.lock': () => null })
        const service = buildVaultService({ bridge, modal, notifications })
        service.setStore(storage)
        await assert.rejects(service.decrypt(storage))
        assert.equal(bridge.calls.length, 0, 'no native command may run after cancellation')
        assert.deepEqual(notifications.calls, [], 'cancellation must not be reported as a bad passphrase')
    })
}

for (const [label, failure] of [
    ['invalidData format/version', { code: 'invalidData', details: 'unsupported vault format version 2' }],
    ['io failure', { code: 'io', details: 'vault operation failed' }],
    ['permissionDenied locked', { code: 'permissionDenied', details: 'vault is locked' }],
    ['unrelated error', new Error('network down')],
]) {
    await check(`${label} must preserve the original error without a password notice`, async () => {
        const notifications = notificationRecorder()
        const bridge = bridgeStub({ 'vault.unlock': () => { throw failure }, 'vault.lock': () => null })
        const service = buildVaultService({ bridge, modal: { open: () => ({ result: Promise.resolve(null) }) }, notifications })
        service.setStore(storage)
        let caught = null
        try { await service.decrypt(storage, 'wrong-passphrase') } catch (error) { caught = error }
        assert.equal(caught, failure, 'the original error object must be preserved')
        assert.deepEqual(notifications.calls, [], 'a non-decryption failure must not report a bad passphrase')
    })
}

await check('TauriModule binds NotificationsService to the shared Toastr service', async () => {
    const ngModules = []
    const moduleCache = new Map()
    function moduleFor (specifier) {
        if (!moduleCache.has(specifier)) {
            const exports = {}
            Object.defineProperty(exports, '__esModule', { value: true, enumerable: false })
            moduleCache.set(specifier, new Proxy(exports, {
                get: (target, name) => {
                    if (name in target) return target[name]
                    if (typeof name !== 'string') return undefined
                    target[name] = class {}
                    return target[name]
                },
            }))
        }
        return moduleCache.get(specifier)
    }
    moduleFor('@angular/core').NgModule = metadata => target => { ngModules.push({ metadata, target }) }
    moduleFor('@angular/core').Injectable = () => target => target
    moduleFor('tabby-core').NotificationsService = SharedNotifications

    const indexSource = fs.readFileSync(path.join(root, 'tabby-tauri/src/index.ts'), 'utf8')
    const indexExports = {}
    vm.runInNewContext(ts.transpileModule(indexSource, { compilerOptions }).outputText, {
        exports: indexExports,
        require: specifier => {
            if (specifier === './services/notifications.service') {
                assert.ok(nativeNotificationsModule, 'a notification provider import must resolve to an existing module')
                return nativeNotificationsModule.exports
            }
            return moduleFor(specifier)
        },
        console,
    }, { filename: 'index.ts' })

    assert.equal(ngModules.length, 1, 'the Tauri entry point must declare exactly one NgModule')
    const providers = ngModules[0].metadata.providers
    assert.ok(Array.isArray(providers) && providers.length > 0, 'TauriModule must declare providers')
    const binding = providers.find(provider => provider && provider.provide === SharedNotifications)
    const effective = binding ? (binding.useExisting ?? binding.useClass ?? binding.provide) : SharedNotifications

    const toastr = toastrRecorder()
    const bridge = bridgeStub({ 'notification.show': () => null })
    const isShared = effective === SharedNotifications
    const isNative = effective === NativeNotifications
    assert.ok(isShared || isNative, 'the notification binding must resolve to a known notification service')
    const instance = new effective(isShared ? toastr : bridge)

    const referenceToastr = toastrRecorder()
    const reference = new SharedNotifications(referenceToastr)
    for (const service of [instance, reference]) {
        service.notice('notice-text')
        service.info('info-text', 'info-details')
        service.error('error-text', 'error-details')
    }

    assert.deepEqual(json(referenceToastr.calls), [
        ['info', 'notice-text', null, { timeOut: 1000 }],
        ['info', 'info-text', 'info-details'],
        ['error', 'error-text', 'error-details'],
    ], 'the fixed original shared NotificationsService is the reference')
    assert.deepEqual(json(toastr.calls), json(referenceToastr.calls), 'the bound notification service must emit the fixed-original Toastr calls')
    assert.deepEqual(bridge.calls, [], 'the bound notification service must not send OS notifications')
})

const failed = checks.filter(entry => !entry.ok).length
console.log(`Tauri Vault feedback regression cases: ${checks.length - failed} passed, ${failed} failed, ${checks.length} total`)
if (failed) process.exitCode = 1
