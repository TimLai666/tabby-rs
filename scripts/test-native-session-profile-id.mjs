import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

// Quick-connect profiles (SSH, Telnet, serial) are never saved, so they keep the
// core default id ''. The native managers reject an empty profile id, which made
// every quick connection fail with "SSH target is invalid". Each native session
// must send a non-empty id: the saved profile id, or one derived from its own
// connection id when the profile is unsaved.
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = file => fs.readFileSync(path.join(root, file), 'utf8')

const helperFile = 'tabby-tauri/src/nativeProfileId.ts'
const js = ts.transpileModule(source(helperFile), {
    compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS },
}).outputText
const module = { exports: {} }
vm.runInNewContext(js, { module, exports: module.exports })
const { nativeProfileId } = module.exports

assert.equal(nativeProfileId({ id: 'ssh:custom:office:1' }, 'conn-1'), 'ssh:custom:office:1')
for (const id of ['', '   ', undefined, null]) {
    const value = nativeProfileId({ id }, 'conn-1')
    assert.ok(value.trim().length > 0, `${JSON.stringify(id)} must map to a non-empty id`)
    assert.ok(value.includes('conn-1'), 'an unsaved id must be scoped to its connection')
}
assert.notEqual(nativeProfileId({ id: '' }, 'conn-1'), nativeProfileId({ id: '' }, 'conn-2'),
    'two unsaved quick connections must not share an id')

for (const file of [
    'tabby-tauri/src/ssh/session.ts',
    'tabby-tauri/src/telnet/session.ts',
    'tabby-tauri/src/serial/session.ts',
]) {
    const text = source(file)
    assert.match(text, /profileId: nativeProfileId\(this\.profile, this\.connectionId\)/,
        `${file} must send nativeProfileId(...) to the native manager`)
    assert.doesNotMatch(text, /profileId: this\.profile\.id\b/, `${file} must not send the raw profile id`)
}

console.log('native session profile id: unsaved quick-connect profiles get a connection-scoped id')
