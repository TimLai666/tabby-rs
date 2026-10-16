import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'

const source = readFileSync(new URL('./exercise-windows-frame.mjs', import.meta.url), 'utf8')
const start = source.indexOf('    const captionColor = () => {')
const end = source.indexOf('    const recoverMouse = (', start)
assert.ok(start >= 0 && end > start)
const good = () => ({ ok: true, status: 'ok', prerequisiteStatus: 'passed', expectedForeground: '#b050d0', requiredExactPixels: 20,
    exactColorPixelCount: 36, root: { processId: 100, handle: '0x3E7', creationUtc: 'owned-original' },
    button: { role: 'ControlType.Button', name: 'Maximize', processId: 100, enabled: true, offscreen: false }, crop: { width: 46, height: 31 } })
let runtimePath = path.posix
function harness ({ data = good(), status = 0, runError = null, report = true, image = true, malformed = false, shortDeadline = false, pngWidth = 46 } = {}) {
    const files = new Map()
    const calls = []
    const result = {}
    const captionColor = vm.runInNewContext(`${source.slice(start, end)}; captionColor`, {
        path: runtimePath, Buffer, deadline: Date.now() + (shortDeadline ? 100 : 100000), result,
        outputDirectory: '/owned', ownerPid: 100, ownerHwnd: 999, ownerCreationUtc: 'owned-original',
        powerShell: 'powershell.exe', COLOR_HELPER: '/owned/probe-windows-caption-color.ps1',
        timeout: cap => cap, mkdirSync: directory => { assert.equal(directory, runtimePath.join('/owned', 'caption-color')) },
        existsSync: file => files.has(file), readFileSync: file => files.get(file),
        spawnSync: (executable, argv, options) => {
            calls.push({ executable, argv, options })
            if (report) files.set(runtimePath.join('/owned', 'caption-color', 'caption-color.json'), malformed ? 'invalid json' : JSON.stringify(data))
            if (image) {
                const png = Buffer.alloc(24)
                Buffer.from([137,80,78,71,13,10,26,10]).copy(png)
                png.write('IHDR', 12); png.writeUInt32BE(pngWidth,16); png.writeUInt32BE(31,20)
                files.set(runtimePath.join('/owned', 'caption-color', 'caption-color.png'), png)
            }
            return { status, error: runError, stderr: 'probe stderr' }
        },
    })
    return { captionColor, calls, result }
}
let passed = 0
let failed = 0
function check (name, options, expected) {
    const test = harness(options)
    let caught = null
    try { test.captionColor() } catch (error) { caught = error }
    try {
        assert.equal(test.calls.length, options?.shortDeadline ? 0 : 1, 'helper execution count')
        if (expected === null) {
            assert.equal(caught, null)
            assert.equal(test.result.captionColor.data.ok, true)
            const call = test.calls[0]
            assert.equal(call.executable, 'powershell.exe')
            for (const [key,value] of Object.entries({ '-AppProcessId':'100', '-CreationUtc':'owned-original', '-WindowHandle':'999', '-ExpectedForeground':'#b050d0', '-OutputDirectory':runtimePath.join('/owned', 'caption-color') })) assert.equal(call.argv[call.argv.indexOf(key)+1],value)
            assert.equal(call.options.shell,false)
            assert.equal(call.options.windowsHide,true)
            assert.ok(call.options.timeout <= 25000 && call.options.timeout > 0)
            assert.equal(call.options.maxBuffer,1024*1024)
        } else {
            assert.ok(caught, 'failure must stop caller before further gestures')
            assert.match(caught.message, expected)
            if (!options?.shortDeadline) assert.equal(test.result.captionColor.status, options && Object.hasOwn(options, 'status') ? options.status : 0)
        }
        passed++; console.log(`PASS ${name}`)
    } catch (error) { failed++; console.error(`FAIL ${name}: ${error.message}`) }
}
for (const platformPath of [path.posix, path.win32]) {
    runtimePath = platformPath
check('valid same-owned screenshot and color', {}, null)
check('nonzero helper exit cannot pass', { status:1 }, /caption color/)
check('timeout cannot pass', { status:null, runError:new Error('ETIMEDOUT') }, /caption color/)
check('status zero without report', { report:false }, /caption color/)
check('malformed report', { malformed:true }, /caption color|JSON/)
check('status zero without PNG', { image:false }, /caption color/)
check('PNG geometry differs', { pngWidth:45 }, /caption color/)
check('other process report', { data:{ ...good(), root:{ ...good().root, processId:101 } } }, /caption color/)
check('reused process identity', { data:{ ...good(), root:{ ...good().root, creationUtc:'reused' } } }, /caption color/)
check('other root HWND', { data:{ ...good(), root:{ ...good().root, handle:'0x3E8' } } }, /caption color/)
check('other configured color', { data:{ ...good(), expectedForeground:'#ffffff' } }, /caption color/)
check('failed prerequisites cannot pass', { data:{ ...good(), prerequisiteStatus:'failed' } }, /caption color/)
check('insufficient visible color', { data:{ ...good(), exactColorPixelCount:19 } }, /caption color/)
check('impossible pixel count', { data:{ ...good(), exactColorPixelCount:2000 } }, /caption color/)
check('invalid crop dimensions', { data:{ ...good(), crop:{width:257,height:31} } }, /caption color/)
check('caption virtual HWND is allowed', { data:{ ...good(), button:{ ...good().button,nativeWindowHandle:'0x0' } } }, null)
check('insufficient deadline refuses helper', { shortDeadline:true }, /insufficient time/)
}
console.log(`Caption color orchestration: ${passed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
