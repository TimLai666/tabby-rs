import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const bundlePath = path.join(root, 'app/dist-tauri/bundle.js')
assert.ok(fs.existsSync(bundlePath), `Tauri bundle does not exist: ${bundlePath}`)

const bundle = fs.readFileSync(bundlePath, 'utf8')
const forbiddenRuntimeImports = [
    /(?:require|import)\s*\(\s*["'](?:electron|@electron\/[^"']+)["']\s*\)/,
    /(?:require|import)\s*\(\s*["']node:(?:assert|child_process|cluster|crypto|dgram|dns|fs|http|https|module|net|os|path|perf_hooks|process|readline|stream|tls|tty|url|util|vm|worker_threads|zlib)["']\s*\)/,
    /from\s*["'](?:electron|@electron\/[^"']+)["']/,
    /from\s*["']node:(?:assert|child_process|cluster|crypto|dgram|dns|fs|http|https|module|net|os|path|perf_hooks|process|readline|stream|tls|tty|url|util|vm|worker_threads|zlib)["']/,
]

for (const pattern of forbiddenRuntimeImports) {
    assert.equal(pattern.test(bundle), false, `Tauri bundle contains a forbidden runtime import: ${pattern}`)
}

console.log(`Tauri bundle runtime audit passed (${bundlePath}, ${bundle.length} characters)`)
