#!/usr/bin/env node

import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'

const root = process.cwd()
const corePackagePath = path.join(root, 'tabby-core', 'package.json')
const compilerPackagePath = path.join(root, 'tabby-core', 'node_modules', 'ngx-translate-messageformat-compiler', 'package.json')

const corePackage = JSON.parse(fs.readFileSync(corePackagePath, 'utf8'))
const compilerPackage = JSON.parse(fs.readFileSync(compilerPackagePath, 'utf8'))
const compilerVersion = Number.parseInt(compilerPackage.version.split('.')[0], 10)
const compilerModulePath = path.join(path.dirname(compilerPackagePath), compilerPackage.module)

assert.match(corePackage.devDependencies['ngx-translate-messageformat-compiler'], /^\^7\./)
assert.equal(compilerVersion, 7)
assert.match(compilerPackage.peerDependencies['@ngx-translate/core'], /\^14\.0\.0/)
assert.equal(corePackage.devDependencies['@messageformat/core'], '^3.2.0')
assert.ok(fs.existsSync(compilerModulePath), `compiler module is missing: ${compilerModulePath}`)
const compilerModule = fs.readFileSync(compilerModulePath, 'utf8')
assert.doesNotMatch(compilerModule, /\b__extends\b/, 'the compiler must use the native ESM class entry')

console.log(`translate compiler compatibility passed (${compilerPackage.version})`)
