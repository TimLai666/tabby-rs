import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'
import { chromium } from 'playwright-core'

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url))
const projectRoot = path.resolve(scriptDirectory, '..')
const rendererPath = path.join(projectRoot, 'tabby-tauri/src/services/windowsCaption.ts')
const browserOverride = process.env.TABBY_CAPTION_TEST_BROWSER

if (process.argv.includes('--help')) {
    console.log(`Usage: node scripts/test-windows-caption-renderer.mjs

Runs pixel assertions against the pure caption renderer in real Chromium Canvas2D.
This verifies browser canvas pixels only; it makes no claim about native desktop rendering.
Set TABBY_CAPTION_TEST_BROWSER to a Chromium executable, or use Playwright's default.
Set TABBY_CAPTION_TEST_EVIDENCE_DIR to an empty directory for saved PNG evidence.`)
    process.exit(0)
}

const source = fs.readFileSync(rendererPath, 'utf8')
const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText
assert.match(compiled, /exports\.renderWindowsCaption\s*=/, 'TypeScript source must export renderWindowsCaption')

const configuredEvidenceDirectory = process.env.TABBY_CAPTION_TEST_EVIDENCE_DIR
const evidenceDirectory = configuredEvidenceDirectory
    ? path.resolve(configuredEvidenceDirectory)
    : fs.mkdtempSync(path.join(os.tmpdir(), 'tabby-caption-canvas-'))
fs.mkdirSync(evidenceDirectory, { recursive: true })
if (configuredEvidenceDirectory && fs.readdirSync(evidenceDirectory).length !== 0) {
    throw new Error(`Evidence directory must be empty: ${evidenceDirectory}`)
}

const failures = []
let checksPassed = 0
let browser

function recordFailure(name, error) {
    const message = error instanceof Error ? error.message : String(error)
    failures.push({ name, message })
    console.error(`FAIL ${name}: ${message}`)
}

async function check(name, callback) {
    try {
        await callback()
        checksPassed++
        console.log(`PASS ${name}`)
    } catch (error) {
        recordFailure(name, error)
    }
}

function button(kind, left, rightOrWidth, topOrOverrides, bottom, overrides = {}) {
    if (typeof topOrOverrides === 'object') {
        overrides = topOrOverrides
        return { kind, x: left, y: 1, width: rightOrWidth, height: 31, hovered: false, pressed: false, ...overrides }
    }
    return { kind, x: left, y: topOrOverrides, width: rightOrWidth - left, height: bottom - topOrOverrides, hovered: false, pressed: false, ...overrides }
}

function fullState(scaleFactor = 1, overrides = {}) {
    const edge = value => Math.round(value * scaleFactor)
    const top = edge(1)
    const bottom = edge(32)
    return {
        scaleFactor,
        maximized: false,
        hoverColor: '#ffffff',
        buttons: [button('minimize', edge(662), edge(707), top, bottom), button('maximize', edge(707), edge(753), top, bottom), button('close', edge(753), edge(799), top, bottom)],
        ...overrides,
    }
}

