import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-rs-signature-'))
const artifact = path.join(directory, 'artifact')
const signature = path.join(directory, 'artifact.sig')
const publicKey = path.join(directory, 'public.key')
fs.writeFileSync(artifact, 'test')
fs.writeFileSync(signature, `untrusted comment: signature from minisign secret key
RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=
trusted comment: timestamp:1555779966\tfile:test
QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==
`)
fs.writeFileSync(publicKey, `untrusted comment: minisign public key E7620F1842B4E81F
RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3
`)

const verifier = ['run', '--quiet', '--manifest-path', 'src-tauri/Cargo.toml', '--bin', 'verify-tauri-signature', '--', artifact, signature, publicKey]
execFileSync('cargo', verifier, { cwd: root, stdio: 'inherit' })
fs.writeFileSync(artifact, 'Test')
assert.throws(() => execFileSync('cargo', verifier, { cwd: root, stdio: 'pipe' }))
console.log('Tauri updater signature contract passed')
