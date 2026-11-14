import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

// Native serial quick connect must parse and print like upstream
// tabby-serial/src/profiles.ts (c9a7a54f): `port@baud` or `port:baud`,
// unparsable rates fall back to 115200, and the default rate prints bare.
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const source = file => fs.readFileSync(path.join(root, file), 'utf8')

function methods (file, className, names) {
    const tree = ts.createSourceFile(file, source(file), ts.ScriptTarget.Latest, true)
    const node = tree.statements.find(x => ts.isClassDeclaration(x) && x.name?.text === className)
    assert.ok(node, `${className} must exist in ${file}`)
    const printer = ts.createPrinter()
    const body = node.members
        .filter(member => ts.isMethodDeclaration(member) && names.includes(member.name.getText(tree)))
        .map(member => printer.printNode(ts.EmitHint.Unspecified, member, tree))
        .join('\n')
    assert.equal(body.split('\n').filter(line => /^\s{4}\w+ \(/.test(line)).length, names.length, `${className} must define ${names.join(', ')}`)
    const js = ts.transpileModule(`class Probe {\n${body}\n}\nmodule.exports = Probe`, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS } }).outputText
    const module = { exports: {} }
    vm.runInNewContext(js, { module, exports: module.exports, parseInt, Number })
    return new module.exports()
}

const names = ['quickConnect', 'intoQuickConnectString']
const native = methods('tabby-tauri/src/serial/profiles.ts', 'TauriSerialProfilesService', names)
const upstream = methods('tabby-serial/src/profiles.ts', 'SerialProfilesService', names)
const rate = profile => profile.options.baudRate ?? profile.options.baudrate
const asNative = profile => ({ options: { port: profile.options.port, baudRate: rate(profile) } })
const asUpstream = profile => ({ options: { port: profile.options.port, baudrate: rate(profile) } })

const queries = ['/dev/ttyUSB0', '/dev/ttyUSB0@9600', '/dev/ttyUSB0:57600', 'COM3@abc', 'COM3:', '@115200', '/dev/tty@9600:1', 'COM3:9600@1']
for (const query of queries) {
    const mine = native.quickConnect(query)
    const theirs = upstream.quickConnect(query)
    assert.equal(mine.options.port, theirs.options.port, `${query}: port`)
    assert.equal(rate(mine), rate(theirs), `${query}: baud rate`)
    assert.equal(mine.name, theirs.name, `${query}: name`)
    assert.equal(mine.type, 'serial')
    assert.equal(native.intoQuickConnectString(asNative(mine)), upstream.intoQuickConnectString(asUpstream(theirs)), `${query}: round trip string`)
}
for (const profile of [
    { port: '/dev/ttyUSB0', baudRate: 115200 },
    { port: '/dev/ttyUSB0', baudRate: 9600 },
    { port: '/dev/ttyUSB0', baudRate: undefined },
    { port: '', baudRate: 9600 },
    { port: undefined, baudRate: 9600 },
]) {
    assert.equal(
        native.intoQuickConnectString({ options: profile }),
        upstream.intoQuickConnectString({ options: { port: profile.port, baudrate: profile.baudRate } }),
        `${JSON.stringify(profile)}: quick connect string`,
    )
}
console.log(`serial quick connect: ${queries.length} queries and 5 profiles match upstream tabby-serial`)
