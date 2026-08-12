import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = fs.readFileSync(path.join(root, 'tabby-core/src/components/appRoot.component.ts'), 'utf8')
const settings = fs.readFileSync(path.join(root, 'tabby-settings/src/components/settingsTab.component.pug'), 'utf8')

const refreshMethod = source.match(/private async refreshUpdateAvailability \(\): Promise<void> \{([\s\S]*?)\n    \}/)?.[1]
assert.ok(refreshMethod, 'startup update refresh method must exist')
assert.match(source, /config\.ready\$\.toPromise\(\)\.then\(async \(\) => \{[\s\S]*?void this\.refreshUpdateAvailability\(\)/)
assert.match(refreshMethod, /capabilities\.updater/)
assert.match(refreshMethod, /enableAutomaticUpdates/)
assert.match(refreshMethod, /await this\.updater\.check\(\)/)
assert.doesNotMatch(refreshMethod, /updater\.(download|install|update)\(/)
assert.match(settings, /\{\{updateAvailable\.version\}\}/)
assert.match(settings, /\{\{updateAvailable\.notes\}\}/)
assert.match(settings, /\(click\)='installUpdate\(\)'/)

console.log('Updater startup-check contract passed')
