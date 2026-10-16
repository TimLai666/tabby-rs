import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'

const source = readFileSync(new URL('./exercise-windows-frame.mjs', import.meta.url), 'utf8')
const start = source.indexOf('    const recoverMouse = (')
const end = source.indexOf('    const interruptedInput = async extra => {', start)
assert.ok(start >= 0 && end > start)
function harness ({ remaining = 100000, recoverOk = true, malformed = false, recoveryMalformed = false, leaveJournal = true } = {}) {
    const files = new Map()
    const calls = []
    const result = { actions: [] }
    const native = vm.runInNewContext(`${source.slice(start, end)}; native`, {
        path, deadline: Date.now() + remaining, result,
        outputDirectory: '/owned', ownerPid: 100, ownerHwnd: 999, ownerCreationUtc: 'owned-original',
        powerShell: 'pwsh', HELPER: '/owned/helper.ps1',
        existsSync: file => files.has(file), readFileSync: file => files.get(file),
        spawnSync: (executable, argv, options) => {
            calls.push({ executable, argv, options })
            const file = argv[argv.indexOf('-OutputFile') + 1]
            const action = argv[argv.indexOf('-Action') + 1]
            if (action === 'RecoverMouse') {
                files.set(file, recoveryMalformed ? 'invalid json' : JSON.stringify({ ok: recoverOk, action, appProcessId: 100, hwnd: 999 }))
                if (recoverOk) files.delete(argv[argv.indexOf('-InputJournal') + 1])
                return { status: recoverOk ? 0 : 1, pid: 556 }
            }
            if (leaveJournal) files.set(file + '.mouse-down', '100\n999\n555')
            if (malformed) files.set(file, 'invalid json')
            return { status: null, pid: 555, error: new Error('spawnSync ETIMEDOUT') }
        },
    })
    return { native, calls, files, result }
}
let checks = 0
{
    const test = harness({ remaining: 40000 })
    assert.throws(() => test.native('Drag'), /insufficient time/)
    assert.equal(test.calls.length, 0)
    checks++
}
{
    const test = harness()
    assert.throws(() => test.native('Drag'), /did not prove/)
    assert.equal(test.calls.length, 2)
    assert.equal(test.calls[0].options.timeout, 25000)
    assert.equal(test.calls[1].options.timeout, 15000)
    assert.equal(test.calls[1].argv.includes('-CreationUtc'), false)
    assert.equal(test.calls[1].argv[test.calls[1].argv.indexOf('-InputHelperPid') + 1], '555')
    assert.equal(test.result.actions[0].recovery.data.ok, true)
    assert.equal([...test.files.keys()].some(file => file.endsWith('.mouse-down')), false)
    checks++
}
{
    const test = harness({ malformed: true })
    assert.throws(() => test.native('Resize'), /JSON/)
    assert.equal(test.calls.length, 2)
    assert.equal([...test.files.keys()].some(file => file.endsWith('.mouse-down')), false)
    checks++
}
{
    const test = harness({ recoverOk: false })
    assert.throws(() => test.native('Drag'), /could not confirm/)
    assert.equal(test.calls.length, 2)
    assert.equal(test.result.actions[0].recovery.data.ok, false)
    assert.equal(test.result.inputCleanupFailed, true)
    checks++
}
{
    const test = harness({ recoveryMalformed: true })
    assert.throws(() => test.native('Drag'), /JSON/)
    assert.equal(test.result.inputCleanupFailed, true)
    assert.equal(test.calls.length, 2)
    checks++
}
{
    const test = harness({ leaveJournal: false })
    assert.throws(() => test.native('Snapshot'), /did not prove/)
    assert.equal(test.calls.length, 1)
    checks++
}
console.log(`Mouse recovery orchestration: ${checks} controlled checks passed; actual Windows release requires runtime evidence`)
