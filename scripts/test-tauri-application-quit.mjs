import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import vm from 'node:vm'
import { execFileSync, spawnSync } from 'node:child_process'
import ts from 'typescript'

const root = new URL('../', import.meta.url)
const current = fs.readFileSync(new URL('src-tauri/src/lib.rs', root), 'utf8')
const appCommands = fs.readFileSync(new URL('src-tauri/src/commands/app.rs', root), 'utf8')
const platformService = fs.readFileSync(new URL('tabby-tauri/src/services/platform.service.ts', root), 'utf8')
const hostAppService = fs.readFileSync(new URL('tabby-tauri/src/services/hostApp.service.ts', root), 'utf8')
const sourceFailures = []
function expectSource (callback, message) {
    try { callback() } catch { sourceFailures.push(message) }
}
expectSource(() => assert.ok(/\.build\(\s*tauri::generate_context!\(\)\s*\)\s*\.expect\([^)]*\)\s*\.run\(\s*on_application_event\s*\)/s.test(current)), 'Tauri builder must register the callback')
expectSource(() => assert.ok(/pub\s+fn\s+app_quit\s*\([\s\S]*?\)\s*->\s*Result<\(\),\s*AppError>\s*\{[\s\S]*?app\.exit\(0\);[\s\S]*?\}/.test(appCommands)), 'app_quit must retain app.exit(0)')
expectSource(() => assert.ok(!/crash::clear/.test(appCommands.match(/pub\s+fn\s+app_quit\s*\([\s\S]*?\)\s*->\s*Result<\(\),\s*AppError>\s*\{([\s\S]*?)\n\}/)?.[1] ?? '')), 'app_quit must not clear the marker during a quit request')
const oldHostApp = execFileSync('git', ['show', '14e2d60b9b6dee84a53c37f05eefeb803787de04:tabby-electron/src/services/hostApp.service.ts'], { cwd: root, encoding: 'utf8' })
const oldPlatform = execFileSync('git', ['show', '14e2d60b9b6dee84a53c37f05eefeb803787de04:tabby-electron/src/services/platform.service.ts'], { cwd: root, encoding: 'utf8' })

function methodSource (source, className, methodName) {
    const ast = ts.createSourceFile('service.ts', source, ts.ScriptTarget.Latest, true)
    const declaration = ast.statements.find(node => ts.isClassDeclaration(node) && node.name?.text === className)
    const method = declaration?.members.find(node => ts.isMethodDeclaration(node) && node.name.getText(ast) === methodName)
    assert.ok(method, `${className}.${methodName} must exist`)
    return ts.createPrinter().printNode(ts.EmitHint.Unspecified, method, ast)
}

assert.match(methodSource(oldHostApp, 'ElectronHostAppService', 'quit'), /this\.electron\.app\.quit\(\)/, 'Electron host quit must use app.quit()')
assert.match(methodSource(oldPlatform, 'ElectronPlatformService', 'quit'), /this\.electron\.app\.exit\(0\)/, 'Electron platform quit must preserve immediate exit(0)')

function loadQuitMethod (source, className) {
    const module = { exports: {} }
    const code = `export class QuitHarness { ${methodSource(source, className, 'quit')} }`
    vm.runInNewContext(ts.transpileModule(code, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS } }).outputText, { exports: module.exports })
    return module.exports.QuitHarness
}

const PlatformQuit = loadQuitMethod(platformService, 'TauriPlatformService')
const HostQuit = loadQuitMethod(hostAppService, 'TauriHostAppService')
const quitCalls = []
new PlatformQuit().quit.call({ bridge: { invoke: (command, payload) => quitCalls.push([command, payload]) } })
new HostQuit().quit.call({ bridge: { invoke: (command, payload) => quitCalls.push([command, payload]) } })
expectSource(() => {
    assert.equal(quitCalls.length, 2)
    assert.equal(quitCalls[0][0], 'app.quit')
    assert.deepEqual(Object.keys(quitCalls[0][1]).sort(), ['force'])
    assert.equal(quitCalls[0][1].force, true)
    assert.equal(quitCalls[1][0], 'app.quit')
    assert.equal(Object.keys(quitCalls[1][1]).length, 0)
}, 'Tauri platform quit must send force=true while host quit sends {}')

