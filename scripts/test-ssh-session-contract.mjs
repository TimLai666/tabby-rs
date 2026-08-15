import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const session = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/session.ts'), 'utf8')
const tab = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/tab.component.ts'), 'utf8')
const recovery = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/recoveryProvider.ts'), 'utf8')

const authForOptions = session.match(
    /private async authForOptions \(options: SSHProfile\['options'\]\): Promise<SshAuthMethodRef\[]> \{([\s\S]*?)\n    \}/,
)
assert.ok(authForOptions, 'SSH auth option mapping is missing')
assert.match(authForOptions[1], /if \(!options\.auth\)/)
assert.match(authForOptions[1], /const privateKeys = options\.privateKeys\.length/)
assert.match(authForOptions[1], /auth\.push\(\{ type: 'privateKey', fileRef, passphraseRef: null \}\)/)
assert.match(authForOptions[1], /auth\.push\(\{ type: 'agent', socket: null \}\)/)
assert.match(authForOptions[1], /auth\.push\(\{ type: 'keyboardInteractive' \}\)/)
assert.match(authForOptions[1], /options\.auth === 'keyboardInteractive'[\s\S]*auth\.push\(\{ type: 'keyboardInteractive' \}\)/)

assert.match(session, /private pendingExit: SshExitEvent\|null = null/)
assert.match(session, /private readonly serviceMessage = new Subject<string>\(\)/)
assert.match(session, /get serviceMessage\$ \(\): Observable<string>/)
assert.match(session, /event\.exitCode !== null[\s\S]*event\.signal !== null/)
assert.match(session, /this\.serviceMessage\.complete\(\)/)
assert.match(session, /const account = options\.user \|\| 'root'/)
assert.match(session, /keepalive: options\.keepaliveInterval > 0[\s\S]*intervalMs: options\.keepaliveInterval[\s\S]*maxCount: options\.keepaliveCountMax/)
assert.match(session, /environment: options\.environment/)
assert.match(tab, /attachSessionHandler\(session\.serviceMessage\$/)
assert.match(tab, /Object\.entries\(profile\.options\)\.filter\(\(\[key\]\) => key !== 'password'\)/)
assert.doesNotMatch(tab, /safeOptions[\s\S]*password:/)
assert.match(recovery, /recoveryToken\.type === 'app:ssh-tab'/)
assert.match(recovery, /getConfigProxyForProfile\(recoveryToken\.profile\)/)
assert.match(recovery, /savedState: recoveryToken\.savedState/)

const rust = fs.readFileSync(path.join(root, 'src-tauri/src/ssh/mod.rs'), 'utf8')
const engine = fs.readFileSync(path.join(root, 'src-tauri/src/ssh/engine.rs'), 'utf8')
assert.match(rust, /let mut exit_event_emitted = false/)
assert.match(rust, /ChannelMsg::ExitStatus \{ \.\. \}[\s\S]*ChannelMsg::ExitSignal \{ \.\. \}/)
assert.match(rust, /exit_event_emitted \|= is_exit_message/)
assert.match(rust, /if !exit_event_emitted \{[\s\S]*exit_code: None[\s\S]*signal: None/)
assert.match(rust, /use zeroize::Zeroize/)
assert.match(rust, /bytes\.zeroize\(\)/)
assert.match(rust, /text\.zeroize\(\)/)
assert.match(engine, /const AUTH_TIMEOUT: Duration = Duration::from_secs\(120\);/)
assert.match(engine, /tokio::time::timeout\([\s\S]*AUTH_TIMEOUT[\s\S]*authenticate_handle\([\s\S]*SshError::Timeout/)
assert.match(rust, /async fn disconnect_jump_handles\(/)
assert.match(rust, /async fn disconnect_connection\(/)
assert.match(rust, /\*max_count == 0/)

const connectMethod = rust.match(
    /pub async fn connect\([\s\S]*?\r?\n    \}\r?\n\r?\n    pub async fn host_key_decision/,
)
assert.ok(connectMethod, 'SSH connect method contract is missing')
const directConnectBranch = connectMethod[0].match(
    /if request\.jump_chain\.is_empty\(\) \{([\s\S]*?)\r?\n        \} else \{/,
)
assert.ok(directConnectBranch, 'SSH connect branch contract is missing')
assert.match(directConnectBranch[1], /connect_direct_engine\(/)
assert.doesNotMatch(directConnectBranch[1], /\.authenticate\(/)
const jumpConnectBranch = connectMethod[0].match(
    /\} else \{([\s\S]*?)\r?\n        \}\r?\n\r?\n        let channel/,
)
assert.ok(jumpConnectBranch, 'SSH jump branch contract is missing')
assert.match(jumpConnectBranch[1], /connect_over_channel\(/)
assert.match(jumpConnectBranch[1], /for \(index, hop\) in request\.jump_chain\.iter\(\)\.enumerate\(\)\.skip\(1\)\s*\{/)
assert.ok((jumpConnectBranch[1].match(/\.authenticate\(/g) || []).length >= 3, 'SSH jump branch must authenticate first hop, intermediate hops, and target')
assert.match(jumpConnectBranch[1], /disconnect_jump_handles\(/)
assert.match(jumpConnectBranch[1], /disconnect_connection\(/)

console.log('SSH session exit contract passed')
