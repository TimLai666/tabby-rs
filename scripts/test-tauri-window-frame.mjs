import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'

const root = process.cwd()
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const loggerWarnings = []
let reloads = 0
const core = {
    PlatformService: class { supportsWindowControls = false },
    FileUpload: class {},
    FileDownload: class {},
    DirectoryDownload: class {},
    HostAppService: class {},
    CLIHandler: class {},
    ConfigService: class {},
    Platform: { macOS: 'macOS', Windows: 'Windows', Linux: 'Linux' },
}
const logger = {
    info () {},
    warn (...args) { loggerWarnings.push(args) },
    error () {},
}
const plain = value => JSON.parse(JSON.stringify(value))

function loadClass (file, dependencies = {}, globals = {}) {
    const module = { exports: {} }
    const source = fs.readFileSync(path.join(root, file), 'utf8')
    const output = ts.transpileModule(source, { compilerOptions: {
        target: ts.ScriptTarget.ES2020,
        module: ts.ModuleKind.CommonJS,
        experimentalDecorators: true,
    } }).outputText
    vm.runInNewContext(output, {
        exports: module.exports,
        module,
        console,
        ...globals,
        require: name => {
            if (name === '@angular/core') return { Injectable: () => x => x, Inject: () => () => {} }
            if (name === 'tabby-core') return core
            if (name in dependencies) return dependencies[name]
            throw new Error('Unprovided dependency: ' + name)
        },
    })
    return module.exports
}

class Subject { next () {} }
core.PlatformService = loadClass('tabby-core/src/api/platform.ts', {
    rxjs: { Subject },
    './fileTransfer': { DirectoryUpload: class {}, HTMLFileUpload: class {} },
}).PlatformService

const { TauriHostAppService } = loadClass('tabby-tauri/src/services/hostApp.service.ts', {
    '../api/hostBridge': {},
}, { window: { location: { reload: () => { reloads++ } } } })
const { TauriPlatformService } = loadClass('tabby-tauri/src/services/platform.service.ts', {
    '../api/hostBridge': {},
    rxjs: { Subject },
}, { window: { matchMedia: () => ({ matches: false }) } })

const settle = async () => {
    for (let i = 0; i < 20; i++) await new Promise(resolve => setImmediate(resolve))
}

async function createHost (platform, startupWindowFrame, { rejectRestart = false } = {}) {
    const calls = []
    let configReads = 0
    const runtimeInfo = { platform, startupWindowFrame, windowsBuild: null }
    const injector = { get: token => {
        if (token === core.ConfigService) {
            configReads++
            throw new Error('native frame must come from startupWindowFrame, not ConfigService')
        }
        if (token === core.CLIHandler) return []
        throw new Error('Unexpected injector token: ' + String(token))
    } }
    const bridge = {
        listen: async () => () => {},
        invoke: async (command, request) => {
            calls.push({ command, request: plain(request) })
            if (command === 'app.initialLaunch') return null
            if (command === 'app.restart' && rejectRestart) throw new Error('fixture restart IPC rejected')
            return null
        },
    }
    const service = new TauriHostAppService(injector, { run: action => action() }, bridge, runtimeInfo)
    service.logger = logger
    await settle()
    return { service, calls, runtimeInfo, get configReads () { return configReads } }
}

for (const [saved, expected] of [
    ['native', 'native'], ['full', 'full'], ['thin', 'thin'],
    [undefined, 'thin'], [null, 'thin'], ['', 'thin'], ['invalid', 'thin'], [[], 'thin'],
]) {
    const host = await createHost('macos', saved)
    assert.equal(host.calls.filter(call => call.command === 'window.applyState').length, 0,
        'the constructor does not send frame before emitReady')
    host.runtimeInfo.startupWindowFrame = 'full'
    host.runtimeInfo.platform = 'linux'
    assert.doesNotThrow(() => host.service.emitReady())
    await settle()
    assert.equal(host.configReads, 0, 'constructor and emitReady do not read ConfigService')
    const readyCalls = host.calls.filter(call => call.command === 'window.applyState')
    assert.deepEqual(readyCalls, [{ command: 'window.applyState', request: { frame: expected, visible: true } }],
        `macOS uses its constructor-time startup snapshot (${String(saved)}) with visibility`)
}

for (const platform of ['windows', 'linux']) {
    const host = await createHost(platform, 'native')
    host.service.emitReady()
    await settle()
    assert.equal(host.configReads, 0)
    assert.deepEqual(host.calls.filter(call => call.command === 'window.applyState'), [
        { command: 'window.applyState', request: { visible: true } },
    ], `${platform} does not send a frame`)
}

const restartedProcess = await createHost('macos', 'full')
restartedProcess.service.emitReady()
await settle()
assert.deepEqual(restartedProcess.calls.filter(call => call.command === 'window.applyState'), [
    { command: 'window.applyState', request: { frame: 'full', visible: true } },
], 'a new process instance captures the updated startup frame')

