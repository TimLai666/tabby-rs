import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

if (process.argv.length === 3 && ['--help', '-h'].includes(process.argv[2])) {
    console.log('Usage: node scripts/test-window-topmost.mjs [--help]\nChecks the production always-on-top bridge on Linux, macOS, and Windows feature fixtures. Native/Tauri boundaries are simulated. Runs Cargo offline in an owned temporary crate and removes it after the run.')
    process.exit(0)
}
assert.equal(process.argv.length, 2, 'unknown argument; use --help')

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const bridgeFile = path.join(root, 'src-tauri/src/platform/window_topmost.rs')
const linuxFile = path.join(root, 'src-tauri/src/platform/window_topmost_linux.rs')
const lockFile = path.join(root, 'src-tauri/Cargo.lock')
const inputs = [bridgeFile, linuxFile, lockFile]
const fingerprints = new Map(inputs.map(file => [file, crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')]))
const bridge = fs.readFileSync(bridgeFile, 'utf8')
const extract = name => {
    const marker = `pub(crate) fn ${name}(`
    const start = bridge.indexOf(marker)
    assert.ok(start >= 0 && bridge.indexOf(marker, start + 1) < 0, `unique production ${name}`)
    let depth = 0
    let body = bridge.indexOf('{', start)
    for (let i = body; i < bridge.length; i++) {
        if (bridge[i] === '{') depth++
        else if (bridge[i] === '}' && --depth === 0) return bridge.slice(start, i + 1)
    }
    throw new Error(`incomplete production ${name}`)
}
const read = extract('read')
const set = extract('set')
const runnable = source => source.replaceAll('target_os = "linux"', 'feature = "linux"')
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-window-topmost-'))
try {
    fs.mkdirSync(path.join(temporary, 'src'))
    fs.writeFileSync(path.join(temporary, 'Cargo.toml'), '[package]\nname="window-topmost-harness"\nversion="0.1.0"\nedition="2021"\n[features]\ndefault=[]\nlinux=[]\nmacos=[]\nwindows=[]\n')
    fs.writeFileSync(path.join(temporary, 'src/lib.rs'), `#![allow(dead_code)]
extern crate self as tauri;
use std::{cell::RefCell,rc::Rc};
#[derive(Debug,PartialEq,Eq)] pub enum AppError { Io(String) }
mod error { pub use crate::AppError; }
fn io_error(e:impl std::fmt::Display)->AppError { AppError::Io(e.to_string()) }
#[derive(Clone)] pub struct WebviewWindow { raw:bool, native:bool, fail:Option<&'static str>, calls:Rc<RefCell<Vec<&'static str>>>, shown:bool }
impl WebviewWindow {
 fn new(raw:bool,native:bool)->Self{Self{raw,native,fail:None,calls:Default::default(),shown:false}}
 fn op(&self,name:&'static str,value:bool)->Result<bool,&'static str>{self.calls.borrow_mut().push(name);if self.fail==Some(name){Err(name)}else{Ok(value)}}
 fn is_always_on_top(&self)->Result<bool,&'static str>{self.op("raw-read",self.raw)}
 fn set_always_on_top(&self,v:bool)->Result<(),&'static str>{self.op(if v{"raw-set-true"}else{"raw-set-false"},true).map(|_|())}
}
pub mod window_topmost_linux {
 use crate::{AppError,WebviewWindow};
 pub fn read(w:&WebviewWindow)->Result<bool,AppError>{w.op("native-read",w.native).map_err(|e|AppError::Io(e.into()))}
 pub fn set(w:&WebviewWindow,v:bool)->Result<(),AppError>{w.op(if v{"native-set-true"}else{"native-set-false"},true).map(|_|()).map_err(|e|AppError::Io(e.into()))}
}
mod platform { pub(crate) use crate::window_topmost_linux; mod actual { use crate::error::AppError; ${runnable(read)} ${runnable(set)} } pub(crate) use actual::{read,set}; }
#[cfg(test)] mod tests {
 use super::*;
 fn linux()->bool{cfg!(feature="linux")}
 #[test] fn read_selects_native_only_on_linux(){for (raw,native) in [(false,false),(true,false),(false,true),(true,true)]{let w=WebviewWindow::new(raw,native);assert_eq!(platform::read(&w),Ok(if linux(){native}else{raw}));assert_eq!(w.calls.borrow().as_slice(),if linux(){&["native-read"]}else{&["raw-read"]});}}
 #[test] fn set_true_uses_host_boundary_without_visibility_calls(){let w=WebviewWindow::new(false,false);platform::set(&w,true).unwrap();assert_eq!(w.calls.borrow().as_slice(),if linux(){&["native-set-true"]}else{&["raw-set-true"]});assert!(!w.shown);}
 #[test] fn set_false_uses_host_boundary_without_visibility_calls(){let w=WebviewWindow::new(true,true);platform::set(&w,false).unwrap();assert_eq!(w.calls.borrow().as_slice(),if linux(){&["native-set-false"]}else{&["raw-set-false"]});assert!(!w.shown);}
 #[test] fn read_errors_keep_original_message(){let mut w=WebviewWindow::new(false,false);w.fail=Some(if linux(){"native-read"}else{"raw-read"});assert_eq!(platform::read(&w),Err(AppError::Io(if linux(){"native-read"}else{"raw-read"}.into())));}
 #[test] fn set_errors_keep_original_message(){for value in [false,true]{let mut w=WebviewWindow::new(false,false);w.fail=Some(match (linux(),value){(true,true)=>"native-set-true",(true,false)=>"native-set-false",(false,true)=>"raw-set-true",(false,false)=>"raw-set-false"});let e=platform::set(&w,value).unwrap_err();assert_eq!(e,AppError::Io(w.fail.unwrap().into()));}}
 #[test] fn independent_windows_do_not_share_boundary_calls(){let a=WebviewWindow::new(false,false);let b=WebviewWindow::new(false,true);platform::set(&a,true).unwrap();assert_eq!(platform::read(&b),Ok(linux()));assert_eq!(a.calls.borrow().len(),1);assert_eq!(b.calls.borrow().len(),1);}
}
`)
    for (const feature of ['linux', 'macos', 'windows']) {
        const result = spawnSync(process.env.CARGO || 'cargo', ['test', '--offline', '--manifest-path', path.join(temporary, 'Cargo.toml'), '--features', feature, '--', '--test-threads=1'], { encoding: 'utf8', timeout: 60000, maxBuffer: 8 * 1024 * 1024 })
        assert.equal(result.status, 0, `${feature}: ${result.error ?? ''}\n${result.stdout ?? ''}${result.stderr ?? ''}`)
        assert.match(result.stdout, /test result: ok\. 6 passed; 0 failed/)
        process.stdout.write(`feature=${feature}\n${result.stdout}`)
    }
} finally {
    fs.rmSync(temporary, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 })
    for (const [file, expected] of fingerprints) assert.equal(crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex'), expected, `source fingerprint unchanged: ${file}`)
}
