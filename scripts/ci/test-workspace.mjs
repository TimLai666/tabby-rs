#!/usr/bin/env node

import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import vm from 'node:vm'
import ts from 'typescript'

const source = await readFile('tabby-core/src/workspace.ts', 'utf8')
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
        strict: true,
    },
    fileName: 'workspace.ts',
})

const module = { exports: {} }
vm.runInNewContext(compiled.outputText, { module, exports: module.exports }, { filename: 'workspace.cjs' })
const { createWorkspaceSnapshot, normalizeRatios, sanitizeRecoveryToken, validateWorkspaceSnapshot, workspaceReducer } = module.exports

assert.deepEqual([...normalizeRatios([0, 0, 0], 3)], [1 / 3, 1 / 3, 1 / 3])
const first = { schemaVersion: 1, tabId: 'one', profileId: 'local:one', sessionKind: 'local', sessionState: {} }
const second = {
    schemaVersion: 1,
    tabId: 'two',
    profileId: 'ssh:two',
    sessionKind: 'ssh',
    sessionState: { auth: { password: 'must-not-persist', nested: { token: 'must-not-persist' } }, keep: 'safe' },
}
const snapshot = createWorkspaceSnapshot([first, second], 'one')
assert.equal(snapshot.schemaVersion, 1)
assert.equal(snapshot.layout.type, 'split')
assert.equal(JSON.stringify(snapshot.tabs[1].sessionState), JSON.stringify({ auth: { nested: {} }, keep: 'safe' }))
assert.equal(JSON.stringify(sanitizeRecoveryToken({
    type: 'app:ssh-tab',
    profile: { options: { password: 'must-not-persist', privateKeys: ['private-key'], keep: 'safe' } },
    savedState: { auth: { password: 'must-not-persist', token: 'must-not-persist' }, keep: 'safe' },
})), JSON.stringify({
    type: 'app:ssh-tab',
    profile: { options: { keep: 'safe' } },
    savedState: { auth: {}, keep: 'safe' },
}))
assert.equal(validateWorkspaceSnapshot({ ...snapshot, layout: { ...snapshot.layout, ratios: [1] } }).layout.ratios.length, 2)
assert.equal(validateWorkspaceSnapshot({ ...snapshot, tabs: [{ ...snapshot.tabs[0], tabId: 'duplicate' }, snapshot.tabs[1]] }), null)
const split = workspaceReducer(snapshot, { type: 'split', tabId: 'one', direction: 'vertical', tab: { schemaVersion: 1, tabId: 'three', profileId: 'local:three', sessionKind: 'local', sessionState: {} } })
assert.equal(split.tabs.length, 3)
assert.equal(workspaceReducer(split, { type: 'focus', tabId: 'three' }).activeTabId, 'three')
assert.equal(workspaceReducer(split, { type: 'resize', path: [0], index: 0, delta: 0.1 }).schemaVersion, 1)
const moved = workspaceReducer(split, { type: 'move', tabId: 'three', targetTabId: 'one', direction: 'horizontal', before: true })
assert.equal(moved.tabs.length, 3)
const unsplit = workspaceReducer(moved, { type: 'unsplit', tabId: 'two' })
assert.equal(validateWorkspaceSnapshot(unsplit)?.tabs.length, 3)

console.log('Workspace tests passed.')
