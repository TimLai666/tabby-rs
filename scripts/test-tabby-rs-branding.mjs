import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'

const root = new URL('..', import.meta.url)
const read = relativePath => readFile(new URL(relativePath, root), 'utf8')

const tauriConfig = JSON.parse(await read('src-tauri/tauri.conf.json'))
assert.equal(tauriConfig.productName, 'Tabby RS')
assert.equal(tauriConfig.identifier, 'io.tabbyrs.app')
assert.equal(tauriConfig.app.windows[0].title, 'Tabby RS')
assert.ok(tauriConfig.bundle.icon.includes('../build/mac/icon.icns'))
assert.deepEqual(tauriConfig.plugins['deep-link'].desktop.schemes, ['tabby-rs'])

const homeBase = await read('tabby-core/src/services/homeBase.service.ts')
assert.match(homeBase, /github\.com\/TimLai666\/tabby-rs/)
assert.doesNotMatch(homeBase, /github\.com\/Eugeny\/tabby/)

const canonicalLogo = await read('app/assets/logo.svg')
assert.match(canonicalLogo, /#E84A8A/)
assert.match(canonicalLogo, /#E6C34A/)
assert.doesNotMatch(canonicalLogo, /<text|\bRS\b/)

const preloadStyle = await read('app/src/preload.scss')
assert.match(preloadStyle, /background: url\('\.\.\/assets\/logo\.svg'\)/)
assert.match(preloadStyle, /color: #E6C34A/)

const legacyEntry = await read('app/index.pug')
assert.match(legacyEntry, /h1\.tabby-title Tabby RS/)
assert.doesNotMatch(legacyEntry, /sup α/)

const legacyLogo = await read('build/icons/icon.svg')
assert.match(legacyLogo, /#E84A8A/)
assert.match(legacyLogo, /<rect[^>]+fill="#000"/)
assert.doesNotMatch(legacyLogo, /<text|\bRS\b/)

for (const relativePath of [
    'tabby-core/src/components/startPage.component.pug',
    'tabby-core/src/components/titleBar.component.pug',
    'tabby-core/src/components/welcomeTab.component.pug',
    'tabby-settings/src/components/settingsTab.component.pug',
]) {
    const source = await read(relativePath)
    assert.match(source, /Tabby RS/)
    assert.doesNotMatch(source, /sup α/)
}

console.log('Tabby RS branding contract passed')
