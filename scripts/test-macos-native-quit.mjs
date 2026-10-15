#!/usr/bin/env node
// Drives the native macOS application-quit regression.
//
// The Rust example loads the real `src/platform/macos_quit.rs` and terminates a
// real NSApplication. Each scenario runs in its own helper process. A pass
// requires the single complete JSON evidence marker with the expected values:
// a clean exit code is not enough, because the empty-skeleton install lets
// AppKit terminate the helper before it can print anything.

import { spawn, spawnSync } from 'node:child_process'
import process from 'node:process'

const MARKER = 'TABBY_QUIT_NATIVE_EVIDENCE '

const CASES = [
  {
    name: 'terminate_cancels_direct_exit',
    expect: e => e.passed === true && Array.isArray(e.exitCodes) && e.exitCodes.length === 1 &&
      e.exitCodes.every(code => code === 0) && e.willTerminate === false,
  },
  {
    name: 'repeated_terminate_stays_alive',
    expect: e => e.passed === true && e.terminateCalls === 3 && Array.isArray(e.exitCodes) &&
      e.exitCodes.length === 3 && e.exitCodes.every(code => code === 0) && e.willTerminate === false,
  },
  {
    name: 'repeat_install_preserves_delegate',
    expect: e => e.passed === true && e.firstInstallOk === true && e.secondInstallOk === true &&
      e.delegatePreserved === true && e.classPreserved === true,
  },
  {
    name: 'preserves_original_callbacks_and_ivars',
    expect: e => e.passed === true && e.reopenCount === 2 && e.sentinel === 0xabcdef &&
      e.delegatePreserved === true,
  },
  {
    name: 'changed_delegate_install_errors',
    expect: e => e.passed === true && e.result === 'err',
  },
  {
    name: 'non_main_thread_install_errors',
    expect: e => e.passed === true && e.result === 'err',
  },
  {
    name: 'no_delegate_install_errors',
    expect: e => e.passed === true && e.result === 'err',
  },
  {
    name: 'conflicting_delegate_install_errors',
    expect: e => e.passed === true && e.result === 'err' && e.delegatePreserved === true &&
      e.shouldTerminateReplyPreserved === true,
  },
]

if (process.platform !== 'darwin') {
  console.log('macos native quit probe: skipped (focused Cocoa probe requires macOS)')
  process.exit(0)
}

function buildHelper () {
  const result = spawnSync('cargo', [
    'build', '--locked', '--offline',
    '--manifest-path', 'src-tauri/Cargo.toml',
    '--example', 'macos_quit_native',
    '--message-format=json',
  ], { encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 })

  if (result.error) {
    console.error('macos native quit probe: cargo could not start:', result.error.message)
    process.exit(1)
  }
  if (result.status !== 0) {
    for (const line of (result.stdout || '').split('\n')) {
      try {
        const message = JSON.parse(line)
        if (message.reason === 'compiler-message' && message.message?.rendered) {
          console.error(message.message.rendered)
        }
      } catch { /* Cargo diagnostics use JSON; ignore unrelated output. */ }
    }
    if (result.stderr) console.error(result.stderr)
    throw new Error('macos native quit probe: cargo build failed with status ' + result.status)
  }

  let executable = null
  for (const line of result.stdout.split('\n')) {
    if (!line.trim()) continue
    let message
    try { message = JSON.parse(line) } catch { continue }
    if (message.reason !== 'compiler-artifact' || !message.executable) continue
    const target = message.target || {}
    if (target.name === 'macos_quit_native' && Array.isArray(target.kind) && target.kind.includes('example')) {
      executable = message.executable
    }
  }
  if (!executable) {
    console.error('macos native quit probe: cargo did not report an example executable')
    process.exit(1)
  }
  return executable
}

function runCase (executable, name) {
  return new Promise(resolve => {
    const child = spawn(executable, [name], { stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    let timedOut = false
    const timer = setTimeout(() => {
      timedOut = true
      child.kill('SIGKILL')
    }, 15000)
    child.stdout.on('data', chunk => { stdout += chunk })
    child.stderr.on('data', chunk => { stderr += chunk })
    child.on('error', error => {
      clearTimeout(timer)
      resolve({ code: null, timedOut: false, spawnError: error, stdout, stderr })
    })
    child.on('close', code => {
      clearTimeout(timer)
      resolve({ code, timedOut, stdout, stderr })
    })
  })
}

function parseMarker (stdout) {
  const lines = stdout.split('\n').filter(line => line.startsWith(MARKER))
  if (lines.length !== 1) return { error: `expected exactly one evidence marker, found ${lines.length}` }
  try {
    return { evidence: JSON.parse(lines[0].slice(MARKER.length)) }
  } catch (error) {
    return { error: 'evidence marker is not valid JSON: ' + error.message }
  }
}

const executable = buildHelper()
let passed = 0

for (const testCase of CASES) {
  const outcome = await runCase(executable, testCase.name)
  const fail = reason => {
    console.log(`FAIL ${testCase.name} (${reason})`)
    const tail = (outcome.stderr || '').trim().split('\n').slice(-3).join(' | ')
    if (tail) console.log(`     stderr: ${tail}`)
  }

  if (outcome.spawnError) {
    fail('spawn error: ' + outcome.spawnError.message)
    continue
  }
  if (outcome.timedOut) {
    fail('timed out after 15s without a decision')
    continue
  }
  if (outcome.code !== 0) {
    fail('abnormal helper exit: ' + outcome.code)
    continue
  }
  const { evidence, error } = parseMarker(outcome.stdout)
  if (error) {
    fail(`no complete evidence (${error}); exit=${outcome.code}`)
    continue
  }
  if (evidence.case !== testCase.name) {
    fail(`evidence case mismatch: ${evidence.case}`)
    continue
  }
  if (!testCase.expect(evidence)) {
    fail('evidence values did not match expectations: ' + JSON.stringify(evidence))
    continue
  }
  console.log(`PASS ${testCase.name}`)
  passed++
}

console.log(`macos native quit probe: ${passed}/${CASES.length} passed`)
process.exit(passed === CASES.length ? 0 : 1)
