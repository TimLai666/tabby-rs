import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

for (const [kind, label] of [['serial', 'Serial'], ['telnet', 'Telnet']]) {
    const source = fs.readFileSync(path.join(root, `tabby-tauri/src/${kind}/tab.component.ts`), 'utf8')

    assert.match(source, /private reconnectAttempts = 0/)
    assert.match(source, /private reconnectTimer: ReturnType<typeof setTimeout>\|null = null/)
    assert.match(source, /async initializeSession \(\): Promise<void> \{[\s\S]*this\.cancelReconnectTimer\(\)/)
    assert.match(source, /await session\.start\(\)[\s\S]*this\.reconnectAttempts = 0[\s\S]*this\.cancelReconnectTimer\(\)/)
    assert.match(
        source,
        /protected onSessionDestroyed \(\): void \{[\s\S]*this\.profile\.behaviorOnSessionEnd === 'reconnect'[\s\S]*!this\.isDisconnectedByHand/,
    )
    assert.match(source, /this\.reconnectAttempts < 5/)
    assert.match(source, /Math\.min\(30_000, 1_000 \* 2 \*\* this\.reconnectAttempts\)/)
    assert.match(source, /this\.reconnectTimer = setTimeout\(\(\) => \{[\s\S]*this\.reconnectTimer = null[\s\S]*this\.isDisconnectedByHand[\s\S]*void this\.reconnect\(\)/)
    assert.match(source, /async disconnect \(\): Promise<void> \{[\s\S]*this\.cancelReconnectTimer\(\)[\s\S]*await super\.disconnect\(\)/)
    assert.match(source, /ngOnDestroy \(\): void \{[\s\S]*this\.cancelReconnectTimer\(\)[\s\S]*super\.ngOnDestroy\(\)/)
    assert.match(source, /private cancelReconnectTimer \(\): void \{[\s\S]*clearTimeout\(this\.reconnectTimer\)/)
    assert.match(source, /this\.offerReconnection\(\)/)
    assert.match(source, /super\.onSessionDestroyed\(\)/)

    console.log(`${label} reconnect contract passed`)
}