// Exercise the actual AppService methods as emitted from their TypeScript AST.
const appSource = ts.createSourceFile('app.service.ts', fs.readFileSync(new URL('tabby-core/src/services/app.service.ts', root), 'utf8'), ts.ScriptTarget.Latest, true)
const appClass = appSource.statements.find(node => ts.isClassDeclaration(node) && node.name.text === 'AppService')
const methods = appClass.members.filter(node => ts.isMethodDeclaration(node) && ['closeAllTabs', 'closeWindow'].includes(node.name.getText(appSource)))
assert.equal(methods.length, 2)
const module = { exports: {} }
const classCode = `export class AppServiceHarness { ${methods.map(node => ts.createPrinter().printNode(ts.EmitHint.Unspecified, node, appSource)).join('\n')} }`
vm.runInNewContext(ts.transpileModule(classCode, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS } }).outputText, { exports: module.exports })

async function rendererCases () {
    for (const answer of [false, true]) {
        const service = new module.exports.AppServiceHarness()
        const events = []
        service.tabs = [{ canClose: async () => { events.push('confirm'); return answer }, destroy: () => events.push('destroy') }]
        service.tabRecovery = { enabled: true, saveTabs: async () => events.push('save') }
        service.hostWindow = { close: () => events.push('window.close') }
        await service.closeWindow()
        assert.deepEqual(events, answer ? ['save', 'confirm', 'destroy', 'window.close'] : ['save', 'confirm'], 'cancel/approve must preserve or destroy tabs before closing the window')
        assert.equal(service.tabRecovery.enabled, !answer, 'cancel enables tab recovery again; approval leaves it disabled')
    }
    const service = new module.exports.AppServiceHarness()
    const events = []
    service.tabs = [{ canClose: async () => true, destroy: () => events.push('first destroy') }, { canClose: async () => false, destroy: () => events.push('second destroy') }]
    assert.equal(await service.closeAllTabs(), false)
    assert.deepEqual(events, [], 'a rejected tab keeps every tab intact in a multi-tab window')
    service.tabs[1].canClose = async () => true
    assert.equal(await service.closeAllTabs(), true)
    assert.deepEqual(events, ['first destroy', 'second destroy'], 'approved multi-tab close destroys every tab only after all confirmations')

    const pending = new module.exports.AppServiceHarness()
    const approved = new module.exports.AppServiceHarness()
    let resolvePending
    const firstEvents = []
    const secondEvents = []
    pending.tabs = [{ canClose: () => new Promise(resolve => { resolvePending = resolve }), destroy: () => firstEvents.push('tab-1 destroy') }, { canClose: async () => true, destroy: () => firstEvents.push('tab-2 destroy') }]
    pending.tabRecovery = { enabled: true, saveTabs: async () => firstEvents.push('save') }
    pending.hostWindow = { close: () => firstEvents.push('window.close') }
    approved.tabs = [{ canClose: async () => true, destroy: () => secondEvents.push('tab-1 destroy') }, { canClose: async () => true, destroy: () => secondEvents.push('tab-2 destroy') }]
    approved.tabRecovery = { enabled: true, saveTabs: async () => secondEvents.push('save') }
    approved.hostWindow = { close: () => secondEvents.push('window.close') }
    const firstClose = pending.closeWindow()
    await Promise.resolve()
    assert.deepEqual(firstEvents, ['save'], 'pending confirmation keeps the first window and all its tabs intact')
    await approved.closeWindow()
    assert.deepEqual(secondEvents, ['save', 'tab-1 destroy', 'tab-2 destroy', 'window.close'], 'the other window completes approved close while first is pending')
    resolvePending(false)
    await firstClose
    assert.deepEqual(firstEvents, ['save'], 'cancelling the pending window preserves all its tabs and window')

    const retry = new module.exports.AppServiceHarness()
    const retryEvents = []
    let answer = false
    retry.tabs = [{ canClose: async () => answer, destroy: () => retryEvents.push('destroy') }]
    retry.tabRecovery = { enabled: true, saveTabs: async () => retryEvents.push('save') }
    retry.hostWindow = { close: () => retryEvents.push('window.close') }
    await retry.closeWindow()
    assert.deepEqual(retryEvents, ['save'], 'cancelled close leaves the window open for retry')
    answer = true
    await retry.closeWindow()
    assert.deepEqual(retryEvents, ['save', 'save', 'destroy', 'window.close'], 'retry after approval destroys tabs then closes the window')
}
await rendererCases()

