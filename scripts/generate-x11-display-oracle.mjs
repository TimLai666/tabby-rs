#!/usr/bin/env node
// Regenerates src-tauri/src/ssh/fixtures/x11-display.json from the upstream
// X11Socket.resolveDisplaySpec in this checkout, for Unix and Windows, keeping
// the existing display list. Run after syncing tabby-ssh/src/session/x11.ts.
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { execSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const fixturePath = path.join(root, 'src-tauri/src/ssh/fixtures/x11-display.json')
const sourcePath = 'tabby-ssh/src/session/x11.ts'
const source = fs.readFileSync(path.join(root, sourcePath), 'utf8')
const tree = ts.createSourceFile('x11.ts', source, ts.ScriptTarget.Latest, true)
const cls = tree.statements.find(s => ts.isClassDeclaration(s) && s.name?.text === 'X11Socket')
const method = cls.members.find(m => ts.isMethodDeclaration(m) && m.name.getText(tree) === 'resolveDisplaySpec')
const body = ts.createPrinter().printNode(ts.EmitHint.Unspecified, method, tree)
const js = ts.transpileModule(`class Probe {\n${body}\n}\nmodule.exports = Probe`, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS } }).outputText

function resolver (platform) {
    const module = { exports: {} }
    vm.runInNewContext(js, { module, exports: module.exports, process: { platform, env: {} }, parseInt })
    return spec => module.exports.resolveDisplaySpec(spec)
}
const unix = resolver('linux')
const windows = resolver('win32')

const previous = JSON.parse(fs.readFileSync(fixturePath, 'utf8'))
// Keep the existing case order so the fixture diff shows only changed results.
const cases = previous.cases.map(c => ({ display: c.display, windows: c.windows, expected: (c.windows ? windows : unix)(c.display) }))
const baseline = execSync('git rev-parse HEAD', { cwd: root }).toString().trim()
const lines = cases.map(c => `    ${JSON.stringify({ display: c.display, windows: c.windows, expected: c.expected })}`)
fs.writeFileSync(fixturePath, `{\n  "baseline": ${JSON.stringify(process.env.X11_ORACLE_BASELINE ?? baseline)},\n  "source": ${JSON.stringify(sourcePath)},\n  "cases": [\n${lines.join(',\n')}\n  ]\n}\n`)
console.log(`x11 display oracle: ${cases.length} cases from ${sourcePath}`)
