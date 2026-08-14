import assert from 'node:assert/strict'
import fs from 'node:fs'
import Module from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(
    path.join(root, 'tabby-tauri/src/components/diagnosticsSettingsTab.component.ts'),
    'utf8',
)
const service = fs.readFileSync(
    path.join(root, 'tabby-tauri/src/services/diagnostics.service.ts'),
    'utf8',
)
const template = fs.readFileSync(
    path.join(root, 'tabby-tauri/src/components/diagnosticsSettingsTab.component.pug'),
    'utf8',
)
const servicePath = path.join(root, 'tabby-tauri/src/services/diagnostics.service.ts')
const exportMethod = source.match(/async exportBundle \(\): Promise<void> \{([\s\S]*?)\n    \}/)?.[1]

assert.ok(exportMethod, 'diagnostics export method is missing')
assert.doesNotMatch(exportMethod, /loadPreview\(/, 'export must not bypass the preview step')
assert.match(exportMethod, /if \(!this\.preview\)/)
assert.match(exportMethod, /Preview diagnostics before exporting\./)
assert.match(exportMethod, /return/)
assert.match(exportMethod, /this\.diagnostics\.exportBundle\(\)/)
assert.match(service, /showMessageBox\(\{[\s\S]*?Export the reviewed diagnostic files\?/)
assert.match(service, /confirmation\.response !== 0/)
assert.match(service, /buttons: \[[\s\S]*?translate\.instant\('Export'\)[\s\S]*?translate\.instant\('Cancel'\)/)
assert.match(source, /this\.status = await this\.diagnostics\.status\(\)/)
assert.match(template, /\*ngIf='status\?\.crashMarkerPresent'/)
assert.match(template, /The previous session ended unexpectedly\./)
assert.match(template, /Review the preview before exporting diagnostics\./)

const compiled = ts.transpileModule(service, {
    compilerOptions: {
        experimentalDecorators: true,
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2019,
    },
    fileName: servicePath,
}).outputText
const originalLoad = Module._load
Module._load = function (request, parent, isMain) {
    if (request === '@angular/core') {
        return { Injectable: () => target => target }
    }
    if (request === '@ngx-translate/core') {
        return { TranslateService: class {} }
    }
    if (request === 'tabby-core') {
        return { PlatformService: class {} }
    }
    return originalLoad.call(this, request, parent, isMain)
}

const serviceModule = new Module(servicePath)
serviceModule.filename = servicePath
serviceModule.paths = Module._nodeModulePaths(root)
try {
    serviceModule._compile(compiled, servicePath)
} finally {
    Module._load = originalLoad
}

const invokeCalls = []
let saveDestination = null
const bridge = {
    async invoke (command, request) {
        invokeCalls.push([command, request])
        if (command === 'dialog.save') {
            return saveDestination
        }
        return command === 'diagnostics.export' ? '/tmp/tabby-rs-diagnostics.zip' : null
    },
}
const confirmationCalls = []
const platform = {
    async showMessageBox (options) {
        confirmationCalls.push(options)
        return { response: platformResponse }
    },
}
const translate = { instant: value => value }
let platformResponse = 1
const diagnosticsService = new serviceModule.exports.TauriDiagnosticsService(bridge, platform, translate)

assert.equal(await diagnosticsService.exportBundle(), null)
assert.deepEqual(invokeCalls, [['dialog.save', {
    title: 'Export diagnostics',
    fileName: 'tabby-rs-diagnostics.zip',
}]])
assert.equal(confirmationCalls.length, 0)

invokeCalls.length = 0
saveDestination = '/tmp/tabby-rs-diagnostics.zip'
platformResponse = 1
assert.equal(await diagnosticsService.exportBundle(), null)
assert.equal(invokeCalls.length, 1)
assert.equal(invokeCalls[0][0], 'dialog.save')
assert.equal(confirmationCalls.length, 1)

invokeCalls.length = 0
platformResponse = 0
assert.equal(await diagnosticsService.exportBundle({ includeLogs: false }), '/tmp/tabby-rs-diagnostics.zip')
assert.deepEqual(invokeCalls[1], ['diagnostics.export', {
    destination: '/tmp/tabby-rs-diagnostics.zip',
    includeLogs: false,
}])

console.log('Diagnostics UI preview and crash marker contract passed')
