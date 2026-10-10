import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
import { Subject } from 'rxjs'
import ts from 'typescript'
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..')
const source=file=>fs.readFileSync(path.join(root,file),'utf8')
function cls(text,name) {
    const tree=ts.createSourceFile('source.ts',text,ts.ScriptTarget.Latest,true)
    const node=tree.statements.find(x=>ts.isClassDeclaration(x)&&x.name?.text===name)
    assert.ok(node,`${name} must exist`)
    return ts.createPrinter().printNode(ts.EmitHint.Unspecified,node,tree)
}
function run(text,globals={}) {
    const module={exports:{}}
    vm.runInNewContext(ts.transpileModule(text,{compilerOptions:{target:ts.ScriptTarget.ES2020,module:ts.ModuleKind.CommonJS}}).outputText,
        {module,exports:module.exports,require:id=>{assert.equal(id,'rxjs');return {Subject}},Error,Date,Uint8Array,...globals})
    return module.exports
}
const Session=run(cls(source('tabby-tauri/src/ssh/sftp.ts'),'TauriSftpSession')+';module.exports=TauriSftpSession',{Subject})
const failures=[]
async function check(name,test){try{await test();console.log(name+': PASS')}catch(e){failures.push(name);console.error(name+': FAIL: '+e.message)}}
await check('remote chmod preserves path/mode and rejects after session closure',async()=>{
    const calls=[];const session=await Session.open({invoke:async(c,r)=>{calls.push([c,r]);return {id:'sftp-id'}}},'ssh-id')
    await session.chmod('/data/note.txt',0o100640)
    assert.equal(calls[1][0],'sftp.chmod');assert.equal(calls[1][1].mode,0o100640);assert.equal(calls[1][1].path,'/data/note.txt')
    assert.equal(calls[1][1].id,'sftp-id')
    await session.close();await assert.rejects(session.chmod('/data/note.txt',0o600),/closed/)
    assert.equal(calls.length,3)
})
await check('session closure synchronously stops listeners even when native close fails',async()=>{
    const session=await Session.open({invoke:async c=>{if(c==='sftp.close')throw new Error('close failed');return {id:'sftp-id'}}},'ssh-id')
    let ended=0;session.closed$.subscribe(()=>ended++)
    const closing=session.close();assert.equal(ended,1);assert.equal(session.isClosed(),true)
    await assert.rejects(closing,/close failed/);await session.close();assert.equal(ended,1)
})
const nativeSource=source('tabby-tauri/src/services/platform.service.ts')
function platformFor(calls) {
    const tree=ts.createSourceFile('platform.ts',nativeSource,ts.ScriptTarget.Latest,true)
    const node=tree.statements.find(x=>ts.isClassDeclaration(x)&&x.name?.text==='TauriPlatformService')
    const methods=['startUpload','prepareEditableFile'].map(name=>{
        const method=node.members.find(x=>x.name?.getText(tree)===name);assert.ok(method,`platform.${name} must exist`)
        return ts.createPrinter().printNode(ts.EmitHint.Unspecified,method,tree)
    }).join('\n')
    const core=run(source('tabby-core/src/api/platform.ts'))
    const Platform=run(cls(nativeSource,'TauriFileDownload')+'\n'+cls(nativeSource,'TauriFileUpload')+'\nclass TestedPlatform {'+methods+'}\nmodule.exports=TestedPlatform',core)
    const result=new Platform();result.fileTransferStarted=new Subject()
    result.bridge={invoke:async(c,r)=>{
        calls.push([c,r]);if(c==='dialog.open')throw new Error('editor upload must never open a picker')
        const descriptor={id:'file-transfer',name:'note.txt',size:3};return c==='fileEdit.prepare'?{id:'workspace',path:'/owned/note.txt',transfer:descriptor}:[descriptor]
    }}
    return result
}
await check('local editor upload uses explicit file and emits real streaming transfer',async()=>{
    const calls=[];const platform=platformFor(calls);const started=[];platform.fileTransferStarted.subscribe(t=>started.push(t))
    const transfers=await platform.startUpload({multiple:false},['/owned/note.txt'])
    assert.equal(calls.length,1);assert.equal(calls[0][0],'transfer.openUpload');assert.deepEqual(Array.from(calls[0][1].paths),['/owned/note.txt'])
    assert.equal(transfers[0],started[0]);assert.equal(transfers[0].getSize(),3)
})
await check('editor download reuses streaming handle and transfer notification',async()=>{
    const calls=[];const platform=platformFor(calls);const started=[];platform.fileTransferStarted.subscribe(t=>started.push(t))
    const edit=await platform.prepareEditableFile('note.txt',0o600,3)
    assert.equal(edit.id,'workspace');assert.equal(edit.path,'/owned/note.txt');assert.equal(edit.transfer,started[0])
    assert.equal(edit.transfer.id,'file-transfer');assert.equal(edit.transfer.getSize(),3)
    assert.equal(calls[0][0],'fileEdit.prepare');assert.equal(calls[0][1].mode,0o600)
})
console.log(`SFTP edit transport: ${4-failures.length} passed; ${failures.length} failed; IPC is simulated`)
assert.equal(failures.length,0,failures.join('; '))
