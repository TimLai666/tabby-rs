import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const platformPath = path.join(root, 'web/platform.ts')
const platformSource = ts.transpileModule(fs.readFileSync(platformPath, 'utf8'), {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
    },
    fileName: platformPath,
}).outputText
const platformModule = { exports: {} }
vm.runInNewContext(platformSource, {
    exports: platformModule.exports,
    module: platformModule,
}, { filename: platformPath })

const { detectWebProcessPlatform } = platformModule.exports
for (const [userAgent, expected] of [
    ['Mozilla/5.0 (Macintosh; Intel Mac OS X 13_5)', 'darwin'],
    ['Mozilla/5.0 (Windows NT 10.0; Win64; x64)', 'win32'],
    ['Mozilla/5.0 (X11; Linux x86_64)', 'linux'],
    ['Mozilla/5.0 (Linux; Android 14; Pixel 8)', 'linux'],
]) {
    assert.equal(detectWebProcessPlatform(userAgent), expected)
}

const preloadSource = fs.readFileSync(path.join(root, 'web/entry.preload.ts'), 'utf8')
assert.match(preloadSource, /import \{ detectWebProcessPlatform \} from '\.\/platform'/)
assert.match(preloadSource, /platform: detectWebProcessPlatform\(window\.navigator\.userAgent\)/)
assert.doesNotMatch(preloadSource, /platform: 'darwin'/)

console.log('Web process platform contract passed')