async function preparePage(page, {
    background = '#1464d2',
    scaleFactor = 1,
    state = fullState(scaleFactor),
    foreground = '#b050d0',
    windowsBuild = 19045,
    instrumentDom = false,
} = {}) {
    await page.setContent(`<!doctype html><html><head><style>
        html,body,#page { margin:0; width:900px; height:80px; overflow:hidden; }
        #page { position:fixed; inset:0; background:${background}; }
        #caption { position:fixed; left:${state.buttons?.[0]?.x / scaleFactor ?? 662}px; top:${state.buttons?.[0]?.y / scaleFactor ?? 1}px; width:${state.buttons ? (state.buttons[2].x + state.buttons[2].width - state.buttons[0].x) / scaleFactor : 137}px; height:${state.buttons ? (state.buttons[0].height / scaleFactor) : 31}px;
            pointer-events:none; z-index:2; }
    </style></head><body><div id="page"></div><canvas id="caption" aria-hidden="true" tabindex="-1"></canvas></body></html>`)
    return page.evaluate(({ compiled, state, foreground, windowsBuild, scaleFactor, instrumentDom }) => {
        const canvas = document.querySelector('#caption')
        const browserModule = { exports: {} }
        new Function('module', 'exports', compiled)(browserModule, browserModule.exports)
        const render = browserModule.exports.renderWindowsCaption
        let listenerAdds = 0
        const eventTargets = [canvas, document, window]
        const originalAddEventListener = EventTarget.prototype.addEventListener
        if (instrumentDom) {
            EventTarget.prototype.addEventListener = function (...args) {
                if (eventTargets.includes(this)) listenerAdds++
                return originalAddEventListener.apply(this, args)
            }
        }
        let before
        try {
            before = {
                childCount: document.body.childElementCount,
                ariaHidden: canvas.getAttribute('aria-hidden'),
                tabIndex: canvas.tabIndex,
                pointerEvents: getComputedStyle(canvas).pointerEvents,
                focusableCount: document.querySelectorAll('button,a[href],input,select,textarea,[tabindex]:not([tabindex="-1"])').length,
                parentHtml: document.querySelector('#page').innerHTML,
            }
            render(canvas, state, foreground, windowsBuild)
        } finally {
            if (instrumentDom) EventTarget.prototype.addEventListener = originalAddEventListener
        }
        window.__captionTest = { canvas, render, state, foreground, windowsBuild, listenerAdds, before }
        return { width: canvas.width, height: canvas.height, before, listenerAdds }
    }, { compiled, state, foreground, windowsBuild, scaleFactor, instrumentDom })
}

async function redraw(page, { state, foreground = '#b050d0', windowsBuild = 19045 } = {}) {
    await page.evaluate(({ state, foreground, windowsBuild }) => {
        const test = window.__captionTest
        if (!test) throw new Error('Canvas fixture has not been prepared')
        test.render(test.canvas, state, foreground, windowsBuild)
        test.state = state
        test.foreground = foreground
        test.windowsBuild = windowsBuild
    }, { state, foreground, windowsBuild })
}

async function canvasPixels(page) {
    return page.evaluate(() => {
        const canvas = window.__captionTest.canvas
        const data = canvas.getContext('2d', { willReadFrequently: true }).getImageData(0, 0, canvas.width, canvas.height)
        return { width: data.width, height: data.height, data: Array.from(data.data) }
    })
}

function pixelAt(pixels, x, y) {
    const offset = (y * pixels.width + x) * 4
    return pixels.data.slice(offset, offset + 4)
}

function cssPixel(pixels, screenX, screenY, scaleFactor = 1) {
    const x = Math.max(0, Math.min(pixels.width - 1, Math.floor((screenX - 662) * scaleFactor)))
    const y = Math.max(0, Math.min(pixels.height - 1, Math.floor((screenY - 1) * scaleFactor)))
    return pixelAt(pixels, x, y)
}

function exactColorPixels(pixels, rect, scaleFactor, color, origin = { x: 662, y: 1 }) {
    const [red, green, blue] = color.match(/[0-9a-f]{2}/gi).map(channel => Number.parseInt(channel, 16))
    const left = rect.x - origin.x
    const top = rect.y - origin.y
    const right = Math.min(pixels.width, left + rect.width)
    const bottom = Math.min(pixels.height, top + rect.height)
    const points = []
    for (let y = top; y < bottom; y++) {
        for (let x = left; x < right; x++) {
            const [pixelRed, pixelGreen, pixelBlue, alpha] = pixelAt(pixels, x, y)
            if (pixelRed === red && pixelGreen === green && pixelBlue === blue && alpha === 255) points.push([x, y])
        }
    }
    return points
}

function pixelBounds(points) {
    assert.ok(points.length > 0, 'expected at least one exact foreground pixel')
    const xs = points.map(point => point[0])
    const ys = points.map(point => point[1])
    return { left: Math.min(...xs), top: Math.min(...ys), right: Math.max(...xs), bottom: Math.max(...ys) }
}

function countRegion(pixels, left, top, width, height, predicate) {
    let count = 0
    for (let y = top; y < top + height; y++) {
        for (let x = left; x < left + width; x++) {
            if (predicate(pixelAt(pixels, x, y))) count++
        }
    }
    return count
}

