import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const service = fs.readFileSync(path.join(root, 'src-tauri/src/update/service.rs'), 'utf8')
const command = fs.readFileSync(path.join(root, 'src-tauri/src/commands/update.rs'), 'utf8')

assert.match(service, /pub generation: u64/)
assert.match(service, /download_generation: u64/)
assert.match(service, /state\.download_generation == generation/)
assert.match(service, /pub fn cancel_download\([^)]*generation: u64\)/)
assert.match(service, /pub fn fail_download\([\s\S]*?generation: u64/)
assert.match(command, /let download_generation = handle\.generation/)
assert.match(command, /set_download_progress\(\s*download_generation,/)
assert.match(command, /finish_download\(download_generation, bytes\)/)
assert.match(command, /cancel_download\(download_generation\)/)
assert.match(command, /fail_download\(\s*download_generation,/)

console.log('Updater download-generation contract passed')
