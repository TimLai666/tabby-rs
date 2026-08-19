import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const installer = fs.readFileSync(path.join(root, 'scripts/ci/install-linux-dependencies.sh'), 'utf8')
const workflows = [
  fs.readFileSync(path.join(root, '.github/workflows/build.yml'), 'utf8'),
  fs.readFileSync(path.join(root, '.github/workflows/release.yml'), 'utf8'),
  fs.readFileSync(path.join(root, '.github/workflows/docs.yml'), 'utf8'),
]

assert.match(installer, /set -Eeuo pipefail/)
assert.match(installer, /timeout --signal=TERM --kill-after=30s/)
assert.match(installer, /Acquire::Retries=3/)
assert.match(installer, /apt_retries/)
assert.match(installer, /apt_retry_delay_seconds/)
assert.match(installer, /::error::apt-get/)

for (const workflow of workflows) {
  assert.doesNotMatch(workflow, /sudo apt-get (?:update|install)/)
  assert.match(workflow, /bash scripts\/ci\/install-linux-dependencies\.sh/)
}

console.log('Linux dependency installer contract passed')