loggerWarnings.length = 0
reloads = 0
const restart = await createHost('macos', 'thin', { rejectRestart: true })
let unhandledRejection
const onUnhandledRejection = reason => { unhandledRejection = reason }
process.on('unhandledRejection', onUnhandledRejection)
restart.service.relaunch()
await settle()
process.off('unhandledRejection', onUnhandledRejection)
assert.deepEqual(restart.calls.filter(call => call.command === 'app.restart'), [
    { command: 'app.restart', request: {} },
], 'relaunch requests an application restart')
assert.equal(reloads, 0, 'relaunch does not only reload the renderer')
assert.ok(loggerWarnings.some(args => args.some(value => String(value).includes('fixture restart IPC rejected'))),
    'restart IPC rejection is logged')
assert.equal(unhandledRejection, undefined, 'restart IPC rejection is handled')

async function checkPlatformService (platform, expected) {
    const provider = new TauriPlatformService({
        listen: async () => () => {}, invoke: async () => '',
    }, { platform }, { run: action => action() })
    await provider.fileDropReady
    assert.equal(provider.supportsWindowControls, expected, `${platform} frame capability`)
}
await checkPlatformService('macos', true)
await checkPlatformService('windows', false)
await checkPlatformService('linux', false)

const template = fs.readFileSync(path.join(root, 'tabby-settings/src/components/windowSettingsTab.component.pug'), 'utf8')
const frameRow = template.split(/(?=^\.form-line)/m).find(block => block.includes('.title(translate) Window frame'))
assert.ok(frameRow, 'the shared Window frame row remains in the settings template')
assert.match(frameRow, /^\.form-line\(\*ngIf='platform\.supportsWindowControls'\)/)
for (const [id, value, label] of [['frameNative', 'native', 'Native'], ['frameThin', 'thin', 'Thin'], ['frameFull', 'full', 'Full']]) {
    const input = frameRow.split('input.btn-check(').slice(1).find(block => block.includes(`id='${id}'`))
    assert.ok(input, label + ' option is present')
    assert.ok(input.includes(`(ngModelChange)='saveConfiguration(true)'`), `${label} saves and requests restart`)
    assert.ok(input.includes(`[value]='"${value}"'`), `${label} remains the ${value} option`)
}
const settings = fs.readFileSync(path.join(root, 'tabby-settings/src/components/windowSettingsTab.component.ts'), 'utf8')
assert.match(settings, /saveConfiguration \(requireRestart\?: boolean\)\s*\{\s*this\.config\.save\(\)\s*if \(requireRestart\)\s*\{\s*this\.config\.requestRestart\(\)/,
    'frame options preserve the existing config save and restart handler')

const appState = fs.readFileSync(path.join(root, 'src-tauri/src/state.rs'), 'utf8')
const appCommands = fs.readFileSync(path.join(root, 'src-tauri/src/commands/app.rs'), 'utf8')
const appRegistration = fs.readFileSync(path.join(root, 'src-tauri/src/lib.rs'), 'utf8')
const desktop = fs.readFileSync(path.join(root, 'src-tauri/src/desktop.rs'), 'utf8')
const bridge = fs.readFileSync(path.join(root, 'tabby-tauri/src/api/hostBridge.ts'), 'utf8')
const entry = fs.readFileSync(path.join(root, 'app/src/entry.tauri.ts'), 'utf8')
assert.match(appState, /window_frame:\s*window_frame::StartupWindowFrame/)
assert.match(appState, /startup_window_frame\s*\(/)
assert.match(appCommands, /pub struct BootstrapData[\s\S]*?window_frame:\s*String/)
assert.match(appCommands, /let window_frame = state\.startup_window_frame\(&config\)\.to_owned\(\)/,
    'bootstrap derives the cached frame from its original raw config map')
assert.match(appCommands, /Ok\(BootstrapData\s*\{[\s\S]*?window_frame[,\s]/,
    'bootstrap returns the derived process frame')
function frameProperty (interfaceName, propertyName) {
    const ast = ts.createSourceFile('hostBridge.ts', bridge, ts.ScriptTarget.Latest, true)
    const shape = ast.statements.find(node => ts.isInterfaceDeclaration(node) && node.name.text === interfaceName)
    const field = shape?.members.find(node => node.name?.getText(ast) === propertyName)
    assert.ok(field?.questionToken, propertyName + ' remains optional')
    assert.ok(ts.isUnionTypeNode(field.type), propertyName + ' declares supported frame choices')
    assert.deepEqual(field.type.types.map(type => type.literal?.text).sort(), ['full', 'native', 'thin'])
}
frameProperty('RuntimeInfo', 'startupWindowFrame')
assert.match(entry, /startupWindowFrame:\s*bootstrapData\.windowFrame/)
assert.match(appCommands, /pub fn app_restart\s*\([\s\S]*?request_restart\(\)/)
assert.match(appRegistration, /app_restart/)
assert.match(appRegistration, /\.manage\(restart_environment\(tauri::Env::default\(\)\)\)/,
    'the framework restart environment clears already consumed launch arguments')
assert.match(bridge, /'app\.restart':\s*\{\s*request:/)
frameProperty('WindowStatePatch', 'frame')
assert.match(desktop, /pub struct WindowStatePatch\s*\{[\s\S]*?pub frame:\s*Option<String>/)
assert.match(appRegistration, /ExitRequested\s*\{\s*code:\s*None\s*\|\s*Some\(0\)/,
    'ordinary quit confirmation keeps its existing exit-code scope')

console.log('Tauri window frame: startup snapshot, application restart, platform/settings gate, and source wiring passed')
