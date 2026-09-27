import assert from 'node:assert/strict'
import { createHash, webcrypto } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import ts from 'typescript'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const json = value => JSON.parse(JSON.stringify(value))
const hashOf = text => createHash('sha512').update(text).digest('hex')

/** Stands in for a module the service only needs as a DI token or an enum. */
function fakeModule (name, explicit = {}) {
    const target = { ...explicit }
    Object.defineProperty(target, '__esModule', { value: true, enumerable: false })
    return new Proxy(target, {
        get: (obj, prop) => {
            if (prop in obj) {
                return obj[prop]
            }
            if (typeof prop !== 'string') {
                return undefined
            }
            target[prop] = class {}
            Object.defineProperty(target[prop], 'name', { value: `${name}#${prop}` })
            return target[prop]
        },
    })
}

// Only the DI tokens and the Platform enum are faked. The SSH profile and session types
// are not listed, so an import that is more than a type fails loudly instead of being
// resolved from a stand-in, and the tabby-ssh barrel is not reachable at all.
const fakes = {
    '@angular/core': fakeModule('@angular/core', { Injectable: () => target => target }),
    'tabby-core': fakeModule('tabby-core', {
        Platform: { Windows: 'windows', macOS: 'macos', Linux: 'linux', Web: 'web' },
    }),
    '../../../tabby-ssh/src/services/passwordStorage.service': fakeModule('passwordStorage.service'),
    '../api/hostBridge': fakeModule('../api/hostBridge'),
}

const servicePath = path.join(root, 'tabby-tauri/src/services/winscp.service.ts')
const serviceModule = { exports: {} }
vm.runInNewContext(
    ts.transpileModule(fs.readFileSync(servicePath, 'utf8'), {
        compilerOptions: {
            target: ts.ScriptTarget.ES2020,
            module: ts.ModuleKind.CommonJS,
            experimentalDecorators: true,
            emitDecoratorMetadata: true,
        },
    }).outputText,
    {
        exports: serviceModule.exports,
        require: name => {
            if (!(name in fakes)) {
                throw new Error(`Unexpected dependency: ${name}`)
            }
            return fakes[name]
        },
        console,
        window: { crypto: webcrypto },
        TextEncoder,
    },
    { filename: servicePath },
)
const { TauriWinSCPService } = serviceModule.exports
assert.equal(typeof TauriWinSCPService, 'function', 'winscp.service.ts must export the injectable service')

const DETECTED = 'C:\\Program Files\\WinSCP\\WinSCP.exe'
const MANUAL = 'D:\\Tools\\WinSCP\\WinSCP.exe'
const JUMP_KEY = 'file://jump-key'
const TARGET_KEY = 'file://target-key'
const LATER_KEY = 'file://later-key'
const FILE_CONTENT = {
    [JUMP_KEY]: 'jump key material\n',
    [TARGET_KEY]: 'target key material\n',
    [LATER_KEY]: 'later key material\n',
}

/**
 * Builds the service with every boundary mocked and records what it asked for.
 * `convertResults` are handed out one per `winscp.convertKey` call, in order.
 */
function createService (options = {}) {
    const calls = { convert: [], launch: [], retrieved: [], loadPassword: [], loadKeyPassword: [], detection: 0 }
    const convertResults = [...(options.convertResults ?? [])]
    const service = new TauriWinSCPService(
        {
            loadPassword: async (profile, username) => {
                calls.loadPassword.push({ profile: profile.id, username: username ?? null })
                return (options.passwords ?? {})[profile.id] ?? null
            },
            loadPrivateKeyPassword: async hash => {
                calls.loadKeyPassword.push(hash)
                return (options.passphrases ?? {})[hash] ?? null
            },
        },
        { store: { ssh: { winSCPPath: options.manualPath ?? null }, profiles: options.profiles ?? [] } },
        { platform: options.platform ?? 'windows' },
        { getWinSCPPath: () => { calls.detection += 1; return options.detectedPath ?? null } },
        {
            retrieveFile: async ref => {
                calls.retrieved.push(ref)
                if (!(ref in FILE_CONTENT)) {
                    throw new Error(`Private key file is unavailable: ${ref}`)
                }
                return Buffer.from(FILE_CONTENT[ref])
            },
        },
        {
            invoke: async (command, request) => {
                if (command === 'winscp.convertKey') {
                    calls.convert.push(json(request))
                    const result = convertResults.length ? convertResults.shift() : null
                    if (result instanceof Error) {
                        throw result
                    }
                    return result
                }
                if (command === 'winscp.launch') {
                    if (options.launchError) {
                        throw options.launchError
                    }
                    calls.launch.push(json(request))
                    return null
                }
                throw new Error(`Unexpected command: ${command}`)
            },
        },
    )
    return { service, calls }
}

