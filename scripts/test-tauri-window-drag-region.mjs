import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'
import { chromium } from 'playwright-core'

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url))
const projectRoot = path.resolve(scriptDirectory, '..')
const servicePath = path.join(projectRoot, 'tabby-tauri/src/services/windowDragRegion.service.ts')
const browserOverride = process.env.TABBY_CAPTION_TEST_BROWSER

if (process.argv.includes('--help')) {
    console.log(`Usage: node scripts/test-tauri-window-drag-region.mjs

Runs the real Tauri window drag region service against a Chromium DOM and a fake host bridge.
This verifies listener wiring and pointer decisions in the renderer only; it makes no claim about
native GTK dragging or a supported desktop.
Set TABBY_CAPTION_TEST_BROWSER to a Chromium executable, or use Playwright's default.`)
    process.exit(0)
}

const source = fs.readFileSync(servicePath, 'utf8')
const compiled = ts.transpileModule(source, {
    compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2020,
        experimentalDecorators: true,
    },
}).outputText
assert.match(compiled, /exports\.WINDOW_DRAG_REGION_SELECTOR\s*=/, 'TypeScript source must export WINDOW_DRAG_REGION_SELECTOR')
assert.match(compiled, /exports\.TauriWindowDragRegionService\s*=/, 'TypeScript source must export TauriWindowDragRegionService')

const fixtureMarkup = `<!doctype html><html><head><style>
    html, body { margin: 0; }
    .tab-bar { display: block; height: 40px; }
    .preload-logo { display: block; height: 40px; }
    title-bar { display: block; height: 30px; }
    .content { display: block; height: 80px; }
</style></head><body>
    <div class="preload-logo"><div>logo</div></div>
    <div class="tab-bar">
        <tab-header class="tab">tab</tab-header>
        <div class="btn-group"><button class="btn">toolbar</button></div>
        <div class="btn-space background drag"></div>
        <div class="btn-space background"></div>
    </div>
    <title-bar><div class="title">Tabby RS<button class="inner">x</button></div></title-bar>
    <div class="content">terminal</div>
    <div class="modal" style="position: fixed; inset: 0; display: block;"><div class="modal-dialog" style="margin-top: 120px;">dialog</div></div>
</body></html>`

const failures = []
let checksPassed = 0
let browser

function recordFailure (name, error) {
    const message = error instanceof Error ? error.message : String(error)
    failures.push({ name, message })
    console.error(`FAIL ${name}: ${message}`)
}

async function check (name, callback) {
    try {
        await callback()
        checksPassed++
        console.log(`PASS ${name}`)
    } catch (error) {
        recordFailure(name, error)
    }
}

async function installFixture (page) {
    await page.setContent(fixtureMarkup)
    return page.evaluate(compiled => {
        const module = { exports: {} }
        const angular = {
            Injectable: () => value => value,
            Inject: () => () => undefined,
            OnDestroy: class {},
        }
        const hostBridge = { TAURI_RUNTIME_INFO: 'runtimeInfo' }
        const require = name => {
            if (name === '@angular/core') return angular
            if (name === '../api/hostBridge') return hostBridge
            throw new Error('Unprovided dependency: ' + name)
        }
        new Function('module', 'exports', 'require', compiled)(module, module.exports, require)
        const Service = module.exports.TauriWindowDragRegionService
        if (typeof Service !== 'function') throw new Error('the service module did not export a class')

        const fixture = {
            selector: module.exports.WINDOW_DRAG_REGION_SELECTOR,
            create (platform, options = {}) {
                const calls = []
                const bridge = {
                    invoke (command, request) {
                        calls.push({ command, request })
                        if (options.reject) {
                            return Promise.reject(new Error('fixture drag IPC rejected'))
                        }
                        return Promise.resolve(null)
                    },
                }
                const service = new Service(bridge, { platform, startupWindowFrame: options.frame })
                return { service, calls }
            },
            target (selector) {
                const element = document.querySelector(selector)
                if (!element) throw new Error('missing fixture element ' + selector)
                return element
            },
            dispatchDoubleClick (element) {
                let reachedBubbleListener = false
                const observer = () => { reachedBubbleListener = true }
                document.addEventListener('dblclick', observer)
                const event = new MouseEvent('dblclick', { bubbles: true, cancelable: true, button: 0, detail: 2 })
                element.dispatchEvent(event)
                document.removeEventListener('dblclick', observer)
                return { defaultPrevented: event.defaultPrevented, reachedBubbleListener }
            },
            dispatch (element, { button = 0, detail = 1 } = {}) {
                const event = new MouseEvent('mousedown', {
                    bubbles: true,
                    cancelable: true,
                    button,
                    detail,
                })
                element.dispatchEvent(event)
                return event.defaultPrevented
            },
            async scenario (options = {}) {
                const {
                    platform = 'linux',
                    frame = undefined,
                    target = '.btn-space.drag',
                    button = 0,
                    detail = 1,
                    reject = false,
                    initializeCalls = 1,
                    destroy = false,
                    clickable = null,
                    clickableParent = '.preload-logo',
                } = options
                const { service, calls } = fixture.create(platform, { reject, frame })
                let probe = null
                try {
                    if (clickable) {
                        probe = document.createElement(clickable)
                        probe.className = 'probe'
                        const child = document.createElement('span')
                        child.className = 'probe-child'
                        child.textContent = 'probe'
                        probe.appendChild(child)
                        fixture.target(clickableParent).appendChild(probe)
                    }
                    for (let index = 0; index < initializeCalls; index++) {
                        service.initialize()
                    }
                    if (destroy) {
                        service.ngOnDestroy()
                    }
                    const element = probe ? probe.querySelector('.probe-child') : fixture.target(target)
                    const defaultPrevented = fixture.dispatch(element, { button, detail })
                    if (reject) {
                        await new Promise(resolve => setTimeout(resolve, 0))
                    }
                    return {
                        calls: calls.map(call => ({ command: call.command, request: call.request })),
                        defaultPrevented,
                    }
                } finally {
                    probe?.remove()
                    if (!destroy) {
                        service.ngOnDestroy()
                    }
                }
            },
        }
        window.__dragRegionFixture = fixture
        return { selector: fixture.selector }
    }, compiled)
}

