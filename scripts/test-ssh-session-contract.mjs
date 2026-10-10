import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const session = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/session.ts'), 'utf8')
const tab = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/tab.component.ts'), 'utf8')
const recovery = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/recoveryProvider.ts'), 'utf8')
const tabRecovery = fs.readFileSync(path.join(root, 'tabby-core/src/services/tabRecovery.service.ts'), 'utf8')

const authForOptions = session.match(
    /private async authForOptions \(options: SSHProfile\['options'\]\): Promise<SshAuthMethodRef\[]> \{([\s\S]*?)\n    \}/,
)
assert.ok(authForOptions, 'SSH auth option mapping is missing')
assert.match(authForOptions[1], /if \(!options\.auth\)/)
assert.match(authForOptions[1], /const privateKeys = options\.privateKeys\.length/)
assert.match(authForOptions[1], /auth\.push\(\{ type: 'privateKey', fileRef, passphraseRef: null \}\)/)
assert.match(authForOptions[1], /const socket = await this\.resolveAgentSocket\(\)/)
const resolveAgentSocket = session.match(/private async resolveAgentSocket \(\): Promise<string\|null> \{([\s\S]*?)\n    \}/)
assert.ok(resolveAgentSocket, 'SSH agent socket resolver helper is missing')
assert.match(resolveAgentSocket[1], /this\.bridge\.invoke\('ssh\.resolveAgentSocket', \{ agentType, agentPath \}\)/)
assert.match(authForOptions[1], /auth\.push\(\{ type: 'agent', socket/)
assert.match(authForOptions[1], /auth\.push\(\{ type: 'keyboardInteractive', password: options\.password \}\)/)
assert.match(authForOptions[1], /authMode === 'keyboardInteractive'[\s\S]*auth\.push\(\{ type: 'keyboardInteractive', secretRef: this\.passwordSecretRef\(\) \}\)/)

assert.match(session, /private pendingExit: SshExitEvent\|null = null/)
assert.match(session, /private readonly serviceMessage = new Subject<string>\(\)/)
assert.match(session, /get serviceMessage\$ \(\): Observable<string>/)
assert.match(session, /event\.exitCode !== null[\s\S]*event\.signal !== null/)
assert.match(session, /this\.serviceMessage\.complete\(\)/)
// Upstream resolves prompted/environment usernames before loading saved credentials.
assert.match(session, /'ssh-password:\/\/vault' : 'ssh-password:\/\/keychain'/)
assert.doesNotMatch(session, /options\.user \|\| 'root'/)
assert.match(session, /keepalive: options\.keepaliveInterval > 0[\s\S]*intervalMs: options\.keepaliveInterval[\s\S]*maxCount: options\.keepaliveCountMax/)
assert.match(session, /environment: options\.environment/)
assert.match(tab, /attachSessionHandler\(session\.serviceMessage\$/)
assert.match(tab, /Object\.entries\(profile\.options\)\.filter\(\(\[key\]\) => key !== 'password'\)/)
assert.doesNotMatch(tab.match(/async getRecoveryToken[\s\S]*?return token/)[0], /safeOptions[\s\S]*password:/)
assert.match(recovery, /recoveryToken\.type === 'app:ssh-tab'/)
assert.match(recovery, /getConfigProxyForProfile\(recoveryToken\.profile\)/)
assert.match(recovery, /savedState: recoveryToken\.savedState/)
assert.match(tabRecovery, /tokens = parsed\.map\(token => sanitizeRecoveryToken\(token\) as RecoveryToken\)/)

const rust = fs.readFileSync(path.join(root, 'src-tauri/src/ssh/mod.rs'), 'utf8')
const engine = fs.readFileSync(path.join(root, 'src-tauri/src/ssh/engine.rs'), 'utf8')
// Native lifecycle is covered with real SSH peers in ssh/lifecycle/tests.rs.
// The fixed upstream shell does not terminate on exit status or signal alone.
assert.match(rust, /use zeroize::Zeroize/)
assert.match(rust, /bytes\.zeroize\(\)/)
// Decoding borrows key bytes; the owning material clears them on every exit path.
assert.match(engine, /std::str::from_utf8\(&self\.openssh\)/)
assert.match(engine, /impl Drop for PrivateKeyMaterial[\s\S]*self\.openssh\.zeroize\(\)/)
assert.match(engine, /const AUTH_TIMEOUT: Duration = Duration::from_secs\(120\);/)
assert.match(engine, /tokio::time::timeout\([\s\S]*AUTH_TIMEOUT[\s\S]*authenticate_handle\([\s\S]*SshError::Timeout/)
const engineAgentAuth = engine.match(/async fn authenticate_agent\([^;{]*\) -> Result<bool, SshError> \{[\s\S]*?\r?\n    \}\r?\n/)
assert.ok(engineAgentAuth, 'Engine SSH agent authentication contract is missing')
assert.match(engineAgentAuth[0], /let mut agent = super::connect_agent\(socket\.map\(str::to_owned\)\)\.await\?;/)
assert.doesNotMatch(engineAgentAuth[0], /AgentClient::connect_/)
const connectAgent = rust.match(/async fn connect_agent\(socket: Option<String>\)[\s\S]*?\r?\n\}\r?\n/)
assert.ok(connectAgent, 'Shared SSH agent connector is missing')
const windowsAgent = connectAgent[0].match(/#\[cfg\(windows\)\]\s*let stream = match socket \{([\s\S]*?)\r?\n    \};/)
assert.ok(windowsAgent, 'Windows SSH agent routing contract is missing')
assert.match(windowsAgent[1], /Some\(path\) => AgentClient::connect_named_pipe\(path\)/)
assert.match(windowsAgent[1], /None => AgentClient::connect_pageant\(\)\.await\.into_inner\(\)/)
assert.doesNotMatch(windowsAgent[1], /SSH_AUTH_SOCK|env::var/)
assert.match(connectAgent[0], /AgentClient::connect\(\s*agent_transport::AgentTransport::new\(\s*stream\s*,?\s*\)\s*\)/)
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

console.log('SSH session contract passed')