const profile = (over = {}) => ({
    id: 'target',
    type: 'ssh',
    options: { host: 'target.example.com', port: null, user: 'root', auth: null, privateKeys: [], jumpHost: null, ...over },
})
const session = (target, authUsername = null, activePrivateKey = false) => ({ profile: target, authUsername, activePrivateKey })

const results = []
const test = async (name, fn) => {
    try {
        await fn()
        results.push({ name, ok: true })
    } catch (error) {
        results.push({ name, ok: false, error })
    }
}

await test('detected path wins over the manually configured one', async () => {
    const { service, calls } = createService({ detectedPath: DETECTED, manualPath: MANUAL })
    assert.equal(service.getWinSCPPath(), DETECTED)
    assert.equal(calls.detection, 1, 'the detected path is read once, at construction')
    await service.launchWinSCP(session(profile()))
    assert.deepEqual(calls.launch, [{ executable: DETECTED, target: {
        host: 'target.example.com', port: 22, username: 'root', password: null, privateKey: null,
    }, jump: null }])
})

await test('manually configured path is used when nothing is detected', async () => {
    const { service, calls } = createService({ manualPath: MANUAL })
    assert.equal(service.getWinSCPPath(), MANUAL)
    await service.launchWinSCP(session(profile()))
    assert.equal(calls.launch[0].executable, MANUAL)
})

await test('non-Windows hosts never launch WinSCP and never probe for it', async () => {
    const { service, calls } = createService({ platform: 'macos', detectedPath: DETECTED, manualPath: MANUAL })
    assert.equal(calls.detection, 0, 'path detection only runs on Windows')
    assert.equal(service.getWinSCPPath(), MANUAL)
    await service.launchWinSCP(session(profile()))
    assert.deepEqual(calls.launch, [])
})

await test('a missing WinSCP path makes the launch a no-op', async () => {
    const { service, calls } = createService()
    assert.equal(service.getWinSCPPath(), null)
    await service.launchWinSCP(session(profile()))
    assert.deepEqual(calls.launch, [])
    assert.deepEqual(calls.loadPassword, [], 'no credential is read when WinSCP cannot be started')
})

await test('the authenticated username and password are used for the target', async () => {
    const { service, calls } = createService({ detectedPath: DETECTED, passwords: { target: 'target-secret' } })
    await service.launchWinSCP(session(profile({ port: 2222, user: 'configured' }), 'authenticated', false))
    assert.deepEqual(calls.loadPassword, [{ profile: 'target', username: 'authenticated' }])
    assert.equal(calls.launch[0].target.username, 'authenticated')
    assert.equal(calls.launch[0].target.password, 'target-secret')
    assert.equal(calls.launch[0].target.port, 2222)
})

await test('the target falls back to the configured username', async () => {
    const { service, calls } = createService({ detectedPath: DETECTED })
    await service.launchWinSCP(session(profile({ user: 'configured' })))
    assert.deepEqual(calls.loadPassword, [{ profile: 'target', username: null }])
    assert.equal(calls.launch[0].target.username, 'configured')
})

