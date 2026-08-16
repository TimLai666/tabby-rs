import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-tauri/src/ssh/tab.component.ts'), 'utf8')

assert.match(source, /private reconnectAttempts = 0/)
assert.match(source, /private reconnectTimer: ReturnType<typeof setTimeout>\|null = null/)
assert.match(source, /await session\.start\(\)[\s\S]*this\.reconnectAttempts = 0/)
assert.match(
    source,
    /protected onSessionDestroyed \(\): void \{[\s\S]*this\.profile\.behaviorOnSessionEnd === 'reconnect'[\s\S]*!this\.isDisconnectedByHand/,
)
assert.match(source, /this\.reconnectAttempts < 5/)
assert.match(source, /Math\.min\(30_000, 1_000 \* 2 \*\* this\.reconnectAttempts\)/)
assert.match(source, /this\.reconnectTimer = setTimeout\(\(\) => \{[\s\S]*this\.reconnectTimer = null[\s\S]*this\.isDisconnectedByHand[\s\S]*void this\.reconnect\(\)/)
assert.match(source, /this\.cancelReconnectTimer\(\)/)
assert.match(source, /async disconnect \(\): Promise<void> \{[\s\S]*await super\.disconnect\(\)/)
assert.match(source, /ngOnDestroy \(\): void \{[\s\S]*super\.ngOnDestroy\(\)/)
assert.match(source, /clearTimeout\(this\.reconnectTimer\)/)
assert.match(source, /this\.offerReconnection\(\)/)
assert.match(source, /super\.onSessionDestroyed\(\)/)

const sharedReconnectSource = fs.readFileSync(
    path.join(root, 'tabby-terminal/src/api/connectableTerminalTab.component.ts'),
    'utf8',
)
assert.match(
    sharedReconnectSource,
    /async reconnect \(\): Promise<void> \{\s*this\.isDisconnectedByHand = true\s*await this\.session\?\.destroy\(\)/,
)

console.log('SSH reconnect contract passed')
