import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'

const root = new URL('..', import.meta.url)
const read = relativePath => readFile(new URL(relativePath, root), 'utf8')

const homeBase = await read('tabby-core/src/services/homeBase.service.ts')
assert.match(homeBase, /github\.com\/TimLai666\/tabby-rs/)
assert.doesNotMatch(homeBase, /github\.com\/Eugeny\/tabby/)

for (const relativePath of [
    'tabby-core/src/components/startPage.component.pug',
    'tabby-core/src/components/titleBar.component.pug',
    'tabby-core/src/components/welcomeTab.component.pug',
    'tabby-settings/src/components/settingsTab.component.pug',
]) {
    const source = await read(relativePath)
    assert.match(source, /Tabby RS/)
}

console.log('Tabby RS branding contract passed')
