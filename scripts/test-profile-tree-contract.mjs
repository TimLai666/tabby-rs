import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const component = fs.readFileSync(path.join(root, 'tabby-core/src/components/profileTree.component.ts'), 'utf8')
const template = fs.readFileSync(path.join(root, 'tabby-core/src/components/profileTree.component.pug'), 'utf8')

assert.match(component, /activeProfileId: string\|null = null/)
assert.match(component, /subscribeUntilDestroyed\(this\.app\.tabsChanged\$/)
assert.match(component, /subscribeUntilDestroyed\(this\.app\.activeTabChange\$/)
assert.match(component, /let activeTab = this\.app\.activeTab as ProfileTab\|null/)
assert.match(component, /activeTab\.getFocusedTab\(\)/)
assert.match(component, /this\.activeProfileId = activeTab\?\.profile\?\.id \?\? null/)
assert.doesNotMatch(component, /TODO: show active tab in the side panel with eye icon/)
assert.match(template, /\.fw-20\.text-muted\(\*ngIf='activeProfileId === profile\.id'\)/)
assert.match(template, /\.fa\.fa-fw\.fas\.fa-eye/)

console.log('Profile tree active-tab contract passed')
