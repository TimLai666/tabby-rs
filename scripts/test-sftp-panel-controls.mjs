import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'
import constants from 'node:constants'
import { createRequire } from 'node:module'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
const stage = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(stage, 'package.json'))
const ts = require('typescript')
const { Subject } = require('rxjs')
class EventEmitter extends Subject { emit(value) { this.next(value) } }
const metadata = new Map()
const decorators = new Proxy({ EventEmitter, Component: meta => target => { metadata.set(target, meta) } }, { get: (target, key) => target[key] ?? (() => () => {}) })
const cache = new Map()
const notices=[]
const core = { BaseComponent: class { destroyed$ = new Subject() }, NotificationsService: class {}, PlatformService: class {}, TranslateService: class {}, DirectoryUpload: class {} }
function load(file) {
  file=path.resolve(file)
  if(file.endsWith('.pug')) return require('pug').renderFile(file,{require:id=>fs.readFileSync(path.resolve(path.dirname(file),id),'utf8')})
  if(file.endsWith('.scss'))return fs.readFileSync(file,'utf8')
  if(cache.has(file))return cache.get(file).exports
  const module={exports:{}};cache.set(file,module)
  const code=ts.transpileModule(fs.readFileSync(file,'utf8'),{compilerOptions:{target:ts.ScriptTarget.ES2020,module:ts.ModuleKind.CommonJS,experimentalDecorators:true,esModuleInterop:true}}).outputText
  const get=id=>id==='@angular/core'?decorators:id==='@ng-bootstrap/ng-bootstrap'?{NgbModal:class {},NgbActiveModal:class {}}:id==='tabby-core'?core:id==='path'?{posix:path.posix}:id==='constants'?constants:id.startsWith('.')?load(path.resolve(path.dirname(file),id)+(/\.(pug|scss)$/.test(id)?'':'.ts')):require(id)
  vm.runInNewContext(code,{module,exports:module.exports,require:get,window:{localStorage:{}},console,Date,Error},{filename:file})
  return module.exports
}
const { SFTPPanelComponent }=load(path.join(stage,'tabby-ssh/src/components/sftpPanel.component.ts'))
const entry=(name,isDirectory=false)=>({name,isDirectory,isSymlink:false,fullPath:'/data/'+name,mode:0o100644,size:3,modified:new Date(0)})
const sftp={readdir:async p=>{ if(p==='/missing')throw new Error('missing');return [entry('z.txt'),entry('A.txt'),entry('folder',true)] }}
const panel=new SFTPPanelComponent({}, {error:m=>notices.push(m)}, {}, [])
panel.session={openSFTP:async()=>sftp};panel.path='/data'
const output=[];panel.pathChange.subscribe(p=>output.push(p))
await panel.ngOnInit()
assert.equal(panel.sftp,sftp);assert.deepEqual(Array.from(panel.fileList,x=>x.name),['folder','A.txt','z.txt'])
assert.deepEqual(Array.from(panel.pathSegments,x=>({name:x.name,path:x.path})),[{name:'data',path:'/data'}])
panel.showFilter=true;panel.filterText='a.';panel.onFilterChange();assert.deepEqual(Array.from(panel.filteredFileList,x=>x.name),['A.txt'])
panel.filterText='none';panel.onFilterChange();assert.equal(panel.filteredFileList.length,0)
panel.clearFilter();assert.equal(panel.filteredFileList.length,3);assert.equal(panel.showFilter,false)
await panel.navigate('/missing');await new Promise(resolve=>setTimeout(resolve,0));assert.equal(panel.path,'/data');assert.equal(panel.filteredFileList.length,3);assert.deepEqual(notices,['missing'])
const originalPanelSource=execFileSync('git',['show','14e2d60:tabby-ssh/src/components/sftpPanel.component.ts'],{cwd:stage,encoding:'utf8'})
function modeFormatter(source) {
  const tree=ts.createSourceFile('panel.ts',source,ts.ScriptTarget.Latest,true)
  const component=tree.statements.find(node=>ts.isClassDeclaration(node))
  const method=component.members.find(node=>node.name?.getText(tree)==='getModeString')
  assert.ok(method, 'permission control must use the actual component method')
  return new Function('C','item',method.body.getText(tree).slice(1,-1))
}
const originalMode=modeFormatter(originalPanelSource)
const sharedMode=modeFormatter(fs.readFileSync(path.join(stage,'tabby-ssh/src/components/sftpPanel.controller.ts'),'utf8'))
assert.equal(panel.getModeString(entry('A.txt')),originalMode(constants,entry('A.txt')), 'host constants must match fixed upstream on the same platform')
const unixModes={S_IFDIR:0o40000,S_IRUSR:0o400,S_IWUSR:0o200,S_IXUSR:0o100,S_IRGRP:0o40,S_IWGRP:0o20,S_IXGRP:0o10,S_IROTH:0o4,S_IWOTH:0o2,S_IXOTH:0o1}
const windowsModes={...unixModes,S_IRGRP:undefined,S_IWGRP:undefined,S_IXGRP:undefined,S_IROTH:undefined,S_IWOTH:undefined,S_IXOTH:undefined}
for(const [host,bits,expected] of [['unix',unixModes,'rw-r--r--'],['windows',windowsModes,'rw-------']]) {
  assert.equal(originalMode(bits,entry('A.txt')).trim(),expected, host+' fixed upstream control')
  assert.equal(sharedMode(bits,entry('A.txt')),originalMode(bits,entry('A.txt')), host+' shared permissions match the fixed original')
}
assert.deepEqual(output,['/data','/missing','/data'])
console.log('Shared SFTP controller legacy navigation/filter/fallback/permissions: PASS')

