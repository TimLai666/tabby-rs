import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

if (process.argv.length === 3 && process.argv[2] === '--help') {
    console.log('Usage: node scripts/test-window-operation-completion.mjs\nCompile and test the real non-macOS window-operation completion function with the locked Tokio version. Tauri UI dispatch is simulated. Uses Cargo offline and removes its temporary crate.')
    process.exit(0)
}
assert.equal(process.argv.length, 2, 'no arguments supported; use --help')

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourcePath = path.join(root, 'src-tauri/src/platform/mod.rs')
const lockPath = path.join(root, 'src-tauri/Cargo.lock')
const files = [sourcePath, lockPath]
const fingerprint = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')
const fingerprints = new Map(files.map(file => [file, fingerprint(file)]))
const source = fs.readFileSync(sourcePath, 'utf8')
const marker = 'pub async fn wait_for_window_operations('
const start = source.indexOf(marker)
assert.ok(start >= 0 && start === source.lastIndexOf(marker), 'one wait_for_window_operations definition')
const body = source.indexOf('{', start)
assert.ok(body > start, 'function body exists')
let depth = 0
let quoted = false
let escaped = false
let lineComment = false
let blockComment = false
let end = -1
for (let index = body; index < source.length; index++) {
    const ch = source[index]
    const next = source[index + 1]
    if (lineComment) {
        if (ch === '\n') lineComment = false
    } else if (blockComment) {
        if (ch === '*' && next === '/') { blockComment = false; index++ }
    } else if (quoted) {
        if (escaped) escaped = false
        else if (ch === '\\') escaped = true
        else if (ch === '"') quoted = false
    } else if (ch === '"') quoted = true
    else if (ch === '/' && next === '/') { lineComment = true; index++ }
    else if (ch === '/' && next === '*') { blockComment = true; index++ }
    else if (ch === '{') depth++
    else if (ch === '}' && --depth === 0) { end = index + 1; break }
}
assert.ok(end > body, 'complete production function extracted')
const productionFunction = source.slice(start, end)
const lock = fs.readFileSync(lockPath, 'utf8')
const tokio = lock.match(/\[\[package\]\]\r?\nname = "tokio"\r?\nversion = "([^"]+)"/)
assert.ok(tokio, 'locked Tokio version')

