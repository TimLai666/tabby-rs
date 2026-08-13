import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const windowSource = fs.readFileSync(path.join(root, 'app/lib/window.ts'), 'utf8')
const serviceSource = fs.readFileSync(path.join(root, 'tabby-electron/src/services/updater.service.ts'), 'utf8')

assert.match(windowSource, /autoUpdater\.autoDownload\s*=\s*false/)
assert.match(windowSource, /autoUpdater\.autoInstallOnAppQuit\s*=\s*false/)
assert.match(windowSource, /on\('updater:download-update'/)
assert.match(windowSource, /autoUpdater\.downloadUpdate\(\)/)
assert.match(windowSource, /on\('update-available',\s*updateInfo\s*=>[\s\S]*this\.send\('updater:update-available',\s*updateInfo\)/)

assert.match(serviceSource, /send\('updater:download-update'\)/)
assert.doesNotMatch(serviceSource, /onUpdate[\s\S]*downloaded\.then\(/)

console.log('Electron updater policy checks passed')