const { TauriSftpPanelComponent }=load(path.join(stage,'tabby-tauri/src/ssh/sftpPanel.component.ts'))
const { SFTPPanelController }=load(path.join(stage,'tabby-ssh/src/components/sftpPanel.controller.ts'))
const nativeSource=fs.readFileSync(path.join(stage,'tabby-tauri/src/ssh/sftpPanel.component.ts'),'utf8')
let moduleMetadata
const moduleExports={exports:{}}
const moduleSymbol=class {}
const moduleDependency=new Proxy({__esModule:true,default:moduleSymbol},{get:(target,key)=>target[key]??moduleSymbol})
const filesizeDependency={NgxFilesizeModule:class {}}
vm.runInNewContext(ts.transpileModule(fs.readFileSync(path.join(stage,'tabby-tauri/src/index.ts'),'utf8'),{
  compilerOptions:{target:ts.ScriptTarget.ES2020,module:ts.ModuleKind.CommonJS,experimentalDecorators:true,esModuleInterop:true},
}).outputText,{
  module:moduleExports,exports:moduleExports.exports,
  require:id=>id==='@angular/core'?{NgModule:meta=>target=>{moduleMetadata=meta}}:id==='ngx-filesize'?filesizeDependency:moduleDependency,
})
assert.ok(moduleMetadata.imports.includes(filesizeDependency.NgxFilesizeModule),'native module must provide the shared file-size pipe')
assert.ok(TauriSftpPanelComponent.prototype instanceof SFTPPanelController, 'native panel must execute the shared controller')
const meta=metadata.get(TauriSftpPanelComponent)
assert.equal(meta.template,load(path.join(stage,'tabby-ssh/src/components/sftpPanel.component.pug')), 'native and original must compile the exact same Pug')
assert.equal(meta.styles[0],load(path.join(stage,'tabby-ssh/src/components/sftpPanel.component.scss')))
assert.ok(!nativeSource.includes('window.prompt('), 'original modal/context menu replace action prompts')
assert.ok(!nativeSource.includes('window.confirm('), 'native message box replaces browser confirmations')
const nativeNotices=[];const calls=[];let confirmed=1;let popped=[]
const platform={setClipboard:value=>calls.push(['clipboard',value.text]),showMessageBox:async options=>{calls.push(['dialog',options]);return {response:confirmed}},popupContextMenu:items=>{popped=items}}
const native=new TauriSftpPanelComponent(platform,{error:m=>nativeNotices.push(m),notice:m=>nativeNotices.push(m)}, {}, {instant:text=>text})
native.path='/data';native.sftp={remove:async(...args)=>calls.push(['remove',...args])};native.navigate=async p=>calls.push(['navigate',p])
const file={...entry('note.txt'),isOperable:true};const directory={...entry('folder',true),isOperable:true}
const menus=await native.buildContextMenu(file)
assert.deepEqual(Array.from(menus.filter(x=>x.label),x=>x.label),['Copy full path','Create directory','Download','Delete'])
await menus.find(x=>x.label==='Copy full path').click();assert.deepEqual(calls.shift(),['clipboard','/data/note.txt'])
await menus.find(x=>x.label==='Delete').click();assert.equal(calls.filter(x=>x[0]==='remove').length,0);assert.equal(calls.filter(x=>x[0]==='navigate').length,0)
assert.equal(calls[0][1].cancelId,1);assert.equal(calls[0][1].buttons[1],'Cancel')
let prevented=0;await native.showContextMenu(directory,{preventDefault:()=>prevented++});assert.equal(prevented,1);assert.ok(popped.some(x=>x.label==='Download directory'))
await native.showContextMenu({...file,isOperable:false,unoperableReason:'display-only'},{preventDefault:()=>prevented++});assert.equal(nativeNotices.at(-1),'display-only')
console.log('Native SFTP shared template/controller/context menu/cancel/display-only controls: PASS; UI boundaries are simulated')

