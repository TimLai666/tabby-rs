import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const component = fs.readFileSync(path.join(root, 'tabby-terminal/src/components/loginScriptsSettings.component.ts'), 'utf8')
const template = fs.readFileSync(path.join(root, 'tabby-terminal/src/components/loginScriptsSettings.component.pug'), 'utf8')

assert.match(component, /CdkDragDrop, moveItemInArray/)
assert.match(component, /dropScript \(event: CdkDragDrop<LoginScript\[\]>\): void/)
assert.match(component, /moveItemInArray\(this\.scripts, event\.previousIndex, event\.currentIndex\)/)
assert.match(template, /cdkDropList/)
assert.match(template, /\(cdkDropListDropped\)='dropScript\(\$event\)'/)
assert.match(template, /cdkDragHandle/)
assert.doesNotMatch(template, /TODO.*sortablejs/)

console.log('Login scripts ordering contract passed')
