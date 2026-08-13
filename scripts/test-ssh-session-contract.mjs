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

const rust = fs.readFileSync(path.join(root, 'src-tauri/src/ssh/mod.rs'), 'utf8')
assert.match(rust, /let mut exit_event_emitted = false/)
assert.match(rust, /ChannelMsg::ExitStatus \{ \.\. \}[\s\S]*ChannelMsg::ExitSignal \{ \.\. \}/)
assert.match(rust, /exit_event_emitted \|= is_exit_message/)
assert.match(rust, /if !exit_event_emitted \{[\s\S]*exit_code: None[\s\S]*signal: None/)
assert.match(rust, /use zeroize::Zeroize/)
assert.match(rust, /bytes\.zeroize\(\)/)
assert.match(rust, /text\.zeroize\(\)/)

const directConnectBranch = rust.match(
    /if request\.jump_chain\.is_empty\(\) \{([\s\S]*?)\n        \} else \{/,
)
assert.ok(directConnectBranch, 'SSH connect branch contract is missing')
assert.match(directConnectBranch[1], /connect_direct_engine\(/)
assert.doesNotMatch(directConnectBranch[1], /\.authenticate\(/)
const jumpConnectBranch = rust.match(
    /\} else \{([\s\S]*?)\n        \}\n\n        let channel/,
)
assert.ok(jumpConnectBranch, 'SSH jump branch contract is missing')
assert.match(jumpConnectBranch[1], /connect_over_channel\(/)
assert.match(jumpConnectBranch[1], /if !self\s*\.\s*authenticate\(/)

console.log('SSH session exit contract passed')