async function launchBrowser () {
    const launchOptions = { headless: true }
    if (browserOverride) {
        launchOptions.executablePath = browserOverride
        return chromium.launch(launchOptions)
    }
    try {
        return await chromium.launch(launchOptions)
    } catch (error) {
        // CI images ship Chrome without Playwright's downloaded browser.
        for (const channel of ['chrome', 'chromium']) {
            try {
                return await chromium.launch({ ...launchOptions, channel })
            } catch {}
        }
        throw error
    }
}

async function main () {
    browser = await launchBrowser()
    const page = await browser.newPage()
    const consoleWarnings = []
    const pageErrors = []
    page.on('console', message => {
        if (message.type() === 'warning') consoleWarnings.push(message.text())
    })
    page.on('pageerror', error => pageErrors.push(error.message))

    const installed = await installFixture(page)
    const scenario = options => page.evaluate(options => window.__dragRegionFixture.scenario(options), options)

    await check('the shared selector matches the upstream drag regions', async () => {
        assert.equal(installed.selector, '.tab-bar > .btn-space.drag, title-bar .title, .preload-logo')
        const matched = await page.evaluate(() => [...document.querySelectorAll(window.__dragRegionFixture.selector)]
            .map(element => element.className || element.tagName.toLowerCase()))
        assert.deepEqual(matched, ['preload-logo', 'btn-space background drag', 'title'])
    })

    await check('Linux left press on the tab bar drag space starts a native drag', async () => {
        const result = await scenario({ platform: 'linux', target: '.btn-space.drag', button: 0, detail: 1 })
        assert.equal(result.defaultPrevented, true, 'the press must be consumed before WebKitGTK selects text')
        assert.deepEqual(result.calls, [{ command: 'window.startDragging', request: {} }])
    })

    await check('Linux left press on the title bar text starts a native drag', async () => {
        const result = await scenario({ platform: 'linux', target: 'title-bar .title', button: 0, detail: 1 })
        assert.equal(result.defaultPrevented, true)
        assert.deepEqual(result.calls, [{ command: 'window.startDragging', request: {} }])
    })

    await check('Linux left press on the preload logo starts a native drag', async () => {
        const result = await scenario({ platform: 'linux', target: '.preload-logo', button: 0, detail: 1 })
        assert.equal(result.defaultPrevented, true)
        assert.deepEqual(result.calls, [{ command: 'window.startDragging', request: {} }])
    })

    await check('Linux double press toggles maximization without starting a drag', async () => {
        const result = await scenario({ platform: 'linux', target: '.btn-space.drag', button: 0, detail: 2 })
        assert.equal(result.defaultPrevented, true)
        assert.deepEqual(result.calls, [{ command: 'window.toggleMaximize', request: {} }])
    })

    for (const [button, detail] of [[1, 1], [2, 1], [0, 3], [1, 3], [2, 3]]) {
        await check(`Linux button ${button} detail ${detail} in a drag region is left alone`, async () => {
            const result = await scenario({ platform: 'linux', target: '.btn-space.drag', button, detail })
            assert.equal(result.defaultPrevented, false)
            assert.deepEqual(result.calls, [])
        })
    }

    for (const target of ['.content', 'tab-header.tab', '.btn-group .btn', '.btn-space.background:not(.drag)']) {
        await check(`Linux left press on ${target} outside every drag region is left alone`, async () => {
            const result = await scenario({ platform: 'linux', target, button: 0, detail: 1 })
            assert.equal(result.defaultPrevented, false)
            assert.deepEqual(result.calls, [])
        })
    }

    await check('Linux clickable controls inside a drag region keep their own behavior', async () => {
        const result = await scenario({ platform: 'linux', target: 'title-bar .title .inner', button: 0, detail: 1 })
        assert.deepEqual(result.calls, [])
    })

    for (const clickable of ['button', 'a', 'input', 'select', 'textarea', 'label']) {
        await check(`Linux press under a nested ${clickable} inside a drag region is left alone`, async () => {
            const result = await scenario({ platform: 'linux', clickable, button: 0, detail: 1 })
            assert.deepEqual(result.calls, [])
        })
    }

    for (const platform of ['macos', 'windows', 'web']) {
        await check(`${platform} attaches no drag listener`, async () => {
            const result = await scenario({ platform, target: '.btn-space.drag', button: 0, detail: 1 })
            assert.equal(result.defaultPrevented, false)
            assert.deepEqual(result.calls, [])
        })
    }

    await check('Linux treats the top 38px strip of an open modal as a drag region (upstream 54869e10)', async () => {
        const result = await page.evaluate(() => {
            const fixture = window.__dragRegionFixture
            const { service, calls } = fixture.create('linux', {})
            service.initialize()
            try {
                const press = (target, clientY, detail = 1) => {
                    const event = new MouseEvent('mousedown', { bubbles: true, cancelable: true, button: 0, detail, clientY })
                    target.dispatchEvent(event)
                    return event.defaultPrevented
                }
                const modal = fixture.target('.modal')
                const dialog = fixture.target('.modal-dialog')
                const inStrip = press(modal, 20)
                const belowStrip = press(modal, 60)
                const onDialog = press(dialog, 20)
                document.documentElement.style.setProperty('--spaciness', '2')
                const scaledStrip = press(modal, 60)
                document.documentElement.style.removeProperty('--spaciness')
                const doublePress = press(modal, 20, 2)
                return {
                    inStrip, belowStrip, onDialog, scaledStrip, doublePress,
                    commands: calls.map(call => call.command),
                }
            } finally {
                service.ngOnDestroy()
            }
        })
        assert.equal(result.inStrip, true, 'a press in the strip starts dragging')
        assert.equal(result.belowStrip, false, 'a press below the strip is left to the modal backdrop')
        assert.equal(result.onDialog, false, 'the dialog body itself is not a drag region')
        assert.equal(result.scaledStrip, true, '--spaciness scales the strip height')
        assert.equal(result.doublePress, true)
        assert.deepEqual(result.commands, ['window.startDragging', 'window.startDragging', 'window.toggleMaximize'])
    })

    await check('Linux swallows the shared dblclick on a drag region so one gesture toggles once', async () => {
        const result = await page.evaluate(() => {
            const fixture = window.__dragRegionFixture
            const { service } = fixture.create('linux', {})
            service.initialize()
            try {
                return {
                    region: fixture.dispatchDoubleClick(fixture.target('.btn-space.drag')),
                    content: fixture.dispatchDoubleClick(fixture.target('.content')),
                }
            } finally {
                service.ngOnDestroy()
            }
        })
        assert.deepEqual(result.region, { defaultPrevented: true, reachedBubbleListener: false })
        assert.deepEqual(result.content, { defaultPrevented: false, reachedBubbleListener: true })
    })

    await check('Linux native frame keeps GTK dragging and attaches no listener', async () => {
        const result = await scenario({ platform: 'linux', frame: 'native', target: '.preload-logo', button: 0, detail: 1 })
        assert.equal(result.defaultPrevented, false)
        assert.deepEqual(result.calls, [])
    })

    for (const frame of ['thin', 'full']) {
        await check(`Linux ${frame} frame attaches the drag listener`, async () => {
            const result = await scenario({ platform: 'linux', frame, target: '.btn-space.drag', button: 0, detail: 1 })
            assert.deepEqual(result.calls, [{ command: 'window.startDragging', request: {} }])
        })
    }

    await check('repeated initialize calls attach one listener', async () => {
        const result = await scenario({ platform: 'linux', target: '.btn-space.drag', initializeCalls: 2 })
        assert.deepEqual(result.calls, [{ command: 'window.startDragging', request: {} }])
    })

    await check('ngOnDestroy removes the listener', async () => {
        const result = await scenario({ platform: 'linux', target: '.btn-space.drag', destroy: true })
        assert.equal(result.defaultPrevented, false)
        assert.deepEqual(result.calls, [])
    })

    await check('a rejected drag request warns without throwing', async () => {
        consoleWarnings.length = 0
        pageErrors.length = 0
        const result = await scenario({ platform: 'linux', target: '.btn-space.drag', reject: true })
        assert.deepEqual(result.calls, [{ command: 'window.startDragging', request: {} }])
        await page.waitForTimeout(200)
        assert.ok(consoleWarnings.some(message => message.includes('fixture drag IPC rejected')),
            `expected a console warning mentioning the rejected request, got ${JSON.stringify(consoleWarnings)}`)
        assert.deepEqual(pageErrors, [])
    })

    await page.close()
    console.log(`\nTauri window drag region: ${checksPassed} passed, ${failures.length} failed`)
    if (failures.length > 0) process.exitCode = 1
}

main().catch(error => {
    console.error(`TEST SETUP FAILURE: ${error instanceof Error ? error.stack : String(error)}`)
    process.exitCode = 2
}).finally(async () => {
    if (browser) await browser.close().catch(() => {})
})
