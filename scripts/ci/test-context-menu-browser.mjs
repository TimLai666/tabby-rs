#!/usr/bin/env node
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises'
import { createRequire } from 'node:module'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const require = createRequire(import.meta.url)
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const output = await mkdtemp(path.join(os.tmpdir(), 'tabby-menu-browser-'))
const compiled = ts.transpileModule(await readFile(path.join(root, 'tabby-tauri/src/services/platform.service.ts'), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, experimentalDecorators: true },
}).outputText
await writeFile(path.join(output, 'provider.js'), `(() => {
 const exports = {};
 const require = name => {
  if (name === '@angular/core') return { Injectable: () => target => target, Inject: () => () => {} };
  if (name === 'tabby-core') return { PlatformService: class {}, FileUpload: class {}, FileDownload: class {}, DirectoryDownload: class {} };
  if (name === '../api/hostBridge') return {};
  throw new Error('Unexpected provider dependency: ' + name);
 };
 ${compiled}
 window.menuProvider = Object.create(exports.TauriPlatformService.prototype);
 window.menuProvider.runtimeInfo = { platform: 'windows' };
 window.menuProvider.zone = { run: fn => fn() };
})();`)
await writeFile(path.join(output, 'fixture.js'), await readFile(path.join(root, 'scripts/ci/context-menu-browser/fixture.js')))
await writeFile(path.join(output, 'index.html'), `<!doctype html><html><head><meta charset="utf-8"><title>Tabby context menu parity fixture</title>
<style>body{font:14px system-ui;background:#202124;color:#eee;--bs-body-bg:#202124;--bs-border-color:#555;--bs-primary:#375a9e}#open-menu{position:fixed;right:12px;bottom:12px;padding:12px}#result{margin:24px}</style></head>
<body><h1>Tabby context menu parity fixture</h1><p id="result">Isolated production renderer. Actions record a choice only.</p><button id="open-menu">Open test menu</button><script>window.fixtureErrors=[];addEventListener('error',e=>fixtureErrors.push(e.message));addEventListener('unhandledrejection',e=>fixtureErrors.push(String(e.reason)));</script><script src="provider.js"></script><script src="fixture.js"></script></body></html>`)
if (process.argv.includes('--prepare')) {
    console.log(output)
} else {
    try {
        const main = path.join(output, 'main.cjs')
        await writeFile(main, `const {app,BrowserWindow}=require('electron');
app.whenReady().then(async()=>{const w=new BrowserWindow({show:false,width:800,height:600,webPreferences:{contextIsolation:true}});try{
await w.loadFile(${JSON.stringify(path.join(output, 'index.html'))});
const result=await w.webContents.executeJavaScript('runMenuChecks()');
const errors=await w.webContents.executeJavaScript('fixtureErrors');
if(errors.length){result.ok=false;result.failures.push(...errors)}
console.log('MENU_RESULT:'+JSON.stringify(result));process.exitCode=result.ok?0:1;
}catch(e){console.error(e);process.exitCode=1}finally{w.destroy();app.quit()}});`)
        const electron = require('electron')
        const command = process.platform === 'linux' ? 'xvfb-run' : electron
        const args = process.platform === 'linux' ? ['-a', electron, '--no-sandbox', main] : ['--no-sandbox', main]
        const result = await new Promise((resolve, reject) => {
            const child = spawn(command, args, { env: { ...process.env, ELECTRON_DISABLE_SECURITY_WARNINGS: 'true' } })
            let stdout = '', stderr = ''
            const timer = setTimeout(() => { child.kill(); reject(new Error('Context menu fixture timed out')) }, 30000)
            child.stdout.on('data', chunk => { stdout += chunk })
            child.stderr.on('data', chunk => { stderr += chunk })
            child.on('error', error => { clearTimeout(timer); reject(error) })
            child.on('close', code => { clearTimeout(timer); resolve({ code, stdout, stderr }) })
        })
        const line = result.stdout.split('\n').find(line => line.startsWith('MENU_RESULT:'))
        assert.ok(line, result.stderr || 'No context menu fixture result')
        const payload = JSON.parse(line.slice('MENU_RESULT:'.length))
        console.log(JSON.stringify(payload, null, 2))
        assert.equal(result.code, 0, payload.failures.join('\n'))
        assert.equal(payload.ok, true)
    } finally {
        await rm(output, { recursive: true, force: true })
    }
}
