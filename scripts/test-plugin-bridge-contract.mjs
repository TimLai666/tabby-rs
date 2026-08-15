import assert from 'node:assert/strict'
import Module from 'node:module'
import path from 'node:path'
import fs from 'node:fs'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourcePath = path.join(root, 'tabby-tauri/src/services/tauriHostBridge.service.ts')
const source = fs.readFileSync(sourcePath, 'utf8')
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        experimentalDecorators: true,
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2019,
    },
    fileName: sourcePath,
}).outputText

const angularCore = {
    Injectable: () => target => target,
}
const HostBridge = class {}
const serviceModule = new Module(sourcePath)
serviceModule.filename = sourcePath
serviceModule.paths = Module._nodeModulePaths(root)
serviceModule.require = request => {
    if (request === '@angular/core') {
        return angularCore
    }
    if (request === '../api/hostBridge') {
        return { HostBridge }
    }
    return Module._load(request, serviceModule)
}
serviceModule._compile(compiled, sourcePath)

const { TauriHostBridge } = serviceModule.exports
const calls = []
globalThis.window = {
    __TAURI__: {
        core: {
            async invoke (command, args) {
                calls.push({ command, args })
                return {
                    packageName: args.request.packageName,
                    entry: 'dist/index.js',
                    code: 'module.exports = {}',
                }
            },
        },
        event: {
            listen: async () => () => {},
        },
    },
}

const bridge = new TauriHostBridge()
const sourceResult = await bridge.invoke('plugins.readEntry', { packageName: 'tabby-clippy' })

assert.deepEqual(sourceResult, {
    packageName: 'tabby-clippy',
    entry: 'dist/index.js',
    code: 'module.exports = {}',
})
assert.deepEqual(calls, [
    {
        command: 'plugins_read_entry',
        args: { request: { packageName: 'tabby-clippy' } },
    },
])

console.log('Plugin Tauri bridge contract passed')
