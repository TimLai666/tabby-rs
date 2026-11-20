import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')

// The real tabby-tauri/src/nativeProfileId.ts module, for fixtures that load a
// native session with a fake `require` map ('../nativeProfileId').
export function loadNativeProfileId () {
    const source = fs.readFileSync(path.join(root, 'tabby-tauri/src/nativeProfileId.ts'), 'utf8')
    const compiled = ts.transpileModule(source, {
        compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS },
    }).outputText
    const module = { exports: {} }
    vm.runInNewContext(compiled, { module, exports: module.exports })
    return module.exports
}
