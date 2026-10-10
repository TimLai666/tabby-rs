import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'
import { Subject } from 'rxjs'
import ts from 'typescript'
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
function run(source, globals={}) {
    const module={exports:{}}
    const code=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2020,module:ts.ModuleKind.CommonJS}}).outputText
    vm.runInNewContext(code,{module,exports:module.exports,require:id=>{assert.equal(id,'rxjs');return {Subject}},...globals})
    return module.exports
}
function classText(source, name) {
    const tree=ts.createSourceFile('source.ts',source,ts.ScriptTarget.Latest,true)
    const cls=tree.statements.find(node=>ts.isClassDeclaration(node)&&node.name?.text===name)
    assert.ok(cls,`${name} must exist`)
    return ts.createPrinter().printNode(ts.EmitHint.Unspecified,cls,tree)
}
const core=run(fs.readFileSync(path.join(root,'tabby-core/src/api/platform.ts'),'utf8'))
const nativeSource=fs.readFileSync(path.join(root,'tabby-tauri/src/services/platform.service.ts'),'utf8')
const originalSource=execFileSync('git',['show','14e2d60:tabby-electron/src/services/platform.service.ts'],{cwd:root,encoding:'utf8'})
const Native=run(classText(nativeSource,'TauriDirectoryDownload')+'; module.exports=TauriDirectoryDownload',{DirectoryDownload:core.DirectoryDownload})
const Original=run(classText(originalSource,'ElectronDirectoryDownload')+'; module.exports=ElectronDirectoryDownload',{DirectoryDownload:core.DirectoryDownload})
const original=new Original('/tmp','folder',15,{powerSaveBlocker:{start:()=>0,stop:()=>{}}},{})
const native=new Native({},'/tmp','folder',15)
for(const transfer of [original,native]) {transfer.setCompleted(true);await transfer.closeAsync()}
assert.equal(original.getState(),'completed');assert.equal(original.isCancelled(),false)
assert.equal(native.getState(),original.getState(),'completion cleanup must not replace Completed with Cancelled')
assert.equal(native.isCancelled(),false)
const cancelled=new Native({},'/tmp','folder',15);cancelled.cancel();await cancelled.closeAsync()
assert.equal(cancelled.getState(),'cancelled');assert.equal(cancelled.isCancelled(),true)
console.log('Directory transfer completion and cancellation match fixed upstream: PASS; transfer lifecycle is real, filesystem is not exercised')