assert.equal(panel.getDisplayName(file), file.name, 'legacy file label preserves the remote name')
assert.equal(native.getDisplayName({...file,isOperable:false}), 'note.txt (display-only)')
assert.equal(native.getDisplayName(file), 'note.txt')
const downloadError = new Error('download denied')
platform.startDownload = async () => ({})
native.sftp.download = async () => { throw downloadError }
await native.download(file)
assert.equal(nativeNotices.at(-1), 'download denied', 'file download failure is reported without rejecting the click handler')
console.log('Native SFTP display-only labels and file download error reporting: PASS')

const { TauriSftpDeleteModalComponent }=load(path.join(stage,'tabby-tauri/src/ssh/sftpDeleteModal.component.ts'))
for (const scenario of ['cancel-pending','partial-failure','navigate-away']) {
  let current;let start;let finish;const removed=[];const refresh=[];const completion=[]
  const started=new Promise(resolve=>{start=resolve})
  const pending=new Promise(resolve=>{finish=resolve})
  const modalHost={open:()=>{
    let close;let dismiss;let componentVisible=true
    const result=new Promise((resolve,reject)=>{
      close=value=>{componentVisible=false;resolve(value)}
      dismiss=error=>{componentVisible=false;reject(error)}
    })
    current=new TauriSftpDeleteModalComponent({close,dismiss})
    queueMicrotask(()=>current.ngOnInit().then(()=>completion.push('settled')))
    return {get componentInstance(){return componentVisible?current:undefined},result}
  }}
  const target=new TauriSftpPanelComponent({...platform,showMessageBox:async()=>({response:0})},{error:m=>nativeNotices.push(m)},modalHost,{instant:text=>text})
  target.path='/data';target.navigate=async p=>refresh.push(p)
  target.sftp={
    readdir:async()=>[{...file,fullPath:'/data/folder/a.txt'},{...file,fullPath:'/data/folder/b.txt'}],
    remove:async p=>{removed.push(p);if(removed.length===1){start();await pending}else if(scenario==='partial-failure'){throw new Error('second removal denied')}},
  }
  const action=(await target.buildContextMenu(directory)).find(x=>x.label==='Delete').click()
  await started
  if(scenario!=='partial-failure'){current.cancel()}
  if(scenario==='navigate-away'){target.path='/elsewhere'}
  await new Promise(resolve=>setTimeout(resolve,0));assert.deepEqual(refresh,[], 'refresh must await the pending remote operation')
  finish();await action
  assert.deepEqual(refresh,scenario==='navigate-away'?[]:['/data'])
  assert.equal(current.changed,true)
  assert.deepEqual(removed,scenario==='partial-failure'?['/data/folder/a.txt','/data/folder/b.txt']:['/data/folder/a.txt'])
  if(scenario==='partial-failure')assert.equal(nativeNotices.at(-1),'second removal denied')
}
console.log('Native SFTP deletion refresh after cancellation/partial failure and navigation race: 3 passed')