await test('conversion stops at the first key WinSCP accepts', async () => {
    const target = profile({ privateKeys: [TARGET_KEY, 'file://missing-later-key'] })
    const { service, calls } = createService({
        detectedPath: DETECTED,
        convertResults: [{ content: 'converted first', passphrase: 'tabby' }],
    })
    await service.launchWinSCP(session(target, null, true))
    assert.deepEqual(calls.retrieved, [TARGET_KEY], 'a key that is never asked for is never read')
    assert.deepEqual(calls.convert, [
        { executable: DETECTED, key: { content: FILE_CONTENT[TARGET_KEY], passphrase: 'tabby' } },
    ])
    assert.deepEqual(calls.loadKeyPassword, [hashOf(FILE_CONTENT[TARGET_KEY])], 'the hash identifies the key by content')
    assert.deepEqual(calls.launch[0].target.privateKey, { content: 'converted first', passphrase: 'tabby' })
})

await test('a rejected key is dropped so the next one can be offered', async () => {
    const target = profile({ privateKeys: [TARGET_KEY, LATER_KEY] })
    const { service, calls } = createService({
        detectedPath: DETECTED,
        convertResults: [null, { content: 'converted second', passphrase: 'tabby' }],
    })
    await service.launchWinSCP(session(target, null, true))
    assert.deepEqual(calls.retrieved, [TARGET_KEY, LATER_KEY])
    assert.deepEqual(calls.convert.map(call => call.key.content), [FILE_CONTENT[TARGET_KEY], FILE_CONTENT[LATER_KEY]])
    assert.deepEqual(calls.launch[0].target.privateKey, { content: 'converted second', passphrase: 'tabby' })
})

await test('a stored empty passphrase stays empty and a missing one defaults to tabby', async () => {
    const target = profile({ privateKeys: [TARGET_KEY] })
    const empty = createService({
        detectedPath: DETECTED,
        convertResults: [{ content: 'converted', passphrase: '' }],
        passphrases: { [hashOf(FILE_CONTENT[TARGET_KEY])]: '' },
    })
    await empty.service.launchWinSCP(session(target, null, true))
    assert.equal(empty.calls.convert[0].key.passphrase, '', 'an empty stored passphrase is not replaced')

    const missing = createService({ detectedPath: DETECTED, convertResults: [{ content: 'converted', passphrase: 'tabby' }] })
    await missing.service.launchWinSCP(session(target, null, true))
    assert.equal(missing.calls.convert[0].key.passphrase, 'tabby', 'a missing passphrase falls back to the default')
})

await test('a session that did not authenticate with a key sends none', async () => {
    const target = profile({ privateKeys: [TARGET_KEY] })
    const { service, calls } = createService({ detectedPath: DETECTED, convertResults: [{ content: 'converted' }] })
    await service.launchWinSCP(session(target, null, false))
    assert.deepEqual(calls.retrieved, [])
    assert.equal(calls.launch[0].target.privateKey, null)
})

await test('WinSCP still launches when every key is rejected', async () => {
    const target = profile({ privateKeys: [TARGET_KEY, LATER_KEY] })
    const { service, calls } = createService({ detectedPath: DETECTED, convertResults: [null, null] })
    await service.launchWinSCP(session(target, null, true))
    assert.deepEqual(calls.retrieved, [TARGET_KEY, LATER_KEY])
    assert.equal(calls.launch[0].target.privateKey, null, 'the session opens interactively instead of failing')
})

await test('the jump host is prepared before the target key and only for its own auth mode', async () => {
    const jumpProfile = { id: 'jump', type: 'ssh', options: {
        host: 'jump.example.com', port: 2022, user: 'jump-user', auth: 'publicKey', privateKeys: [JUMP_KEY], jumpHost: null,
    } }
    const target = profile({ jumpHost: 'jump', privateKeys: [TARGET_KEY] })
    const { service, calls } = createService({
        detectedPath: DETECTED,
        profiles: [target, jumpProfile],
        passwords: { jump: 'jump-secret' },
        convertResults: [{ content: 'converted jump', passphrase: 'tabby' }, { content: 'converted target', passphrase: 'tabby' }],
    })
    await service.launchWinSCP(session(target, null, true))
    assert.deepEqual(calls.retrieved, [JUMP_KEY, TARGET_KEY], 'the jump key is converted before the target key')
    assert.deepEqual(calls.loadPassword, [{ profile: 'target', username: null }], 'a publicKey jump host has no password to read')
    assert.deepEqual(calls.launch[0].jump, {
        host: 'jump.example.com',
        port: 2022,
        username: 'jump-user',
        password: null,
        privateKey: { content: 'converted jump', passphrase: 'tabby' },
    })
})

