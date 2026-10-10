import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'
import ts from 'typescript'
import { Subject } from 'rxjs'
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),require=createRequire(import.meta.url),cache=new Map()
const decorators=new Proxy({EventEmitter:Subject},{get:(target,key)=>target[key]??(()=>()=>{})})
function load(file){
    file=path.resolve(file);if(cache.has(file))return cache.get(file).exports
    if(/\.(pug|scss)$/.test(file))return ''
    const module={exports:{}};cache.set(file,module)
    const code=ts.transpileModule(fs.readFileSync(file,'utf8'),{compilerOptions:{target:ts.ScriptTarget.ES2020,module:ts.ModuleKind.CommonJS,experimentalDecorators:true,esModuleInterop:true}}).outputText
    vm.runInNewContext(code,{module,exports:module.exports,require:id=>id==='@angular/core'?decorators:id==='@ng-bootstrap/ng-bootstrap'?{}:id==='tabby-core'?{BaseComponent:class {},DirectoryUpload:class {}}:id.startsWith('.')?load(path.resolve(path.dirname(file),id)+(/\.(pug|scss)$/.test(id)?'':'.ts')):require(id),Date,Error,console})
    return module.exports
}
assert.equal(load(path.join(root,'tabby-ssh/src/api/contextMenu.ts')).SFTPContextMenuItemProvider, load(path.join(root,'tabby-ssh/src/api/sftpContextMenu.ts')).SFTPContextMenuItemProvider, 'original and native providers share one runtime injection token')
const Panel=load(path.join(root,'tabby-tauri/src/ssh/sftpPanel.component.ts')).TauriSftpPanelComponent
const entry=(name,fullPath,size=0,props={})=>({name,fullPath,size,mode:0o100640,isDirectory:false,isSymlink:false,isOperable:true,modified:new Date(0),...props})
const folder=entry('root','/data/root',0,{isDirectory:true})
function fixture(options={}){
    const calls=[],errors=[],totals=[],statuses=[];let state='pending',cancelled=false
    const transfer={setTotalSize:n=>totals.push(n),setStatus:s=>statuses.push(s),setCompleted:()=>{state='completed'},cancel:()=>{cancelled=true;state='cancelled'},isCancelled:()=>cancelled,closeAsync:async()=>calls.push(['closed',state]),createDirectory:async p=>calls.push(['directory',p]),createFile:async(p,mode,size)=>{calls.push(['file',p,mode,size]);return {} }}
    const platform={startDownloadDirectory:async()=>{if(options.pickerError)throw new Error('picker denied');return options.cancelPicker?null:transfer},setClipboard:x=>calls.push(['clipboard',x.text])}
    const providers=[{weight:20,getItems:async(item,panel)=>[{label:'Plugin item',click:()=>calls.push(['plugin',item.fullPath,panel.path])}]}]
    const editor={edit:async(item,sftp)=>calls.push(['edit',item.fullPath,sftp])}
    const panel=new Panel(platform,{error:x=>errors.push(x)}, {},{instant:x=>x},providers,editor)
    panel.path='/data';panel.sftp={readdir:async p=>{calls.push(['list',p]);if(options.listError)throw new Error('list denied');return p==='/data/root'?[entry('a.bin',p+'/a.bin',4),entry('nested',p+'/nested',0,{isDirectory:true}),entry('bad',p+'/bad',99,{isOperable:false})]:[entry('alias.bin',p+'/alias.bin',18,{isSymlink:true,mode:0o120777})]},stat:async(p,follow)=>{calls.push(['stat',p,follow]);return entry('target','/target',3)},download:async p=>calls.push(['download',p])}
    return {panel,calls,errors,totals,statuses,transfer}
}
const failures=[];let checks=0
async function check(name,test){checks++;try{await test();console.log(name+': PASS')}catch(e){failures.push(name);console.error(name+': FAIL: '+e.message)}}
await check('folder download estimates followed link bytes and displays relative child status',async()=>{
    const f=fixture();await f.panel.download(folder)
    assert.equal(f.totals.at(-1),7);assert.deepEqual(f.statuses,['a.bin','nested','nested/alias.bin',''])
    assert.deepEqual(f.calls.filter(x=>x[0]==='file'),[['file','a.bin',0o100640,4],['file','nested/alias.bin',0o120777,3]])
    assert.deepEqual(f.calls.at(-1),['closed','completed']);assert.deepEqual(f.errors,[])
})
await check('folder selection cancellation performs no remote listing',async()=>{
    const f=fixture({cancelPicker:true});await f.panel.download(folder);assert.deepEqual(f.calls,[])
})
await check('directory picker error is reported without a rejected click',async()=>{
    const f=fixture({pickerError:true});await f.panel.download(folder);assert.equal(f.errors.length,1);assert.match(f.errors[0],/picker denied/)
})
await check('listing error cancels and closes without directory/file creation',async()=>{
    const f=fixture({listError:true});await f.panel.download(folder);assert.deepEqual(f.calls.at(-1),['closed','cancelled']);assert.equal(f.errors.length,1)
    assert.equal(f.calls.some(x=>x[0]==='file'||x[0]==='directory'),false)
})
await check('Edit locally and extension sections retain original provider weights',async()=>{
    const f=fixture(),item=entry('note.bin','/data/note.bin',3)
    const menu=await f.panel.buildContextMenu(item)
    assert.deepEqual(Array.from(menu.filter(x=>x.label),x=>x.label),['Copy full path','Edit locally','Create directory','Download','Delete','Plugin item'])
    const sections=[];for(const x of menu){if(x.type==='separator')sections.push('|');else sections.push(x.label)}
    assert.deepEqual(sections,['Copy full path','Edit locally','|','Create directory','Download','Delete','|','Plugin item'])
    await menu.find(x=>x.label==='Edit locally').click();assert.deepEqual(f.calls[0],['edit','/data/note.bin',f.panel.sftp])
    await menu.find(x=>x.label==='Plugin item').click();assert.deepEqual(f.calls[1],['plugin','/data/note.bin','/data'])
    assert.equal((await f.panel.buildContextMenu(folder)).some(x=>x.label==='Edit locally'),false)
})
console.log(`SFTP folder progress/menu: ${checks-failures.length} passed; ${failures.length} failed; filesystem and IPC are simulated`)
assert.equal(failures.length,0,failures.join('; '))
