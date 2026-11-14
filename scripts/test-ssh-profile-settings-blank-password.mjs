// Executes the actual Tauri SSH profile settings component with a fake modal, bridge,
// and password storage. A blank saved password must be confirmed first, like upstream
// sshProfileSettings.component.ts in 256f09dd.
import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import ts from 'typescript'

const source = fs.readFileSync(new URL('../tabby-tauri/src/ssh/profileSettings.component.ts', import.meta.url), 'utf8')
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS, experimentalDecorators: true } }).outputText
const moduleExports = {}
const dependencies = {
    '@angular/core': { Component: () => target => target },
    '@ng-bootstrap/ng-bootstrap': { NgbModal: class {} },
    'tabby-core': { PromptModalComponent: class {} },
    '../../../tabby-ssh/src/api/interfaces': {},
    '../../../tabby-ssh/src/services/passwordStorage.service': {},
    '../api/hostBridge': {},
    './importModal.component': { TauriSshImportModalComponent: class {} },
}
vm.runInNewContext(js, { exports: moduleExports, require: name => { assert.ok(name in dependencies, name); return dependencies[name] }, console })
const { TauriSshProfileSettingsComponent } = moduleExports

function createComponent ({ modalValue, confirmation = 0, stored = null, loadResult = stored }) {
    const saves = []
    const messages = []
    const deletes = []
    const passwordStorage = {
        async loadPassword () { return loadResult },
        async savePassword (profile, value) { saves.push({ profile: profile.options.user, value }) },
        async deletePassword () { deletes.push(true) },
    }
    const modal = {
        componentInstance: {},
        result: modalValue === undefined ? Promise.resolve(null) : Promise.resolve(modalValue),
    }
    const ngbModal = { open () { return modal } }
    const bridge = {
        async invoke (command, request) {
            assert.equal(command, 'dialog.message')
            messages.push(JSON.parse(JSON.stringify(request)))
            return { response: confirmation }
        },
    }
    const component = new TauriSshProfileSettingsComponent(passwordStorage, ngbModal, bridge)
    component.profile = { options: { user: 'alice', host: 'target.test', port: 22, privateKeys: [], forwardedPorts: [], scripts: [] } }
    return { component, saves, messages, deletes }
}

// hasSavedPassword reflects the stored value, including a stored blank password.
{
    const blank = createComponent({ stored: '' })
    await blank.component.ngOnInit()
    assert.equal(blank.component.hasSavedPassword, true, 'a stored blank password is a saved password')
    const absent = createComponent({ stored: null })
    await absent.component.ngOnInit()
    assert.equal(absent.component.hasSavedPassword, false, 'nothing stored is not a saved password')
    const present = createComponent({ stored: 'synthetic' })
    await present.component.ngOnInit()
    assert.equal(present.component.hasSavedPassword, true)
    console.log('PASS hasSavedPassword is true for a stored blank password and false for nothing stored')
}

// Blank entry asks the upstream confirmation question and saves only on confirmation.
{
    const confirmed = createComponent({ modalValue: { value: '' }, confirmation: 0 })
    await confirmed.component.setPassword()
    assert.equal(confirmed.messages.length, 1, 'blank entry asks for confirmation once')
    assert.deepEqual(confirmed.messages[0], {
        type: 'warning',
        message: 'Save an empty password?',
        detail: 'This only works if the server permits empty passwords. Tabby will offer it on every connection.',
        buttons: ['Save', 'Cancel'],
        defaultId: 0,
        cancelId: 1,
    })
    assert.deepEqual(confirmed.saves, [{ profile: 'alice', value: '' }], 'confirmed blank password is saved')
    assert.equal(confirmed.component.hasSavedPassword, true)
    console.log('PASS blank password is saved after confirmation')

    const declined = createComponent({ modalValue: { value: '' }, confirmation: 1 })
    await declined.component.setPassword()
    assert.equal(declined.messages.length, 1)
    assert.deepEqual(declined.saves, [], 'declined blank password is not saved')
    assert.equal(declined.component.hasSavedPassword, false)
    console.log('PASS declined blank password is not saved')
}

// Non-blank and cancelled entries keep the existing behavior without a confirmation.
{
    const entered = createComponent({ modalValue: { value: 'synthetic-entry' } })
    await entered.component.setPassword()
    assert.deepEqual(entered.messages, [], 'non-blank entry is not confirmed')
    assert.deepEqual(entered.saves, [{ profile: 'alice', value: 'synthetic-entry' }])
    assert.equal(entered.component.hasSavedPassword, true)
    console.log('PASS non-blank password is saved without confirmation')

    const cancelled = createComponent({ modalValue: null })
    await cancelled.component.setPassword()
    assert.deepEqual(cancelled.messages, [])
    assert.deepEqual(cancelled.saves, [], 'cancelled modal saves nothing')
    console.log('PASS cancelled modal saves nothing')
}

console.log('ssh profile settings blank-password tests passed')
