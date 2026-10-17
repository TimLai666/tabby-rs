import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
const root = process.argv[2] ?? fileURLToPath(new URL('../..', import.meta.url))
const yaml = createRequire(path.join(root, 'package.json'))('js-yaml')
const defaults = yaml.load(readFileSync(path.join(root, 'tabby-core/src/configDefaults.yaml'), 'utf8'))
assert.equal(defaults.appearance.frame, 'thin')
const source = readFileSync(new URL('./run-windows-frame-restart.mjs', import.meta.url), 'utf8')
const matches = [...source.matchAll(/^    const configFrame = (.+)$/gm)]
assert.equal(matches.length, 1)
const cases = [
    [{ appearance: {} }, 'thin', 'a saved default omits its frame field'],
    [{}, 'thin', 'absent appearance uses the original default'],
    [{ appearance: { frame: 'thin' } }, 'thin', 'explicit Thin'],
    [{ appearance: { frame: 'native' } }, 'native', 'explicit Native'],
    [{ appearance: { frame: 'full' } }, 'full', 'explicit Full'],
    [{ appearance: { frame: 'unknown' } }, 'unknown', 'unknown values cannot prove a saved Thin selection'],
    [{ appearance: { frame: '' } }, '', 'empty values cannot prove a saved Thin selection'],
    [{ appearance: { frame: null } }, 'thin', 'null uses the missing-value default'],
]
for (const [config, expected, detail] of cases) {
    const before = JSON.stringify(config)
    const reader = vm.runInNewContext('(' + matches[0][1] + ')', {
        yaml, path, portableDataRoot: path.resolve('/owned/data'),
        readFileSync: (file, encoding) => {
            assert.equal(file, path.join(path.resolve('/owned/data'), 'config.yaml'))
            assert.equal(encoding, 'utf8')
            return yaml.dump(config)
        },
    })
    assert.equal(reader(), expected, detail)
    assert.equal(JSON.stringify(config), before)
}
console.log('Window frame readback: 9 controlled checks passed; sparse Thin defaults and invalid-value rejection retained')
