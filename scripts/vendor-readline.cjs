// Run with ELECTRON_RUN_AS_NODE=1 node_modules/electron/dist/Electron.app/Contents/MacOS/Electron.
// Keep the runtime pinned: the checked-in source and ICU widths are reference data.
const fs = require('fs')
const path = require('path')
const assert = require('assert')
const crypto = require('crypto')
assert.equal(process.versions.electron, '38.8.6')
assert.equal(process.versions.node, '22.22.0')
assert.equal(process.versions.icu, '74.2')
const target = path.resolve(__dirname, '../app/src/shims/readline')
fs.mkdirSync(target, { recursive: true })
const natives = process.binding('natives')
const license = natives.readline.slice(0, natives.readline.indexOf("'use strict'"))
const hashes = {}
for (const name of ['interface', 'utils', 'callbacks', 'emitKeypressEvents']) {
    const original = natives[`internal/readline/${name}`]
    hashes[name] = crypto.createHash('sha256').update(original).digest('hex')
    let source = original.replace("'use strict';", "'use strict';\nconst { primordials } = require('./runtime.cjs');")
    source = source.replace(/require\('internal\/readline\/(\w+)'\)/g, "require('./$1.cjs')")
    source = source.replace(/require\('internal\/[^']+'\)/g, "require('./runtime.cjs')")
    source = source.replace("const { clearTimeout, setTimeout } = require('timers');", '// Browser timers are globals.')
    fs.writeFileSync(path.join(target, `${name}.cjs`), license + source)
}
const ranges = []
const icu = process.binding('icu')
for (let cp = 0; cp <= 0x10ffff; cp++) {
    const width = icu.getStringWidth(String.fromCodePoint(cp))
    if (width === 1) continue
    const last = ranges[ranges.length - 1]
    if (last && last[1] === cp - 1 && last[2] === width) last[1] = cp
    else ranges.push([cp, cp, width])
}
fs.writeFileSync(path.join(target, 'widths.json'), JSON.stringify(ranges) + '\n')
const inspect = natives['internal/util/inspect']
const ansi = inspect.slice(inspect.indexOf('const ansi ='), inspect.indexOf('\n\n', inspect.indexOf('const ansi =')))
fs.writeFileSync(path.join(target, 'ansi.cjs'), license + ansi + '\nmodule.exports = ansi;\n')
fs.writeFileSync(path.join(target, 'LICENSE'), license.replace(/^\/\/? ?/gm, '').trimEnd() + '\n')
fs.writeFileSync(path.join(target, 'provenance.json'), JSON.stringify({
    electron: process.versions.electron, node: process.versions.node, icu: process.versions.icu,
    unicode: process.versions.unicode, sourceHashes: hashes,
    widthsHash: crypto.createHash('sha256').update(JSON.stringify(ranges) + '\n').digest('hex'),
}, null, 2) + '\n')
