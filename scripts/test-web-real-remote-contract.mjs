import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'scripts/test-web-real-remote-gateway.mjs'), 'utf8')

assert.match(source, /const sshdPath = process\.env\.TABBY_RS_SSHD \|\| '\/usr\/sbin\/sshd'/)
assert.match(source, /ensureExecutable\(sshdPath\)/)
assert.match(source, /spawn\(sshdPath, \['-D', '-e', '-f', config\]/)
assert.doesNotMatch(source, /commandOutput\(sshdPath, \['-V'\]\)/)

console.log('Web real remote OpenSSH portability contract passed')