// The callback body is spliced verbatim into the harness; only Tauri boundary APIs are faked.
const signature = /(?:pub\s+)?fn\s+on_application_event\s*\(\s*app\s*:\s*&\s*tauri::AppHandle\s*,\s*event\s*:\s*tauri::RunEvent\s*\)\s*(?:->\s*\(\s*\))?\s*\{/m
const match = signature.exec(current)
const quitSignature = /(?:pub\s+)?fn\s+app_quit\s*\(\s*request\s*:\s*QuitRequest\s*,[^)]*\)\s*->\s*Result<\(\),\s*AppError>\s*\{/m
const quitMatch = quitSignature.exec(appCommands)
if (!match || !quitMatch) {
    if (!match) console.error('FAIL native application-quit cases: production application callback is missing.')
    if (!quitMatch) console.error('FAIL native command cases: app_quit(QuitRequest, AppHandle, State<AppState>) is missing.')
    process.exitCode = 1
} else {
    function extractFunction (source, functionMatch) {
        const start = functionMatch.index + functionMatch[0].length - 1
        let depth = 0; let end = start
        for (; end < source.length; end++) { if (source[end] === '{') depth++; else if (source[end] === '}' && --depth === 0) break }
        return source.slice(functionMatch.index, end + 1)
    }
    const callback = extractFunction(current, match)
    const appQuit = extractFunction(appCommands, quitMatch)
    const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-quit-'))
    try {
        const source = `
use std::{collections::HashMap, path::PathBuf, sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex}};
pub type Calls = Arc<Mutex<Vec<String>>>;
mod tauri {
    use std::ops::Deref;
    use super::{HashMap, Calls, Window, AppState};
    #[derive(Clone)] pub struct AppHandle { pub windows: HashMap<String, Window>, pub calls: Calls, pub state: AppState }
    impl AppHandle { pub fn webview_windows(&self) -> HashMap<String, Window> { self.windows.clone() } pub fn state<T>(&self) -> State<'_, AppState> { State { inner: &self.state } } pub fn exit(&self, code: i32) { self.calls.lock().unwrap().push(format!("exit:{}:{}", code, self.state.is_force_quit_requested())); } }
    pub struct State<'a, T> { pub inner: &'a T }
    impl<'a, T> State<'a, T> { pub fn new(inner: &'a T) -> Self { Self { inner } } }
    impl<T> Deref for State<'_, T> { type Target = T; fn deref(&self) -> &T { self.inner } }
    pub enum RunEvent { ExitRequested { code: Option<i32>, api: ExitRequestApi }, Exit, Other }
    #[derive(Clone)] pub struct ExitRequestApi { pub calls: Calls }
    impl ExitRequestApi { pub fn prevent_exit(&self) { self.calls.lock().unwrap().push("prevent_exit".into()); } }
}
#[derive(Clone)] pub struct Window { pub label: String, pub calls: Calls, pub fail: bool }
impl Window { pub fn close(&self) -> Result<(), String> { self.calls.lock().unwrap().push(format!("close:{}", self.label)); if self.fail { self.calls.lock().unwrap().push(format!("close_error:{}", self.label)); Err(format!("{} close failed", self.label)) } else { Ok(()) } } }
#[derive(Clone)] pub struct Paths { pub logs: PathBuf }
#[derive(Clone)] pub struct AppState { pub paths: Paths, pub forced_quit: Arc<AtomicBool> }
impl AppState { pub fn paths(&self) -> &Paths { &self.paths } }
impl AppState { pub fn is_force_quit_requested(&self) -> bool { self.forced_quit.load(Ordering::SeqCst) } pub fn request_forced_quit(&self) { self.forced_quit.store(true, Ordering::SeqCst); } }
impl Paths { pub fn logs_dir(&self) -> &PathBuf { &self.logs } }
pub struct QuitRequest { pub force: bool }
#[derive(Debug)] pub struct AppError;
mod diagnostics { pub mod crash { use std::{fs, path::Path}; pub fn clear(path: &Path) -> Result<(), String> { fs::remove_file(path.join("crash-marker.json")).map_err(|e| e.to_string()) } } }
${callback}
mod commands { use super::{AppError, AppState, QuitRequest}; use super::tauri::{AppHandle, State}; ${appQuit} }
#[cfg(test)] mod tests {
    use super::*;
    fn fixture(labels: &[(&str, bool)]) -> tauri::AppHandle {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let windows = labels.iter().map(|(label, fail)| ((*label).to_string(), Window { label: (*label).into(), calls: calls.clone(), fail: *fail })).collect();
        tauri::AppHandle { windows, calls, state: AppState { paths: Paths { logs: PathBuf::from(std::env::var("TABBY_TEST_LOGS_DIR").unwrap()) }, forced_quit: Arc::new(AtomicBool::new(false)) } }
    }
    fn request(app: &tauri::AppHandle, code: Option<i32>) { on_application_event(app, tauri::RunEvent::ExitRequested { code, api: tauri::ExitRequestApi { calls: app.calls.clone() } }); }
    fn calls(app: &tauri::AppHandle) -> Vec<String> { app.calls.lock().unwrap().clone() }
    #[test] fn two_windows_prevent_exit_before_close_requests() {
        let app = fixture(&[("window-b", false), ("window-a", false)]); request(&app, Some(0));
        let got = calls(&app); assert_eq!(got.first().map(String::as_str), Some("prevent_exit"));
        assert_eq!(got.iter().filter(|x| x.as_str() == "prevent_exit").count(), 1);
        let mut expected = vec!["close:window-a".to_string(), "close:window-b".to_string()]; expected.sort();
        let mut actual = got.iter().filter(|x| x.starts_with("close:")).cloned().collect::<Vec<_>>(); actual.sort();
        assert_eq!(actual, expected, "each owning window receives exactly one close request");
    }
    #[test] fn zero_windows_does_not_block_normal_exit() {
        let app = fixture(&[]); request(&app, None); assert!(calls(&app).is_empty());
    }
    #[test] fn none_and_zero_exit_codes_request_window_closure() {
        for code in [None, Some(0)] { let app = fixture(&[("only", false)]); request(&app, code); assert_eq!(calls(&app), vec!["prevent_exit", "close:only"]); }
    }
    #[test] fn restart_and_nonzero_exit_codes_bypass_window_closure() {
        for code in [Some(1), Some(-1), Some(i32::MAX)] { let app = fixture(&[("only", false)]); request(&app, code); assert!(calls(&app).is_empty()); }
    }
    #[test] fn failed_close_still_attempts_remaining_windows_and_keeps_exit_blocked() {
        let mut app = fixture(&[("window-a", false), ("window-b", false)]);
        let first_label = app.windows.values().next().unwrap().label.clone();
        app.windows.get_mut(&first_label).unwrap().fail = true;
        let mut expected = app.windows.keys().cloned().collect::<Vec<_>>(); expected.sort(); request(&app, Some(0)); let got = calls(&app);
        assert_eq!(got.first().map(String::as_str), Some("prevent_exit"));
        let mut actual = got.iter().filter(|x| x.starts_with("close:")).map(|x| x.trim_start_matches("close:").to_string()).collect::<Vec<_>>(); actual.sort();
        assert_eq!(actual, expected, "a failed first close does not skip the remaining window");
        assert!(got.contains(&format!("close_error:{}", first_label)), "first close really reports an error");
    }
    #[test] fn crash_marker_is_cleared_only_for_actual_exit() {
        let logs = std::env::var("TABBY_TEST_LOGS_DIR").unwrap(); let marker = PathBuf::from(&logs).join("crash-marker.json");
        std::fs::write(&marker, "marker").unwrap(); let app = fixture(&[("only", false)]); request(&app, Some(0));
        assert!(marker.exists(), "ExitRequested must not clear the crash marker");
        on_application_event(&app, tauri::RunEvent::Other); assert!(marker.exists(), "unrelated events must not clear the marker");
        on_application_event(&app, tauri::RunEvent::Exit); assert!(!marker.exists(), "actual Exit clears the crash marker");
    }
    #[test] fn app_quit_without_force_preserves_normal_confirmation() {
        let app = fixture(&[("only", false)]);
        let quit_request = QuitRequest { force: false };
        let _ = commands::app_quit(quit_request, app.clone(), tauri::State::new(&app.state));
        assert!(!app.state.is_force_quit_requested(), "force=false must not set the bypass flag");
        assert_eq!(calls(&app), vec!["exit:0:false"], "normal Quit keeps exit code zero and does not set the force flag");
        request(&app, Some(0));
        assert_eq!(calls(&app), vec!["exit:0:false", "prevent_exit", "close:only"], "normal quit still requests renderer confirmation");
    }
    #[test] fn app_quit_with_force_sets_flag_before_exit() {
        let app = fixture(&[("only", false)]);
        let quit_request = QuitRequest { force: true };
        let _ = commands::app_quit(quit_request, app.clone(), tauri::State::new(&app.state));
        assert!(app.state.is_force_quit_requested(), "force=true sets the bypass flag");
        assert_eq!(calls(&app), vec!["exit:0:true"], "forced flag is set before exit(0)");
    }
    #[test] fn forced_quit_flag_bypasses_window_confirmation() {
        let app = fixture(&[("only", false)]); app.state.request_forced_quit();
        request(&app, Some(0)); assert!(calls(&app).is_empty(), "forced exit bypasses prevent_exit and window.close");
    }
}
`
        const logs = fs.mkdtempSync(path.join(scratch, 'logs-'))
        const rust = path.join(scratch, 'harness.rs'); fs.writeFileSync(rust, source)
        const compiler = process.env.RUSTC || 'rustc'
        const binary = path.join(scratch, process.platform === 'win32' ? 'harness.exe' : 'harness')
        const built = spawnSync(compiler, ['--test', rust, '-o', binary], { encoding: 'utf8', timeout: 30000, env: { ...process.env, TABBY_TEST_LOGS_DIR: logs } })
        assert.equal(built.status, 0, `extracted production callback must compile with boundary fakes:\n${built.error ?? built.stderr}`)
        const ran = spawnSync(binary, [], { encoding: 'utf8', timeout: 30000, env: { ...process.env, TABBY_TEST_LOGS_DIR: logs } })
        assert.equal(ran.status, 0, ran.stderr)
        console.log('Native callback and command cases: 9 Rust tests passed')
    } finally { fs.rmSync(scratch, { recursive: true, force: true }) }
}
for (const failure of sourceFailures) console.error(`FAIL source contract: ${failure}`)
if (sourceFailures.length) process.exitCode = 1
