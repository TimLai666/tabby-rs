import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const session = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/session.ts'), 'utf8')
const tab = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/tab.component.ts'), 'utf8')

assert.match(session, /private pendingExit: SshExitEvent\|null = null/)
assert.match(session, /private readonly serviceMessage = new Subject<string>\(\)/)
assert.match(session, /get serviceMessage\$ \(\): Observable<string>/)
assert.match(session, /event\.exitCode !== null[\s\S]*event\.signal !== null/)
assert.match(session, /this\.serviceMessage\.complete\(\)/)
assert.match(session, /const account = options\.user \|\| 'root'/)
assert.match(tab, /attachSessionHandler\(session\.serviceMessage\$/)

console.log('SSH session exit contract passed')
