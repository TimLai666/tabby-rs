import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
import { Subject } from 'rxjs'
import ts from 'typescript'
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..')
const code=ts.transpileModule(fs.readFileSync(path.join(root,'tabby-tauri/src/ssh/sftpEditor.ts'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,experimentalDecorators:true}}).outputText
function fixture(options={}) {
    let now=0,seq=0,handler,closed=false;const tasks=new Map(),calls=[],notices=[],ended=new Subject()
    const timeout=(fn,delay)=>{const id=++seq;tasks.set(id,{fn,time:now+delay});return id}
    const flush=async()=>{for(let i=0;i<100;i++)await Promise.resolve()}
    const advance=async ms=>{now+=ms;for(const [id,task] of [...tasks])if(task.time<=now){tasks.delete(id);task.fn()}await flush()}
    const module={exports:{}}
    vm.runInNewContext(code,{module,exports:module.exports,require:id=>id==='@angular/core'?{Injectable:()=>x=>x}:{},setTimeout:timeout,clearTimeout:id=>tasks.delete(id),Error,console})
    const bridge={
        invoke:async(c,r)=>{calls.push([c,r]);if(options.invoke)return options.invoke(c,r)},
        listen:async(event,fn)=>{calls.push(['listen',event]);handler=fn;if(options.listen)await options.listen();return()=>calls.push(['unlisten'])},
    }
    const transfer={cancel:()=>calls.push(['cancel-download'])}
    const upload={cancel:()=>calls.push(['cancel-upload'])}
    const platform={prepareEditableFile:async(...args)=>{calls.push(['prepare',...args]);if(options.prepare)await options.prepare();return {id:'owned',path:'/owned/note.bin',transfer}},startUpload:async(...args)=>{calls.push(['start-upload',...args]);return [upload]}}
    const sftp={closed$:ended,isClosed:()=>closed,download:async(...args)=>{calls.push(['download',...args]);if(options.download)await options.download()},upload:async(...args)=>{calls.push(['upload',...args]);if(options.upload)await options.upload()},chmod:async(...args)=>{calls.push(['chmod',...args]);if(options.chmod)await options.chmod()}}
    const editor=new module.exports.TauriSftpEditor(bridge,platform,{error:text=>notices.push(text)})
    const item={name:'note.bin',fullPath:'/data/note.bin',mode:0o100640,size:4,isDirectory:false,isSymlink:false,modified:new Date(0),isOperable:true}
    return {editor,item,sftp,calls,notices,advance,flush,event:type=>{assert.ok(handler,'watch handler must be installed');handler({id:'owned',event:type})},close:()=>{closed=true;ended.next();ended.complete()},tasks}
}
const failures=[];let count=0
async function check(name,test){count++;try{await test();console.log(name+': PASS')}catch(e){failures.push(name);console.error(name+': FAIL: '+e.message)}}
await check('download/open/ready precede the delayed watcher and debounced overwrite with mode restoration',async()=>{
    const f=fixture();await f.editor.edit(f.item,f.sftp)
    assert.deepEqual(f.calls.slice(0,4).map(x=>x[0]),['prepare','download','fileEdit.ready','desktop.openPath'])
    await f.advance(999);assert.equal(f.calls.some(x=>x[0]==='listen'),false)
    await f.advance(1);assert.ok(f.calls.some(x=>x[0]==='fileEdit.watch'))
    f.event('change');await f.advance(999);assert.equal(f.calls.some(x=>x[0]==='upload'),false)
    f.event('change');await f.advance(1000)
    const start=f.calls.find(x=>x[0]==='start-upload');assert.deepEqual(Array.from(start[2]),['/owned/note.bin'])
    const upload=f.calls.find(x=>x[0]==='upload');assert.equal(upload[1],'/data/note.bin');assert.equal(upload[3],'overwrite')
    assert.deepEqual(f.calls.find(x=>x[0]==='chmod'),['chmod','/data/note.bin',0o100640])
    f.close();await f.flush();assert.ok(f.calls.some(x=>x[0]==='fileEdit.stop'));assert.ok(f.calls.some(x=>x[0]==='unlisten'));assert.equal(f.tasks.size,0)
})
await check('session close before arming starts no watch and no upload',async()=>{
    const f=fixture();await f.editor.edit(f.item,f.sftp);f.close();await f.advance(2000)
    assert.equal(f.calls.some(x=>x[0]==='listen'),false);assert.equal(f.calls.some(x=>x[0]==='upload'),false)
})
await check('close while native event subscription is pending removes the eventual listener',async()=>{
    let release;const waiting=new Promise(resolve=>release=resolve);const f=fixture({listen:()=>waiting})
    await f.editor.edit(f.item,f.sftp);await f.advance(1000);f.close();release();await f.flush()
    assert.ok(f.calls.some(x=>x[0]==='unlisten'));assert.equal(f.calls.some(x=>x[0]==='fileEdit.watch'),false)
})
await check('close while temp preparation is pending cancels its transfer and stops context',async()=>{
    let release;const waiting=new Promise(resolve=>release=resolve);const f=fixture({prepare:()=>waiting})
    const edit=f.editor.edit(f.item,f.sftp);f.close();release();await edit;await f.flush()
    assert.ok(f.calls.some(x=>x[0]==='fileEdit.stop'));assert.ok(f.calls.some(x=>x[0]==='cancel-download'))
    assert.equal(f.calls.some(x=>x[0]==='desktop.openPath'),false)
})
await check('rename sends its final upload once and closes the watch',async()=>{
    const f=fixture();await f.editor.edit(f.item,f.sftp);await f.advance(1000);f.event('rename');await f.advance(1000)
    assert.equal(f.calls.filter(x=>x[0]==='upload').length,1);assert.equal(f.calls.filter(x=>x[0]==='chmod').length,1)
    assert.ok(f.calls.some(x=>x[0]==='unlisten'));assert.ok(f.calls.some(x=>x[0]==='fileEdit.stop'));f.close()
})
await check('repeated saves serialize whole uploads and permission restoration',async()=>{
    let release;const waiting=new Promise(resolve=>release=resolve);let n=0;const f=fixture({upload:async()=>{if(++n===1)await waiting}})
    await f.editor.edit(f.item,f.sftp);await f.advance(1000);f.event('change');await f.advance(1000)
    f.event('change');await f.advance(1000);assert.equal(n,1);release();await f.flush()
    assert.equal(n,2);assert.deepEqual(f.calls.filter(x=>['upload','chmod'].includes(x[0])).map(x=>x[0]),['upload','chmod','upload','chmod']);f.close()
})
await check('failed download cancels the incomplete copy and never opens an editor',async()=>{
    const f=fixture({download:async()=>{throw new Error('download denied')}});await f.editor.edit(f.item,f.sftp);await f.flush()
    assert.deepEqual(f.notices,['download denied']);assert.ok(f.calls.some(x=>x[0]==='cancel-download'));assert.ok(f.calls.some(x=>x[0]==='fileEdit.stop'))
    assert.equal(f.calls.some(x=>x[0]==='desktop.openPath'),false);assert.equal(f.tasks.size,0)
})
await check('failed native watch notifies and releases the listener/context',async()=>{
    const f=fixture({invoke:async c=>{if(c==='fileEdit.watch')throw {details:'watch denied'}}})
    await f.editor.edit(f.item,f.sftp);await f.advance(1000)
    assert.deepEqual(f.notices,['watch denied']);assert.ok(f.calls.some(x=>x[0]==='unlisten'));assert.ok(f.calls.some(x=>x[0]==='fileEdit.stop'))
})
await check('closed session after active upload stops queued saves and skips chmod',async()=>{
    let release;const waiting=new Promise(resolve=>release=resolve);const f=fixture({upload:()=>waiting})
    await f.editor.edit(f.item,f.sftp);await f.advance(1000);f.event('change');await f.advance(1000);f.event('change');await f.advance(1000)
    f.close();release();await f.flush();assert.equal(f.calls.filter(x=>x[0]==='upload').length,1);assert.equal(f.calls.some(x=>x[0]==='chmod'),false)
})
console.log(`SFTP local editing: ${count-failures.length} passed; ${failures.length} failed; timers, editor and IPC are simulated`)
assert.equal(failures.length,0,failures.join('; '))
