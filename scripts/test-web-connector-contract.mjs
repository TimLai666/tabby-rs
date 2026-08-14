import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const entry = fs.readFileSync(path.join(root, 'web/entry.ts'), 'utf8')
const polyfills = fs.readFileSync(path.join(root, 'web/polyfills.ts'), 'utf8')
const gateway = fs.readFileSync(path.join(root, 'tabby-web/src/services/connectionGateway.service.ts'), 'utf8')
const platform = fs.readFileSync(path.join(root, 'tabby-web/src/platform.ts'), 'utf8')

assert.match(gateway, /export interface WebHostConnector \{[\s\S]*createSocket: \(\.\.\.args: unknown\[\]\) => WebGatewaySocket[\s\S]*loadConfig \(\): Promise<string>[\s\S]*saveConfig \(content: string\): Promise<void>[\s\S]*getAppVersion \(\): string[\s\S]*\}/)
assert.match(entry, /import type \{ WebHostConnector \} from '\.\.\/tabby-web\/src\/services\/connectionGateway\.service'/)
assert.match(entry, /connector: WebHostConnector/)
assert.match(platform, /import type \{ WebHostConnector \} from '\.\/services\/connectionGateway\.service'/)
assert.doesNotMatch(platform, /@Inject\('WEB_CONNECTOR'\) private connector: any/)

assert.match(
    entry,
    /window\['__connector__'\]\s*=\s*options\.connector/,
    'web bootstrap must publish its connector for the browser socket shim',
)
assert.match(
    polyfills,
    /window\['__connector__'\]\.createSocket\(\.\.\.args\)/,
    'browser socket shim must use the bootstrap connector',
)

console.log('Web connector transport contract passed')