async function screenshotPixels(page, filename) {
    const screenshotPath = path.join(evidenceDirectory, filename)
    const screenshot = await page.screenshot({ path: screenshotPath })
    return page.evaluate(async pngBytes => {
        const bytes = Uint8Array.from(pngBytes)
        const blob = new Blob([bytes], { type: 'image/png' })
        const url = URL.createObjectURL(blob)
        try {
            const image = new Image()
            image.src = url
            await image.decode()
            const surface = document.createElement('canvas')
            surface.width = image.naturalWidth
            surface.height = image.naturalHeight
            const context = surface.getContext('2d', { willReadFrequently: true })
            context.drawImage(image, 0, 0)
            const result = context.getImageData(0, 0, surface.width, surface.height)
            return { width: result.width, height: result.height, data: Array.from(result.data) }
        } finally { URL.revokeObjectURL(url) }
    }, [...screenshot])
}

function screenshotPixel(pixels, x, y) {
    return pixelAt(pixels, x, y)
}

async function drawAndRead(page, options = {}) {
    await preparePage(page, options)
    return canvasPixels(page)
}

async function main() {
    const launchOptions = { headless: true }
    if (browserOverride) launchOptions.executablePath = browserOverride
    browser = await chromium.launch(launchOptions)
    const page = await browser.newPage({ viewport: { width: 900, height: 80 }, deviceScaleFactor: 1 })
    const fg = '#b050d0'
    const buttonRects = {
        minimize: { x: 662, y: 1, width: 45, height: 31 },
        maximize: { x: 707, y: 1, width: 46, height: 31 },
        close: { x: 753, y: 1, width: 46, height: 31 },
    }

    await check('Win10 normal maximize is a 10x10 hollow outline with exactly 36 foreground pixels', async () => {
        const pixels = await drawAndRead(page, { state: fullState(), windowsBuild: 19045 })
        const points = exactColorPixels(pixels, buttonRects.maximize, 1, fg)
        assert.equal(points.length, 36, `expected 36 exact foreground pixels; got ${points.length}`)
        for (let offset = 0; offset < pixels.data.length; offset += 4) {
            if (pixels.data[offset + 3] === 0) continue
            assert.deepEqual(pixels.data.slice(offset, offset + 4), [176, 80, 208, 255], 'normal canvas may contain only opaque foreground glyph pixels')
        }
        const bounds = pixelBounds(points)
        assert.equal(bounds.right - bounds.left + 1, 10)
        assert.equal(bounds.bottom - bounds.top + 1, 10)
        const center = pixelAt(pixels, bounds.left + 4, bounds.top + 4)
        assert.equal(center[3], 0, 'maximize icon center must remain transparent')
        assert.equal(cssPixel(pixels, 707, 2)[3], 0, 'button padding must remain transparent')
    })

    await check('Win10 minimize is exactly a centered 10x1 foreground line', async () => {
        const pixels = await drawAndRead(page, { state: fullState(), windowsBuild: 19045 })
        const points = exactColorPixels(pixels, buttonRects.minimize, 1, fg)
        assert.equal(points.length, 10, `expected 10 exact minimize pixels; got ${points.length}`)
        const bounds = pixelBounds(points)
        assert.equal(bounds.right - bounds.left + 1, 10)
        assert.equal(bounds.bottom - bounds.top + 1, 1)
    })

    await check('normal close glyph uses the requested foreground without painting a background', async () => {
        const pixels = await drawAndRead(page, { state: fullState(), windowsBuild: 19045 })
        const points = exactColorPixels(pixels, buttonRects.close, 1, fg)
        assert.ok(points.length >= 10, `expected visible close foreground; got ${points.length} pixels`)
        assert.ok(points.length < 46 * 31, 'normal close button must not paint its background')
    })

    await check('hover background uses alpha 26 and keeps the max/close left pixel gap only', async () => {
        const state = fullState()
        state.buttons[0].hovered = true
        state.buttons[1].hovered = true
        const pixels = await drawAndRead(page, { state, windowsBuild: 19045 })
        assert.deepEqual(cssPixel(pixels, 707, 2), [0, 0, 0, 0], 'maximize left gap must be transparent')
        assert.deepEqual(cssPixel(pixels, 708, 2), [255, 255, 255, 26], 'maximize hover alpha must be 26')
        assert.deepEqual(cssPixel(pixels, 753, 2), [0, 0, 0, 0], 'close left gap must be transparent')
        assert.deepEqual(cssPixel(pixels, 754, 2), [0, 0, 0, 0], 'unhovered close stays transparent')
        assert.deepEqual(cssPixel(pixels, 662, 2), [255, 255, 255, 26], 'minimize has no left gap')
    })

    await check('hoverProgress interpolates non-close alpha and close red/white colors', async () => {
        const state = fullState()
        state.buttons[1] = button('maximize', 707, 46, { hovered: false, hoverProgress: 0.5 })
        state.buttons[2] = button('close', 753, 46, { hovered: false, hoverProgress: 0.5 })
        const pixels = await drawAndRead(page, { state, windowsBuild: 19045 })
        assert.deepEqual(cssPixel(pixels, 708, 2), [255, 255, 255, 13], 'half max hover alpha truncates to 13')
        const closeReadback = await page.evaluate(() => {
            const baseline = document.createElement('canvas')
            baseline.width = baseline.height = 1
            const context = baseline.getContext('2d', { willReadFrequently: true })
            context.fillStyle = `rgba(232, 17, 35, ${127 / 255})`
            context.fillRect(0, 0, 1, 1)
            return Array.from(context.getImageData(0, 0, 1, 1).data)
        })
        const halfClose = cssPixel(pixels, 754, 2)
        assert.equal(halfClose[3], 127, 'half close hover alpha truncates to 127')
        assert.deepEqual(halfClose.slice(0, 3), closeReadback.slice(0, 3), 'close RGB matches independent Canvas2D premultiplication readback')
        state.buttons[1].hoverProgress = 0.1
        const tenthPixels = await drawAndRead(page, { state, windowsBuild: 19045 })
        assert.deepEqual(cssPixel(tenthPixels, 708, 2), [255, 255, 255, 2], 'one-tenth non-close alpha truncates to 2')
        state.buttons[1].hoverProgress = 0.5
        const normalPixels = await drawAndRead(page, { state: fullState(), windowsBuild: 19045 })
        const closePoints = exactColorPixels(normalPixels, buttonRects.close, 1, fg)
        assert.ok(closePoints.length > 0)
        const [sampleX, sampleY] = closePoints[0]
        const initialState = fullState()
        initialState.buttons[2].hovered = true
        initialState.buttons[2].hoverProgress = 0
        await drawAndRead(page, { state: initialState, windowsBuild: 19045 })
        const initialPixels = await canvasPixels(page)
        assert.deepEqual(pixelAt(initialPixels, sampleX, sampleY).slice(0, 3), [176, 80, 208], 'explicit zero progress preserves original glyph color')
        await drawAndRead(page, { state, windowsBuild: 19045 })
        const transitionPixels = await canvasPixels(page)
        const expected = fg.match(/[0-9a-f]{2}/gi).map(value => Math.round((Number.parseInt(value, 16) + 255) / 2))
        assert.deepEqual(pixelAt(transitionPixels, sampleX, sampleY).slice(0, 3), expected, 'close glyph should transition toward white')
    })

    await check('pressed state immediately wins with alpha 51 and close red alpha 152', async () => {
        const state = fullState()
        state.buttons[0] = button('minimize', 662, 707, 1, 32, { pressed: true, hovered: false, hoverProgress: 0.9 })
        state.buttons[1] = button('maximize', 707, 46, { pressed: true, hovered: false, hoverProgress: 0.9 })
        state.buttons[2] = button('close', 753, 46, { pressed: true, hovered: false, hoverProgress: 0.9 })
        const pixels = await drawAndRead(page, { state, windowsBuild: 19045 })
        assert.deepEqual(cssPixel(pixels, 707, 2), [0, 0, 0, 0], 'pressed maximize keeps the one-pixel left gap')
        assert.deepEqual(cssPixel(pixels, 708, 2), [255, 255, 255, 51])
        assert.deepEqual(cssPixel(pixels, 753, 2), [0, 0, 0, 0], 'pressed close keeps the one-pixel left gap')
        assert.deepEqual(cssPixel(pixels, 754, 2), [232, 17, 35, 152])
        assert.deepEqual(cssPixel(pixels, 662, 2), [255, 255, 255, 51], 'pressed minimize has no left gap')
        assert.ok(exactColorPixels(pixels, buttonRects.close, 1, '#ffffff').length > 0, 'pressed close glyph stays white')
    })

    await check('hovered close uses opaque red #e81123 background and white foreground', async () => {
        const state = fullState()
        state.buttons[2] = button('close', 753, 46, { hovered: true })
        const pixels = await drawAndRead(page, { state, windowsBuild: 19045 })
        assert.deepEqual(cssPixel(pixels, 753, 2), [0, 0, 0, 0], 'close left one-pixel gap stays transparent')
        assert.deepEqual(cssPixel(pixels, 754, 2), [232, 17, 35, 255])
        assert.ok(exactColorPixels(pixels, buttonRects.close, 1, '#ffffff').length > 0)
    })

    await check('hoverProgress absent follows hovered while false remains clear', async () => {
        const state = fullState()
        state.buttons[1] = button('maximize', 707, 46, { hovered: true })
        state.buttons[2] = button('close', 753, 46, { hovered: false })
        const pixels = await drawAndRead(page, { state, windowsBuild: 19045 })
        assert.deepEqual(cssPixel(pixels, 708, 2), [255, 255, 255, 26])
        assert.deepEqual(cssPixel(pixels, 754, 2), [0, 0, 0, 0])
    })

    await check('maximized restore glyph draws two clipped 8x8 outlines', async () => {
        const state = fullState(1, { maximized: true })
        const pixels = await drawAndRead(page, { state, windowsBuild: 19045 })
        const points = exactColorPixels(pixels, buttonRects.maximize, 1, fg)
        const bounds = pixelBounds(points)
        assert.equal(bounds.right - bounds.left + 1, 10, 'restore union width')
        assert.equal(bounds.bottom - bounds.top + 1, 10, 'restore union height')
        const expectedRows = ['..########', '..#......#', '########.#', '#......#.#', '#......#.#', '#......#.#', '#......#.#', '#......###', '#......#..', '########..']
        assert.equal(points.length, 45, `expected 45 exact restore pixels; got ${points.length}`)
        for (let y = 0; y < expectedRows.length; y++) {
            for (let x = 0; x < expectedRows[y].length; x++) {
                const expected = expectedRows[y][x] === '#'
                assert.equal(pixelAt(pixels, bounds.left + x, bounds.top + y)[3] === 255, expected, `restore mask mismatch at ${x},${y}`)
            }
        }
    })

    for (const scaleFactor of [1.25, 1.5, 2]) {
        await check(`DPI scale ${scaleFactor} keeps physical icon size, stroke and CSS/backing ratio`, async () => {
            const pixels = await drawAndRead(page, { scaleFactor, state: fullState(scaleFactor), windowsBuild: 19045 })
            const state = fullState(scaleFactor)
            const physicalWidth = state.buttons[2].x + state.buttons[2].width - state.buttons[0].x
            const physicalHeight = state.buttons[0].height
            assert.equal(pixels.width, physicalWidth, 'physical backing width from union edges')
            assert.equal(pixels.height, physicalHeight, 'physical backing height from top/bottom edges')
            const points = exactColorPixels(pixels, state.buttons[1], scaleFactor, fg, { x: state.buttons[0].x, y: state.buttons[0].y })
            const bounds = pixelBounds(points)
            const iconSize = Math.round(10 * scaleFactor)
            const stroke = Math.round(scaleFactor)
            assert.equal(bounds.right - bounds.left + 1, iconSize)
            assert.equal(bounds.bottom - bounds.top + 1, iconSize)
            assert.equal(points.length, iconSize ** 2 - (iconSize - 2 * stroke) ** 2)
            const ratio = await page.$eval('#caption', canvas => ({ x: canvas.width / canvas.clientWidth, y: canvas.height / canvas.clientHeight }))
            assert.ok(Math.abs(ratio.x - scaleFactor) <= 0.02, `backing x ratio ${ratio.x}`)
            assert.ok(Math.abs(ratio.y - scaleFactor) <= 0.02, `backing y ratio ${ratio.y}`)
        })
    }

    for (const maximized of [false, true]) {
        await check(`Windows 11 ${maximized ? 'restore' : 'maximize'} pixels visibly differ from Windows 10 corner pixels`, async () => {
            const win10 = await drawAndRead(page, { state: fullState(1, { maximized }), windowsBuild: 19045 })
            await page.screenshot({ path: path.join(evidenceDirectory, `caption-win10-${maximized ? 'restore' : 'maximize'}.png`) })
            const win11 = await drawAndRead(page, { state: fullState(1, { maximized }), windowsBuild: 22631 })
            await page.screenshot({ path: path.join(evidenceDirectory, `caption-win11-${maximized ? 'restore' : 'maximize'}.png`) })
            const points10 = exactColorPixels(win10, buttonRects.maximize, 1, fg)
            const bounds10 = pixelBounds(points10)
            const differs = win10.data.some((value, index) => value !== win11.data[index])
            assert.ok(differs, `Win11 ${maximized ? 'restore' : 'max'} must differ in actual Canvas pixels`)
            const corner = maximized ? [bounds10.left + 2, bounds10.top] : [bounds10.left, bounds10.top]
            assert.notDeepEqual(pixelAt(win11, ...corner), pixelAt(win10, ...corner),
                'rounded Win11 corner must differ from Win10 corner pixel')
        })
    }

    await check('Win11 maximize and restore differ at identical geometry, with restore retaining both offset frames', async () => {
        const win11Maximize = await drawAndRead(page, { state: fullState(1, { maximized: false }), windowsBuild: 22631 })
        const win11Restore = await drawAndRead(page, { state: fullState(1, { maximized: true }), windowsBuild: 22631 })
        const maximizePoints = exactColorPixels(win11Maximize, buttonRects.maximize, 1, fg)
        const restorePoints = exactColorPixels(win11Restore, buttonRects.maximize, 1, fg)
        const maxBounds = pixelBounds(maximizePoints)
        const restoreBounds = pixelBounds(restorePoints)
        assert.equal(restoreBounds.right - restoreBounds.left + 1, 10, 'restore frame union width')
        assert.equal(restoreBounds.bottom - restoreBounds.top + 1, 10, 'restore frame union height')
        assert.notDeepEqual(win11Maximize.data, win11Restore.data, 'same geometry must render distinct maximize and restore glyphs')
        const visibleAt = (x, y) => pixelAt(win11Restore, x, y)[3] > 0
        assert.ok(Array.from({ length: 6 }, (_, offset) => visibleAt(restoreBounds.left + 3 + offset, restoreBounds.top)).some(Boolean),
            'restore back frame has a visible upper edge')
        assert.ok(Array.from({ length: 6 }, (_, offset) => visibleAt(restoreBounds.left, restoreBounds.top + 3 + offset)).some(Boolean),
            'restore front frame has a visible left edge')
        assert.equal(countRegion(win11Restore, restoreBounds.left, restoreBounds.top, 2, 2, pixel => pixel[3] > 0), 0,
            'offset frame union leaves its upper-left gap transparent')
        assert.equal(countRegion(win11Restore, restoreBounds.left + 8, restoreBounds.top + 8, 2, 2, pixel => pixel[3] > 0), 0,
            'offset frame union leaves its lower-right gap transparent')
    })

    await check('null clears caption pixels and a subsequent fresh state repaints same canvas', async () => {
        await preparePage(page, { state: fullState(), windowsBuild: 19045 })
        const before = await canvasPixels(page)
        assert.ok(exactColorPixels(before, buttonRects.maximize, 1, fg).length > 0)
        const canvasIdentity = await page.$eval('#caption', canvas => `${canvas.width}x${canvas.height}`)
        await redraw(page, { state: null, foreground: fg, windowsBuild: 19045 })
        const cleared = await canvasPixels(page)
        assert.equal(countRegion(cleared, 0, 0, cleared.width, cleared.height, pixel => pixel[3] !== 0), 0, 'null must clear/hide every existing caption pixel')
        await redraw(page, { state: fullState(), foreground: fg, windowsBuild: 19045 })
        const repainted = await canvasPixels(page)
        assert.ok(exactColorPixels(repainted, buttonRects.maximize, 1, fg).length > 0, 'fresh state must repaint')
        assert.equal(await page.$eval('#caption', canvas => `${canvas.width}x${canvas.height}`), canvasIdentity, 'repaint must reuse the same canvas bitmap')
    })

    await check('foreground updates replace pixels on the same canvas', async () => {
        await preparePage(page, { state: fullState(), windowsBuild: 19045 })
        const canvasBefore = await page.$eval('#caption', canvas => `${canvas.width}x${canvas.height}`)
        await redraw(page, { state: fullState(), foreground: '#22aa44', windowsBuild: 19045 })
        const pixels = await canvasPixels(page)
        assert.equal(exactColorPixels(pixels, buttonRects.maximize, 1, '#22aa44').length, 36)
        assert.equal(exactColorPixels(pixels, buttonRects.maximize, 1, fg).length, 0, 'old foreground pixels must be replaced')
        assert.deepEqual(await page.evaluate(() => ({
            sameElement: window.__captionTest.canvas === document.querySelector('#caption'),
            connected: window.__captionTest.canvas.isConnected,
        })), { sameElement: true, connected: true }, 'same canvas element must remain connected')
        assert.equal(await page.$eval('#caption', canvas => `${canvas.width}x${canvas.height}`), canvasBefore, 'same canvas bitmap dimensions remain in place')
    })

    await check('canvas adds no listeners or focusable/accessibility nodes and remains pointer-transparent', async () => {
        const setup = await preparePage(page, { state: fullState(), instrumentDom: true, windowsBuild: 19045 })
        const renderedPixels = await canvasPixels(page)
        assert.equal(exactColorPixels(renderedPixels, buttonRects.maximize, 1, fg).length, 36,
            'DOM guard requires a visibly rendered maximize glyph')
        const after = await page.evaluate(() => {
            const canvas = document.querySelector('#caption')
            return {
                childCount: document.body.childElementCount,
                ariaHidden: canvas.getAttribute('aria-hidden'),
                tabIndex: canvas.tabIndex,
                pointerEvents: getComputedStyle(canvas).pointerEvents,
                focusableCount: document.querySelectorAll('button,a[href],input,select,textarea,[tabindex]:not([tabindex="-1"])').length,
                parentHtml: document.querySelector('#page').innerHTML,
                eventProperties: ['onclick','onpointerdown','onpointerup','onmousedown','onmouseup'].map(name => canvas[name] ?? null),
                elementAtCanvas: document.elementFromPoint(730, 16)?.id,
            }
        })
        assert.equal(setup.listenerAdds, 0, 'renderer must not register input listeners')
        assert.deepEqual(after, { ...setup.before, eventProperties: [null, null, null, null, null], elementAtCanvas: 'page' })
    })

    await check('live blue then orange HTML background remains visible through transparent glyph center', async () => {
        await preparePage(page, { background: '#0000ff', state: fullState(), windowsBuild: 19045 })
        const pixels = await canvasPixels(page)
        const icon = pixelBounds(exactColorPixels(pixels, buttonRects.maximize, 1, fg))
        const holeX = 662 + icon.left + 4
        const holeY = 1 + icon.top + 4
        const blueScreenshot = await screenshotPixels(page, 'caption-blue-page.png')
        assert.deepEqual(screenshotPixel(blueScreenshot, holeX, holeY).slice(0, 3), [0, 0, 255], 'blue page must show through icon center')
        await page.locator('#page').evaluate(element => { element.style.background = '#ff8800' })
        const orangeScreenshot = await screenshotPixels(page, 'caption-orange-page.png')
        assert.deepEqual(screenshotPixel(orangeScreenshot, holeX, holeY).slice(0, 3), [255, 136, 0], 'orange repaint must show through same center')
        assert.deepEqual(screenshotPixel(orangeScreenshot, 662 + icon.left, 1 + icon.top).slice(0, 3), [176, 80, 208], 'foreground glyph remains visible over orange')
    })

    await page.close()
    await browser.close()
    browser = null
    console.log(`\n${checksPassed} passed, ${failures.length} failed; evidence: ${evidenceDirectory}`)
    if (failures.length > 0) process.exitCode = 1
}

main().catch(async error => {
    if (browser) await browser.close().catch(() => {})
    console.error(`TEST SETUP FAILURE: ${error instanceof Error ? error.stack : String(error)}`)
    process.exitCode = 2
})
