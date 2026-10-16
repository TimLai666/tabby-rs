import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'

// Optional first argument selects the capture-controller source under review.
const source = readFileSync(new URL(process.argv[2] ?? './capture-windows-apps.mjs', import.meta.url), 'utf8')
const start = source.indexOf('function cdpListeners (context, port) {')
const end = source.indexOf('function pickPage (browser) {', start)
assert.ok(start >= 0 && end > start)
let tree
let listeners
let queryOk
const { cdpListeners, verifyCdpOwnership } = vm.runInNewContext(`${source.slice(start, end)}; ({ cdpListeners, verifyCdpOwnership })`, {
    snapshotProcessTree: () => tree,
    runPowerShell: () => ({ ok: queryOk, stdout: JSON.stringify(listeners), stderr: 'query failed' }),
})
const root = { pid: 100, creationTimeUTC: 'root-original' }
const webview = { pid: 101, creationTimeUTC: 'child-original' }
const context = { deadline: Date.now() + 10000, powerShell: 'unused.exe' }
let checks = 0
function check (callback) {
    tree = { processes: [root, webview], error: null }
    listeners = [root]
    queryOk = true
    callback({ pid: root.pid, rootIdentity: root, cdp: {} })
    checks++
}
check(info => { verifyCdpOwnership(context, info, 9222); assert.equal(info.cdp.ownership.verified, true) })
check(info => { listeners = [webview]; verifyCdpOwnership(context, info, 9223); assert.equal(info.cdp.ownership.verified, true) })
check(info => { listeners = [{ pid: 200, creationTimeUTC: 'foreign' }]; assert.throws(() => verifyCdpOwnership(context, info, 9222), /exclusively owned/) })
check(info => { listeners = [root, { pid: 200, creationTimeUTC: 'foreign' }]; assert.throws(() => verifyCdpOwnership(context, info, 9222), /exclusively owned/) })
check(info => { listeners = [{ ...webview, creationTimeUTC: 'child-reused' }]; assert.throws(() => verifyCdpOwnership(context, info, 9222), /exclusively owned/) })
check(info => { tree.processes = [{ ...root, creationTimeUTC: 'root-reused' }]; assert.throws(() => verifyCdpOwnership(context, info, 9222), /tree identity changed/) })
check(info => { listeners = []; assert.throws(() => verifyCdpOwnership(context, info, 9222), /exclusively owned/) })
check(info => { queryOk = false; assert.throws(() => verifyCdpOwnership(context, info, 9222), /query failed/) })
check(() => { assert.throws(() => cdpListeners(context, 0), /invalid CDP/); assert.throws(() => cdpListeners(context, 65536), /invalid CDP/) })
console.log(`CDP process ownership: ${checks} controlled checks passed; Windows listener enumeration remains a separate runtime check`)