await test('a password jump host reads its password and offers no key', async () => {
    const jumpProfile = { id: 'jump', type: 'ssh', options: {
        host: 'jump.example.com', port: null, user: 'jump-user', auth: 'password', privateKeys: [JUMP_KEY], jumpHost: null,
    } }
    const { service, calls } = createService({
        detectedPath: DETECTED,
        profiles: [profile({ jumpHost: 'jump' }), jumpProfile],
        passwords: { jump: 'jump-secret' },
    })
    await service.launchWinSCP(session(profile({ jumpHost: 'jump' })))
    assert.deepEqual(calls.loadPassword, [
        { profile: 'target', username: null },
        { profile: 'jump', username: null },
    ])
    assert.deepEqual(calls.launch[0].jump, {
        host: 'jump.example.com', port: 22, username: 'jump-user', password: 'jump-secret', privateKey: null,
    })
})

await test('a jump host that is not in the profile list is skipped', async () => {
    const { service, calls } = createService({ detectedPath: DETECTED })
    await service.launchWinSCP(session(profile({ jumpHost: 'missing' })))
    assert.equal(calls.launch[0].jump, null)
})

await test('a file that cannot be read aborts the launch', async () => {
    const target = profile({ privateKeys: ['file://gone'] })
    const { service, calls } = createService({ detectedPath: DETECTED })
    await assert.rejects(() => service.launchWinSCP(session(target, null, true)), /unavailable/)
    assert.deepEqual(calls.launch, [])
})

await test('native key paths use the filesystem provider and existing provider references stay intact', async () => {
    for (const ref of ['C:\\Users\\alice\\.ssh\\id_ed25519', '\\\\server\\share\\key', './key', 'file://already-selected', 'vault://stored-key', 'plugin://stored-key']) {
        const expected = ref.includes('://') ? ref : `file://${ref}`
        FILE_CONTENT[expected] = 'native key material\n'
        try {
            const { service, calls } = createService({
                detectedPath: DETECTED,
                convertResults: [{ content: 'converted native key', passphrase: 'tabby' }],
            })
            await service.launchWinSCP(session(profile({ privateKeys: [ref] }), null, true))
            assert.deepEqual(calls.retrieved, [expected])
            assert.equal(calls.convert[0].key.content, FILE_CONTENT[expected])
            assert.equal(calls.launch[0].target.privateKey.content, 'converted native key')
        } finally {
            delete FILE_CONTENT[expected]
        }
    }
})

await test('an IPC failure aborts the launch and propagates', async () => {
    const target = profile({ privateKeys: [TARGET_KEY] })
    const { service, calls } = createService({ detectedPath: DETECTED, convertResults: [new Error('bridge is down')] })
    await assert.rejects(() => service.launchWinSCP(session(target, null, true)), /bridge is down/)
    assert.deepEqual(calls.launch, [])

    const failing = createService({ detectedPath: DETECTED, launchError: new Error('WinSCP launch failed') })
    await assert.rejects(() => failing.service.launchWinSCP(session(profile())), /WinSCP launch failed/)
})

for (const { name, ok, error } of results) {
    console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}`)
    if (!ok) {
        console.log(`     ${String(error.message).split('\n').join('\n     ')}`)
    }
}
const failures = results.filter(result => !result.ok)
console.log(`Tauri WinSCP service: ${results.length - failures.length}/${results.length} passed`)
if (failures.length) {
    process.exitCode = 1
}