const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-window-operation-completion-'))
let result
let spawnError
try {
    fs.mkdirSync(path.join(temporary, 'src'))
    fs.writeFileSync(path.join(temporary, 'Cargo.toml'), `[package]\nname = "tabby-window-operation-completion-fixture"\nversion = "0.0.0"\nedition = "2021"\n[dependencies]\ntokio = { version = "=${tokio[1]}", features = ["macros", "rt", "sync", "time"] }\n`)
    fs.writeFileSync(path.join(temporary, 'src/lib.rs'), `#![allow(dead_code)]
extern crate self as tauri;

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[derive(Debug, PartialEq, Eq)]
pub enum AppError {
    Io(String),
}

type UiTask = Box<dyn FnOnce() + Send + 'static>;

#[derive(Default)]
struct UiDispatcher {
    tasks: Mutex<VecDeque<UiTask>>,
    rejected: AtomicBool,
}

impl UiDispatcher {
    fn run_one(&self) -> bool {
        let task = self.tasks.lock().unwrap().pop_front();
        if let Some(task) = task { task(); true } else { false }
    }

    fn drop_one(&self) -> bool {
        self.tasks.lock().unwrap().pop_front().is_some()
    }

    fn len(&self) -> usize {
        self.tasks.lock().unwrap().len()
    }
}

pub struct WebviewWindow {
    label: String,
    ui: Arc<UiDispatcher>,
}

impl WebviewWindow {
    fn new(label: &str) -> Self {
        Self { label: label.into(), ui: Arc::new(UiDispatcher::default()) }
    }

    fn run_on_main_thread<F>(&self, task: F) -> Result<(), String>
    where
        F: FnOnce() + Send + 'static,
    {
        if self.ui.rejected.load(Ordering::Acquire) {
            return Err(format!("{} dispatcher rejected task", self.label));
        }
        self.ui.tasks.lock().unwrap().push_back(Box::new(task));
        Ok(())
    }

    fn set_maximized(&self, state: Arc<AtomicBool>) -> Result<(), String> {
        self.run_on_main_thread(move || state.store(true, Ordering::Release))
    }
}

${productionFunction}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll, Wake, Waker},
        time::Duration,
    };

    struct NoopWake;
    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);
        future.poll(&mut context)
    }

    fn assert_pending<F: Future>(future: Pin<&mut F>) {
        assert!(matches!(poll_once(future), Poll::Pending), "waiter must remain pending until its UI callback completes");
    }

    async fn bounded<F: Future>(future: F) -> F::Output {
        tokio::time::timeout(Duration::from_secs(2), future)
            .await
            .expect("window-operation waiter did not complete within the bound")
    }

    #[tokio::test]
    async fn registers_one_callback_and_completes_after_it_runs() {
        let window = WebviewWindow::new("main");
        let mut future = Box::pin(wait_for_window_operations(&window));
        assert_pending(future.as_mut());
        assert_eq!(window.ui.len(), 1);
        assert!(window.ui.run_one());
        assert_eq!(bounded(future).await, Ok(()));
    }

    #[tokio::test]
    async fn waits_until_preceding_native_maximize_task_has_run() {
        let window = WebviewWindow::new("main");
        let maximized = Arc::new(AtomicBool::new(false));
        window.set_maximized(maximized.clone()).unwrap();
        let mut future = Box::pin(wait_for_window_operations(&window));
        assert_pending(future.as_mut());
        assert_eq!(window.ui.len(), 2);

        assert!(window.ui.run_one());
        assert!(maximized.load(Ordering::Acquire));
        assert_pending(future.as_mut());
        assert!(window.ui.run_one());
        assert_eq!(bounded(future).await, Ok(()));
    }

    #[tokio::test]
    async fn each_window_callback_completes_only_its_own_waiter() {
        let first = WebviewWindow::new("first");
        let second = WebviewWindow::new("second");
        let mut first_future = Box::pin(wait_for_window_operations(&first));
        let mut second_future = Box::pin(wait_for_window_operations(&second));
        assert_pending(first_future.as_mut());
        assert_pending(second_future.as_mut());
        assert_eq!(first.ui.len(), 1);
        assert_eq!(second.ui.len(), 1);

        assert!(first.ui.run_one());
        assert!(matches!(poll_once(first_future.as_mut()), Poll::Ready(Ok(()))));
        assert_pending(second_future.as_mut());
        assert!(second.ui.run_one());
        assert_eq!(bounded(second_future).await, Ok(()));
    }

    #[tokio::test]
    async fn dispatcher_rejection_returns_an_io_error() {
        let window = WebviewWindow::new("rejected");
        window.ui.rejected.store(true, Ordering::Release);
        let result = bounded(wait_for_window_operations(&window)).await;
        assert_eq!(result, Err(AppError::Io("rejected dispatcher rejected task".into())));
        assert_eq!(window.ui.len(), 0);
    }

    #[tokio::test]
    async fn dropping_the_queued_callback_returns_an_error() {
        let window = WebviewWindow::new("dropped");
        let mut future = Box::pin(wait_for_window_operations(&window));
        assert_pending(future.as_mut());
        assert_eq!(window.ui.len(), 1);
        assert!(window.ui.drop_one());
        assert!(matches!(bounded(future).await, Err(AppError::Io(_))));
    }

    #[tokio::test]
    async fn running_callback_after_waiter_cancellation_does_not_panic() {
        let window = WebviewWindow::new("cancelled");
        let mut future = Box::pin(wait_for_window_operations(&window));
        assert_pending(future.as_mut());
        assert_eq!(window.ui.len(), 1);
        drop(future);
        assert!(window.ui.run_one());
    }
}
`)
    result = spawnSync('cargo', ['test', '--offline', '--manifest-path', path.join(temporary, 'Cargo.toml'), '--', '--nocapture'], { cwd: root, encoding: 'utf8', timeout: 180000, maxBuffer: 12 * 1024 * 1024 })
} catch (error) {
    spawnError = error
} finally {
    fs.rmSync(temporary, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 })
}

for (const file of files) {
    assert.equal(fingerprint(file), fingerprints.get(file), 'production inputs remained unchanged')
}
if (spawnError) throw spawnError
if (result.error) throw result.error
assert.equal(result.signal, null, result.stderr)
if (result.status !== 0) {
    process.stdout.write(result.stdout)
    process.stderr.write(result.stderr)
    throw new Error(`temporary Rust behavior tests failed with status ${result.status}`)
}
process.stdout.write(result.stdout)
process.stderr.write(result.stderr)
assert.match(result.stdout, /test result: ok\. 6 passed; 0 failed/, 'all UI completion cases executed')
console.log('Window operation completion: callback ordering, window scope, dispatch rejection, callback drop, and waiter cancellation passed')
