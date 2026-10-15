#!/usr/bin/env node

import { mkdtemp, mkdir, readFile, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'
import { chromium } from 'playwright-core'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourcePath = path.join(root, 'tabby-tauri/src/components/windowsCaption.ts')
const defaultOutputPrefix = 'tabby-windows-caption-paint-'
const usage = `Usage: node scripts/test-windows-caption-paint.mjs [options]

Render and inspect the Windows caption painter in Chromium.

Options:
  --browser-path <path>          Executable Chromium path (default: playwright-core chromium.executablePath())
  --reference-directory <dir>    Optional original PNGs and result.json for pinned comparisons
  --output-directory <dir>       New directory for result.json and screenshots (must not exist)
  --help                         Show this help and exit

Without --output-directory, a new directory is created under os.tmpdir().
Exit codes: 0 all executed checks pass; 1 a check/runtime fails; 2 invalid arguments.
Examples:
  node scripts/test-windows-caption-paint.mjs
  node scripts/test-windows-caption-paint.mjs --browser-path /path/to/chromium
  node scripts/test-windows-caption-paint.mjs --reference-directory /path/to/reference --output-directory /tmp/caption-results
`

const args = parseArgs(process.argv.slice(2))
if (args.help) {
    process.stdout.write(usage)
    process.exit(0)
}

const results = []
const consoleErrors = []
let browser = null
let context = null
const contexts = []
let page = null
let outputDirectory = args.outputDirectory
let outputDirectoryCreated = false
let originalRasterComparison = { executed: false, reason: args.referenceDirectory ? 'reference checks pending' : 'no --reference-directory supplied' }
let runtimeFailure = null

try {
    if (outputDirectory) {
        await mkdir(outputDirectory)
        outputDirectoryCreated = true
    } else {
        outputDirectory = await mkdtemp(path.join(os.tmpdir(), defaultOutputPrefix))
        outputDirectoryCreated = true
    }
    const { paintSource, transpileDiagnostics } = await loadPainter()
    const executablePath = args.browserPath ?? chromium.executablePath()
    browser = await chromium.launch({ executablePath, headless: true, args: ['--no-sandbox'] })
    context = await browser.newContext({ viewport: { width: 800, height: 580 }, deviceScaleFactor: 1 })
    contexts.push(context)
    page = await context.newPage()
    page.on('console', message => {
        if (message.type() === 'error') consoleErrors.push(message.text())
    })
    page.on('pageerror', error => consoleErrors.push(error.message))
    await page.goto('about:blank')
    await installPainter(page, paintSource)

    const base = makeSnapshot()
    const options = makeOptions()
    let normalPng = null
    const run = async (name, fn) => {
        try {
            const detail = await fn()
            results.push({ name, passed: true, detail: detail ?? null })
        } catch (error) {
            results.push({ name, passed: false, error: error instanceof Error ? error.message : String(error) })
        }
    }

    await run('normal-render', async () => {
        await setupFixture(page, 800, 580, 1)
        const blank = await screenshotBlank(page)
        await paint(page, 'caption-a', base, options)
        normalPng = await page.screenshot()
        const inspect = await page.evaluate(() => {
            const container = document.getElementById('caption-a')
            const style = getComputedStyle(container)
            const svgs = [...container.querySelectorAll('svg')]
            return {
                style: { position: style.position, pointerEvents: style.pointerEvents, left: style.left, top: style.top, width: style.width, height: style.height },
                allSvgHidden: svgs.every(svg => svg.getAttribute('aria-hidden') === 'true'),
                interactive: container.querySelectorAll('button,[role="button"],a,input,[tabindex]:not([tabindex="-1"])').length,
            }
        })
        assert(inspect.style.position === 'absolute', `container position is ${inspect.style.position}`)
        assert(inspect.style.pointerEvents === 'none', `pointer-events is ${inspect.style.pointerEvents}`)
        assert(inspect.style.left === '663px' && inspect.style.top === '1px' && inspect.style.width === '136px' && inspect.style.height === '31px', `container geometry is ${JSON.stringify(inspect.style)}`)
        assert(inspect.allSvgHidden, 'SVG visuals must be aria-hidden')
        assert(inspect.interactive === 0, 'visual overlay added an interactive fake button')
        for (const rect of [{ x: 676, y: 9, width: 18, height: 15 }, { x: 721, y: 9, width: 18, height: 15 }, { x: 767, y: 9, width: 18, height: 15 }]) {
            assert(await regionDiffers(page, blank, normalPng, rect), `caption glyph has no rendered pixels in ${JSON.stringify(rect)}`)
        }
        await savePng(outputDirectory, 'normal.png', normalPng)
        return inspect.style
    })

    await run('dpr2-physical-parity', async () => {
        const dprContext = await browser.newContext({ viewport: { width: 400, height: 290 }, deviceScaleFactor: 2 })
        contexts.push(dprContext)
        const dprPage = await dprContext.newPage()
        listenToPage(dprPage)
        await dprPage.goto('about:blank')
        await installPainter(dprPage, paintSource)
        await setupFixture(dprPage, 400, 290, 2)
        assert(await dprPage.evaluate(() => window.devicePixelRatio) === 2, 'DPR2 context did not report devicePixelRatio=2')
        await paint(dprPage, 'caption-a', base, { ...options, devicePixelRatio: 2 })
        const style = await dprPage.locator('#caption-a').evaluate(node => {
            const s = getComputedStyle(node)
            return { left: s.left, top: s.top, width: s.width, height: s.height }
        })
        assert(normalPng, 'normal-render screenshot was unavailable for physical-pixel comparison')
        assert(JSON.stringify(style) === JSON.stringify({ left: '331.5px', top: '0.5px', width: '68px', height: '15.5px' }), `unexpected DPR2 CSS geometry: ${JSON.stringify(style)}`)
        const image = await dprPage.screenshot()
        await savePng(outputDirectory, 'dpr2.png', image)
        const dimensions = await imageDimensions(dprPage, image)
        assert(dimensions.width === 800 && dimensions.height === 580, `DPR2 image is ${dimensions.width}x${dimensions.height}`)
        if (normalPng) assert(await comparePng(dprPage, normalPng, image), 'DPR1 and DPR2 physical pixels differ')
        await setupFixture(page, 800, 580, 1)
        await paint(page, 'caption-a', withPointer(base, 'maximize', null), { ...options, hoverProgress: { maximize: 1 } })
        const hover1 = await page.screenshot()
        await paint(dprPage, 'caption-a', withPointer(base, 'maximize', null), { ...options, devicePixelRatio: 2, hoverProgress: { maximize: 1 } })
        const hover2 = await dprPage.screenshot()
        await savePng(outputDirectory, 'dpr2-hover.png', hover2)
        assert(await comparePng(dprPage, hover1, hover2), 'DPR1 and DPR2 hover compositing differs')
        return { style, dimensions, hoverPixelParity: true }
    })

    await run('minimize-hover-pixel', async () => {
        await setupFixture(page, 800, 580, 1)
        const snapshot = withPointer(base, 'minimize', null)
        await paint(page, 'caption-a', snapshot, { ...options, hoverProgress: { minimize: 1 } })
        const pixel = await pixelAt(page, 665, 4)
        assertColor(pixel, [43, 133, 214], 1, 'minimize hover background')
        const image = await page.screenshot()
        await savePng(outputDirectory, 'hover-minimize.png', image)
        return { pixel }
    })

    await run('pressed-and-close-hover-states', async () => {
        await setupFixture(page, 800, 580, 1)
        await paint(page, 'caption-a', withPointer(base, 'minimize', 'minimize'), { ...options, hoverProgress: { minimize: 1 } })
        const pressedPixel = await pixelAt(page, 665, 4)
        assertColor(pressedPixel, [66, 146, 218], 1, 'minimize pressed background')
        await paint(page, 'caption-a', withPointer(base, 'close', null), { ...options, hoverProgress: { close: 1 } })
        const closePixel = await pixelAt(page, 775, 4)
        assertColor(closePixel, [232, 17, 35], 1, 'close hover background')
        const closeImage = await page.screenshot()
        assert(await regionHasNearWhitePixel(page, closeImage, { x: 768, y: 10, width: 16, height: 16 }), 'close hover glyph has no white rendered pixels')
        await paint(page, 'caption-a', withPointer(base, 'close', 'close'), { ...options, hoverProgress: { close: 1 } })
        const closePressed = await pixelAt(page, 775, 4)
        assertColor(closePressed, [146, 58, 105], 2, 'close pressed blend')
        const image = await page.screenshot()
        await savePng(outputDirectory, 'hover-close-pressed.png', image)
        return { pressedPixel, closePixel, closePressed }
    })

    await run('drag-out-and-disabled-close', async () => {
        await setupFixture(page, 800, 580, 1)
        const blank = await screenshotBlank(page)
        await paint(page, 'caption-a', base, options)
        const ordinary = await page.screenshot()
        await paint(page, 'caption-a', withPointer(base, 'maximize', 'minimize'), { ...options, hoverProgress: { minimize: 1, maximize: 1, close: 1 } })
        const dragged = await page.screenshot()
        assert(await comparePng(page, ordinary, dragged), 'dragged pointer shows pressed or unrelated hover state')
        const disabled = structuredClone(base)
        disabled.layout.buttons.find(button => button.button === 'close').enabled = false
        await paint(page, 'caption-a', withPointer(disabled, 'close', null), { ...options, hoverProgress: { close: 1 } })
        const disabledImage = await page.screenshot()
        assert(await comparePng(page, ordinary, disabledImage), 'disabled close changed the ordinary button rendering')
        assert(await regionDiffers(page, blank, ordinary, { x: 768, y: 10, width: 16, height: 16 }), 'ordinary close glyph is missing')
        return { dragOutPixelParity: true, disabledPixelParity: true }
    })

    await run('tween-opacity-rounding', async () => {
        await setupFixture(page, 800, 580, 1)
        const blank = await screenshotBlank(page)
        await paint(page, 'caption-a', withPointer(base, 'minimize', null), { ...options, hoverProgress: { minimize: 0.1 } })
        const minOutput = await page.screenshot()
        const minPixel = await pixelAt(page, 665, 4, minOutput)
        const minOracle = await cssOverlayPixel(page, 665, 4, [255, 255, 255], 2 / 255)
        assertColor(minPixel, minOracle, 0, 'minimize tween alpha is floor(.1*nextafter(27,0)) = 2/255')
        await paint(page, 'caption-a', withPointer(base, 'close', null), { ...options, hoverProgress: { close: 0.1 } })
        const closeOutput = await page.screenshot()
        const closePixel = await pixelAt(page, 775, 4, closeOutput)
        const closeOracle = await cssOverlayPixel(page, 775, 4, [232, 17, 35], 25 / 255)
        assertColor(closePixel, closeOracle, 0, 'close tween alpha is floor(.1*nextafter(256,0)) = 25/255')
        return { blankDimensions: await imageDimensions(page, blank), minPixel, minOracle, closePixel, closeOracle }
    })

    await run('close-premultiplied-color-blend', async () => {
        await setupFixture(page, 800, 580, 1)
        const blank = await screenshotBlank(page)
        await paint(page, 'caption-a', withPointer(base, 'close', null), {
            ...options, symbolColor: [0, 0, 0, 0], hoverProgress: { close: 0.5 },
        })
        const candidate = await page.screenshot()
        assert(await premultipliedSymbolPixelHit(page, blank, candidate, { x: 768, y: 8, width: 16, height: 16 }), 'close tween did not render white alpha128 over red alpha127 with premultiplied blending')
        return { checked: 'rasterized close glyph colors against independently composited RGBA oracle' }
    })

    await run('hover-base-symbol-color-and-inactive', async () => {
        await setupFixture(page, 800, 580, 1)
        await paint(page, 'caption-a', withPointer(base, 'minimize', null), { ...options, symbolColor: [255, 255, 255, 255], hoverBase: 'black', hoverProgress: { minimize: 1 } })
        const blackBase = await pixelAt(page, 665, 4)
        assertColor(blackBase, [17, 107, 188], 1, 'black hover base alpha 26/255')
        await paint(page, 'caption-a', withPointer({ ...base, active: false }, 'minimize', null), { ...options, symbolColor: [0, 0, 0, 128], hoverBase: 'white', hoverProgress: { minimize: 1 } })
        const whiteBase = await pixelAt(page, 665, 4)
        assertColor(whiteBase, [43, 133, 214], 1, 'white hover base alpha 26/255 independent of symbol')
        const image = await page.screenshot()
        assert(await regionHasColor(page, image, { x: 676, y: 9, width: 18, height: 15 }, [22, 67, 107], 2), 'configured symbol RGBA black/alpha128 was not preserved in rendered pixels')
        return { blackBase, whiteBase, symbolPixel: [22, 67, 107] }
    })

    await run('maximized-restore-geometry', async () => {
        await setupFixture(page, 1024, 728, 1)
        const blank = await screenshotBlank(page)
        const max = makeMaximizedSnapshot()
        await paint(page, 'caption-a', max, options)
        const style = await page.locator('#caption-a').evaluate(node => {
            const s = getComputedStyle(node)
            return { left: s.left, top: s.top, width: s.width, height: s.height }
        })
        assert(JSON.stringify(style) === JSON.stringify({ left: '887px', top: '0px', width: '137px', height: '32px' }), `maximized clip geometry is ${JSON.stringify(style)}`)
        const image = await page.screenshot()
        for (const rect of [{ x: 897, y: 9, width: 24, height: 15 }, { x: 943, y: 9, width: 24, height: 15 }, { x: 989, y: 9, width: 24, height: 15 }]) {
            assert(await regionDiffers(page, blank, image, rect), `maximized button glyph missing in ${JSON.stringify(rect)}`)
        }
        await savePng(outputDirectory, 'maximized-restore.png', image)
        return { style }
    })

    await run('windows11-rendering-distinct', async () => {
        await setupFixture(page, 800, 580, 1)
        const normalBlank = await screenshotBlank(page)
        await paint(page, 'caption-a', base, { ...options, windows11: false })
        const normal10 = await page.screenshot()
        await paint(page, 'caption-a', base, { ...options, windows11: true })
        const normal11 = await page.screenshot()
        assert(await regionDiffers(page, normalBlank, normal11, { x: 673, y: 9, width: 65, height: 15 }), 'Windows 11 normal rendering is empty')
        assert(!(await comparePng(page, normal10, normal11)), 'Windows 11 and Windows 10 normal pixels are identical')

        await setupFixture(page, 1024, 728, 1)
        const snapshot = makeMaximizedSnapshot()
        const blank = await screenshotBlank(page)
        await paint(page, 'caption-a', snapshot, { ...options, windows11: false })
        const win10 = await page.screenshot()
        await paint(page, 'caption-a', snapshot, { ...options, windows11: true })
        const win11 = await page.screenshot()
        assert(await regionDiffers(page, blank, win11, { x: 897, y: 9, width: 70, height: 15 }), 'Windows 11 restore rendering is empty')
        assert(!(await comparePng(page, win10, win11)), 'Windows 11 and Windows 10 restore pixels are identical')
        assert(await regionDiffers(page, blank, win11, { x: 943, y: 9, width: 24, height: 15 }), 'Windows 11 restore glyph is missing')
        return { normal10Bytes: normal10.length, normal11Bytes: normal11.length, restore10Bytes: win10.length, restore11Bytes: win11.length, nativeDesktopVerified: false, win11NativeVerified: false }
    })

    await run('empty-hidden-and-container-isolation', async () => {
        await setupFixture(page, 800, 580, 1)
        await page.evaluate(() => { const b = document.createElement('div'); b.id = 'caption-b'; document.body.append(b) })
        await paint(page, 'caption-a', base, options)
        await paint(page, 'caption-b', base, options)
        const beforeB = await page.locator('#caption-b').evaluate(node => ({ html: node.innerHTML, style: node.getAttribute('style') }))
        const idRefs = await page.evaluate(() => {
            const roots = ['caption-a', 'caption-b'].map(id => document.getElementById(id))
            const owners = new Map()
            const ids = []
            for (const [owner, root] of roots.entries()) for (const node of [root, ...root.querySelectorAll('[id]')]) {
                if (node.id) {
                    ids.push({ id: node.id, owner })
                    const entries = owners.get(node.id) ?? []
                    entries.push(owner)
                    owners.set(node.id, entries)
                }
            }
            const refs = []
            for (const [owner, root] of roots.entries()) for (const node of [root, ...root.querySelectorAll('*')]) {
                for (const attr of [...node.attributes]) {
                    for (const match of attr.value.matchAll(/url\(#([^)]*)\)/g)) refs.push({ owner, id: match[1] })
                    if ((attr.name === 'href' || attr.name.endsWith(':href')) && attr.value.startsWith('#')) refs.push({ owner, id: attr.value.slice(1) })
                }
            }
            return { ids, refs, owners: [...owners.entries()] }
        })
        assert(new Set(idRefs.ids.map(entry => entry.id)).size === idRefs.ids.length, 'IDs collide across caption containers')
        const ownerById = new Map(idRefs.owners)
        assert(idRefs.refs.every(ref => ownerById.get(ref.id)?.length === 1 && ownerById.get(ref.id)[0] === ref.owner), `ID reference is missing, duplicated, or crosses containers: ${JSON.stringify(idRefs.refs)}`)
        await paint(page, 'caption-a', makeSnapshot({ visible: false }), options)
        const hidden = await page.locator('#caption-a').evaluate(node => ({ html: node.innerHTML, display: getComputedStyle(node).display, visibility: getComputedStyle(node).visibility }))
        const afterB = await page.locator('#caption-b').evaluate(node => ({ html: node.innerHTML, style: node.getAttribute('style') }))
        assert(hidden.html === '' && (hidden.display === 'none' || hidden.visibility === 'hidden'), `hidden paint retained visual or remained visible: ${JSON.stringify(hidden)}`)
        assert(JSON.stringify(beforeB) === JSON.stringify(afterB), 'painting A modified B markup or style')
        await paint(page, 'caption-a', makeSnapshot({ layout: { clip: { x: 0, y: 0, width: 0, height: 0 }, buttons: [] } }), options)
        const empty = await page.locator('#caption-a').evaluate(node => ({ html: node.innerHTML, display: getComputedStyle(node).display, visibility: getComputedStyle(node).visibility }))
        assert(empty.html === '' && (empty.display === 'none' || empty.visibility === 'hidden'), `empty clip retained old visual: ${JSON.stringify(empty)}`)
        return { hidden, empty, idRefs }
    })

    await run('preexisting-derived-clip-ids-do-not-collide', async () => {
        await setupFixture(page, 1024, 728, 1)
        await installPainter(page, paintSource)
        await page.evaluate(() => {
            const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
            for (const id of ['windows-caption-clip-1-physical', 'windows-caption-clip-2-glyph-1']) {
                const clip = document.createElementNS(svg.namespaceURI, 'clipPath')
                clip.id = id
                svg.append(clip)
            }
            svg.id = 'preexisting-clips'
            document.body.append(svg)
        })
        const before = await page.locator('#preexisting-clips').evaluate(node => node.outerHTML)
        await paint(page, 'caption-a', makeMaximizedSnapshot(), options)
        const ids = await page.locator('[id]').evaluateAll(nodes => nodes.map(node => node.id))
        assert(new Set(ids).size === ids.length, `painter reused an existing derived ID: ${JSON.stringify(ids)}`)
        assert(await page.locator('#preexisting-clips').evaluate(node => node.outerHTML) === before, 'painter changed preexisting clip definitions')
        return { ids }
    })

    await run('sparse-rgba-rejected-without-dom-change', async () => {
        await setupFixture(page, 800, 580, 1)
        await paint(page, 'caption-a', base, options)
        const outcomes = await page.evaluate(({ snapshot, options }) => {
            const container = document.getElementById('caption-a')
            const before = container.outerHTML
            return [0, 1, 2, 3].map(index => {
                const color = [...options.symbolColor]
                delete color[index]
                let errorName = null
                try { window.__paintWindowsCaption(container, snapshot, { ...options, symbolColor: color }) } catch (error) { errorName = error.name }
                return { index, errorName, unchanged: container.outerHTML === before }
            })
        }, { snapshot: base, options })
        assert(outcomes.every(outcome => outcome.errorName === 'RangeError' && outcome.unchanged), `sparse color was accepted or mutated DOM: ${JSON.stringify(outcomes)}`)
        return { outcomes }
    })

    await run('omitted-buttons-retain-type-spacing', async () => {
        await setupFixture(page, 800, 580, 1)
        const blank = await screenshotBlank(page)
        const closeOnly = makeSnapshot({ layout: { ...base.layout, buttons: base.layout.buttons.filter(button => button.button === 'close') } })
        await paint(page, 'caption-a', withPointer(closeOnly, 'close', null), { ...options, hoverProgress: { close: 1 } })
        const image = await page.screenshot()
        assert(!(await regionDiffers(page, blank, image, { x: 663, y: 1, width: 90, height: 31 })), 'omitted buttons retained their glyphs or background')
        assertColor(await pixelAt(page, 753, 4, image), await pixelAt(page, 753, 4, blank), 0, 'close retains its one-DIP leading gap when first in the array')
        assertColor(await pixelAt(page, 775, 4, image), [232, 17, 35], 0, 'remaining close still paints its hover fill')
        return { visibleButtons: ['close'] }
    })

    await run('native-scale-is-independent-of-webview-dpr', async () => {
        const observed = []
        for (const scale of [0.4, Math.fround(1.15), 1.5, Math.fround(1.65), 2]) {
            await setupFixture(page, Math.round(800 * scale), Math.round(580 * scale), 1)
            const scaleRect = rect => Object.fromEntries(Object.entries(rect).map(([key, value]) => [key, value * scale]))
            const snapshot = makeSnapshot({ scale, layout: { clip: scaleRect(base.layout.clip), buttons: base.layout.buttons.map(button => ({ ...button, bounds: scaleRect(button.bounds) })) } })
            await paint(page, 'caption-a', snapshot, options)
            const bounds = snapshot.layout.buttons[0].bounds
            const size = Math.round(Math.fround(10 * scale))
            const x = bounds.x + Math.floor((Math.ceil(bounds.width) - size) / 2)
            const y = bounds.y + Math.floor((Math.ceil(bounds.height) - size) / 2)
            const ink = await colorBounds(page, await page.screenshot(), { x: Math.floor(x) - 2, y: Math.floor(y) - 2, width: size + 4, height: size + 4 }, [224, 224, 224])
            assert(ink && ink.width === size && ink.height === Math.max(1, Math.round(scale)), `scale ${scale} minimize ink is ${JSON.stringify(ink)}`)
            if (scale === 2) assert(ink.x === 1359 && ink.y === 32, `scale2 symbol centering is ${JSON.stringify(ink)}`)
            observed.push({ scale, ink })
        }
        return observed
    })

    await run('fadeout-and-pressed-owner-isolation', async () => {
        await setupFixture(page, 800, 580, 1)
        await paint(page, 'caption-a', withPointer(base, 'close', null), { ...options, hoverProgress: { close: 0.5 } })
        const hovered = await page.screenshot()
        await paint(page, 'caption-a', base, { ...options, hoverProgress: { close: 0.5 } })
        assert(await comparePng(page, hovered, await page.screenshot()), 'leaving Close discarded the supplied fadeout progress')
        await paint(page, 'caption-a', withPointer(base, 'minimize', 'minimize'), options)
        const pressed = await page.screenshot()
        await paint(page, 'caption-a', withPointer(base, 'minimize', 'minimize'), { ...options, hoverProgress: { minimize: 1, maximize: 1, close: 1 } })
        assert(await comparePng(page, pressed, await page.screenshot()), 'a non-owner retained hover progress during another button press')
        return { fadeoutRetained: true, onlyPressOwnerChanged: true }
    })

    await run('windows11-source-derived-rounded-contours', async () => {
        const observed = []
        for (const maximized of [false, true]) {
            await setupFixture(page, maximized ? 1024 : 800, maximized ? 728 : 580, 1)
            const blank = await screenshotBlank(page)
            await paint(page, 'caption-a', maximized ? makeMaximizedSnapshot() : base, { ...options, windows11: true })
            const candidate = await page.screenshot()
            const oracle = await win11ContourOracle(page, blank, maximized)
            const region = maximized ? { x: 932, y: 0, width: 46, height: 32 } : { x: 707, y: 1, width: 46, height: 31 }
            const actualPixels = await readRegion(page, candidate, region)
            const expectedPixels = await readRegion(page, oracle, region)
            const maxDifference = Math.max(...actualPixels.map((value, index) => Math.abs(value - expectedPixels[index])))
            assert(maxDifference <= 1, `Win11 ${maximized ? 'restore' : 'maximize'} differs from the C++ rounded contour: max channel delta ${maxDifference}`)
            observed.push({ maximized, maxDifference })
        }
        return { observed, oracle: 'Canvas2D translation of pinned win_icon_painter.cc; not native Windows 11 acceptance' }
    })

    await run('transparent-close-tween-underflow-stays-transparent', async () => {
        await setupFixture(page, 800, 580, 1)
        await page.locator('#caption-a').evaluate(node => { node.style.stroke = 'red' })
        const blank = await screenshotBlank(page)
        await paint(page, 'caption-a', withPointer(base, 'close', null), { ...options, symbolColor: [0, 0, 0, 0], hoverProgress: { close: Number.MIN_VALUE } })
        assert(await comparePng(page, blank, await page.screenshot()), 'a zero f32 alpha inherited the container stroke instead of staying transparent')
        return { progress: Number.MIN_VALUE, alphaUnderflow: 0 }
    })

    await run('invalid-input-range-errors-preserve-dom', async () => {
        await setupFixture(page, 800, 580, 1)
        await paint(page, 'caption-a', base, options)
        const original = await page.locator('#caption-a').evaluate(node => node.outerHTML)
        const badCases = [
            ['scale-zero', makeSnapshot({ scale: 0 }), options],
            ['scale-negative', makeSnapshot({ scale: -1 }), options],
            ['scale-nan', makeSnapshot({ scale: NaN }), options],
            ['scale-infinity', makeSnapshot({ scale: Infinity }), options],
            ['dpr-zero', base, { ...options, devicePixelRatio: 0 }],
            ['dpr-negative', base, { ...options, devicePixelRatio: -1 }],
            ['dpr-nan', base, { ...options, devicePixelRatio: NaN }],
            ['dpr-infinity', base, { ...options, devicePixelRatio: Infinity }],
            ['clip-nonfinite', makeSnapshot({ layout: { ...base.layout, clip: { ...base.layout.clip, x: Infinity } } }), options],
            ['button-rect-nonfinite', makeSnapshot({ layout: { ...base.layout, buttons: base.layout.buttons.map((b, i) => i ? b : { ...b, bounds: { ...b.bounds, y: NaN } }) } }), options],
            ['rect-negative', makeSnapshot({ layout: { ...base.layout, buttons: base.layout.buttons.map((b, i) => i ? b : { ...b, bounds: { ...b.bounds, width: -1 } }) } }), options],
            ['rect-negative-height', makeSnapshot({ layout: { ...base.layout, buttons: base.layout.buttons.map((b, i) => i ? b : { ...b, bounds: { ...b.bounds, height: -1 } }) } }), options],
            ['rgba-out-of-range', base, { ...options, symbolColor: [256, 0, 0, 255] }],
            ['rgba-noninteger', base, { ...options, symbolColor: [1.5, 0, 0, 255] }],
            ['progress-nan', base, { ...options, hoverProgress: { minimize: NaN } }],
            ['progress-infinity', base, { ...options, hoverProgress: { minimize: Infinity } }],
            ['progress-negative', base, { ...options, hoverProgress: { minimize: -0.01 } }],
            ['progress-out-of-range', base, { ...options, hoverProgress: { minimize: 1.01 } }],
        ]
        const failures = []
        for (const [name, snapshot, paintOptions] of badCases) {
            const outcome = await tryPaint(page, 'caption-a', snapshot, paintOptions)
            const after = await page.locator('#caption-a').evaluate(node => node.outerHTML)
            const caseFailures = []
            if (!outcome.threw || outcome.name !== 'RangeError') caseFailures.push(`expected RangeError, received ${outcome.name ?? 'no error'}`)
            if (JSON.stringify(after) !== JSON.stringify(original)) caseFailures.push('changed container DOM or style before rejection')
            results.push({ name: `invalid-${name}`, passed: caseFailures.length === 0, ...(caseFailures.length ? { error: caseFailures.join('; ') } : {}) })
            if (caseFailures.length) failures.push(`${name}: ${caseFailures.join('; ')}`)
        }
        assert(failures.length === 0, failures.join('; '))
        return { cases: badCases.map(([name]) => name), errors: badCases.length }
    })

    if (args.referenceDirectory) {
        await run('reference-metadata-and-pinned-raster-comparisons', async () => {
            const reference = await loadReference(args.referenceDirectory)
            assert(reference.manifest?.baseline === '14e2d60b9b6dee84a53c37f05eefeb803787de04', 'reference baseline metadata is missing or does not match pinned baseline')
            assert(reference.manifest?.electron === '38.8.6', 'reference Electron metadata is missing or incorrect')
            assert(reference.manifest?.chromium === '140.0.7339.249', 'reference Chromium metadata is missing or incorrect')
            assert(reference.caption?.restored?.dpi === 96 && reference.caption?.maximized?.dpi === 96, 'reference DPI metadata is missing or not 96')
            const checks = []
            await setupFixture(page, 800, 580, 1); await paint(page, 'caption-a', base, options)
            const normalReferenceCandidate = await page.screenshot()
            await savePng(outputDirectory, 'reference-normal-candidate.png', normalReferenceCandidate)
            checks.push(await compareReference(page, 'electron-thin.png', args.referenceDirectory, normalReferenceCandidate, { width: 800, height: 580 }))
            await setupFixture(page, 800, 580, 1); await paint(page, 'caption-a', withPointer(base, 'maximize', null), { ...options, hoverProgress: { maximize: 1 } })
            const maxHoverReferenceCandidate = await page.screenshot()
            await savePng(outputDirectory, 'reference-max-hover-candidate.png', maxHoverReferenceCandidate)
            checks.push(await compareReference(page, 'electron-thin-hover-max.png', args.referenceDirectory, maxHoverReferenceCandidate, { width: 800, height: 580 }))
            await setupFixture(page, 1024, 728, 1)
            await paint(page, 'caption-a', withPointer(makeMaximizedSnapshot(), 'maximize', null), { ...options, hoverProgress: { maximize: 1 } })
            const restoreHoverReferenceCandidate = await page.screenshot()
            await savePng(outputDirectory, 'reference-hover-restore-candidate.png', restoreHoverReferenceCandidate)
            checks.push(await compareReference(page, 'electron-thin-hover-restore.png', args.referenceDirectory, restoreHoverReferenceCandidate, {
                referenceWidth: 1040, referenceHeight: 744, candidateWidth: 1024, candidateHeight: 728,
                referenceRoi: { x: 895, y: 8, width: 137, height: 32 }, candidateRoi: { x: 887, y: 0, width: 137, height: 32 },
            }))
            assert(checks.every(check => check.equal), `reference raster mismatch: ${checks.filter(check => !check.equal).map(check => check.file).join(', ')}`)
            originalRasterComparison = { executed: true, passed: true, checks }
            return { checks }
        })
    }

    if (transpileDiagnostics.length) results.push({ name: 'typescript-transpile', passed: false, error: transpileDiagnostics.join('\n') })
    await run('browser-console-clean', async () => {
        assert(consoleErrors.length === 0, `browser console errors: ${consoleErrors.join(' | ')}`)
        return { errors: consoleErrors }
    })
} catch (error) {
    runtimeFailure = error instanceof Error ? error.message : String(error)
    results.push({ name: 'harness-runtime', passed: false, error: runtimeFailure })
} finally {
    for (const browserContext of contexts) await browserContext.close().catch(() => {})
    await browser?.close().catch(() => {})
    const result = {
        ok: results.length > 0 && results.every(item => item.passed),
        results: results.map(item => ({ ...item, nativeDesktopVerified: false, win11NativeVerified: false })),
        consoleErrors,
        originalRasterComparison,
        nativeDesktopVerified: false,
        win11NativeVerified: false,
        runtimeFailure,
        outputDirectory,
    }
    if (outputDirectoryCreated) await writeFile(path.join(outputDirectory, 'result.json'), `${JSON.stringify(result, null, 2)}\n`, 'utf8')
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`)
}

if (results.length === 0 || results.some(item => !item.passed)) process.exitCode = 1

function parseArgs (argv) {
    const parsed = { help: false, browserPath: null, referenceDirectory: null, outputDirectory: null }
    for (let index = 0; index < argv.length; index++) {
        const arg = argv[index]
        if (arg === '--help') parsed.help = true
        else if (arg === '--browser-path' || arg === '--reference-directory' || arg === '--output-directory') {
            const value = argv[++index]
            if (!value || value.startsWith('--')) throw usageError(`${arg} requires a value`)
            const key = arg === '--browser-path' ? 'browserPath' : arg === '--reference-directory' ? 'referenceDirectory' : 'outputDirectory'
            parsed[key] = path.resolve(value)
        } else throw usageError(`unknown argument: ${arg}`)
    }
    return parsed
}

function usageError (message) {
    process.stderr.write(`${message}\n\n${usage}`)
    process.exit(2)
}

async function loadPainter () {
    const require = createRequire(import.meta.url)
    const ts = require('typescript')
    const source = await readFile(sourcePath, 'utf8')
    const result = ts.transpileModule(source, {
        compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
        reportDiagnostics: true,
    })
    const diagnostics = (result.diagnostics ?? []).map(diagnostic => ts.flattenDiagnosticMessageText(diagnostic.messageText, '\n'))
    return { paintSource: result.outputText, transpileDiagnostics: diagnostics }
}

function makeOptions () {
    return { devicePixelRatio: 1, symbolColor: [224, 224, 224, 255], hoverBase: 'white', windows11: false }
}

function makeMaximizedSnapshot (overrides = {}) {
    return makeSnapshot({ ...overrides, maximized: true, layout: { clip: { x: 887, y: 0, width: 137, height: 32 }, buttons: [
        { button: 'minimize', bounds: { x: 887, y: 0, width: 45, height: 32 }, enabled: true },
        { button: 'maximize', bounds: { x: 932, y: 0, width: 46, height: 32 }, enabled: true },
        { button: 'close', bounds: { x: 978, y: 0, width: 46, height: 32 }, enabled: true },
    ] } })
}

function makeSnapshot (overrides = {}) {
    const layout = overrides.layout ?? { clip: { x: 663, y: 1, width: 136, height: 31 }, buttons: [
        { button: 'minimize', bounds: { x: 662, y: 1, width: 45, height: 31 }, enabled: true },
        { button: 'maximize', bounds: { x: 707, y: 1, width: 46, height: 31 }, enabled: true },
        { button: 'close', bounds: { x: 753, y: 1, width: 46, height: 31 }, enabled: true },
    ] }
    return { revision: 1, visible: true, scale: 1, layout, pointer: { hovered: null, pressed: null }, maximized: false, active: true, ...overrides, layout }
}

function withPointer (snapshot, hovered, pressed) {
    return { ...snapshot, pointer: { hovered, pressed } }
}

async function setupFixture (page, width, height, dpr) {
    await page.setViewportSize({ width, height })
    await page.evaluate(({ width, height, dpr }) => {
        document.body.innerHTML = '<div id="fixture-marker"></div><div id="caption-a"></div>'
        Object.assign(document.documentElement.style, { margin: '0', padding: '0', width: `${width}px`, height: `${height}px` })
        Object.assign(document.body.style, { margin: '0', padding: '0', width: `${width}px`, height: `${height}px`, overflow: 'hidden', background: '#1377d1' })
        Object.assign(document.getElementById('fixture-marker').style, {
            position: 'absolute', right: `${46 / dpr}px`, top: '0', width: `${46 / dpr}px`, height: `${32 / dpr}px`,
            background: `repeating-conic-gradient(#e1a42b 0 25%, #6c294c 0 50%) 0/${8 / dpr}px ${8 / dpr}px`,
        })
    }, { width, height, dpr })
}

async function screenshotBlank (page) {
    return page.screenshot()
}

async function installPainter (page, paintSource) {
    await page.addScriptTag({ content: `(function () { const exports = {};\n${paintSource}\n;window.__paintWindowsCaption = exports.paintWindowsCaption; })();` })
}

function listenToPage (page) {
    page.on('console', message => {
        if (message.type() === 'error') consoleErrors.push(message.text())
    })
    page.on('pageerror', error => consoleErrors.push(error.message))
}

async function paint (page, id, snapshot, options) {
    await page.evaluate(({ id, snapshot, options }) => {
        const container = document.getElementById(id)
        window.__paintWindowsCaption(container, snapshot, options)
    }, { id, snapshot, options })
}

async function tryPaint (page, id, snapshot, options) {
    return page.evaluate(({ id, snapshot, options }) => {
        try {
            window.__paintWindowsCaption(document.getElementById(id), snapshot, options)
            return { threw: false }
        } catch (error) {
            return { threw: true, name: error?.name ?? typeof error, message: error?.message ?? String(error) }
        }
    }, { id, snapshot, options })
}

function assert (condition, message) {
    if (!condition) throw new Error(message)
}

function assertColor (actual, expected, tolerance, label) {
    assert(actual.length >= 3 && expected.every((value, index) => Math.abs(actual[index] - value) <= tolerance), `${label}: got ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)} ±${tolerance}`)
}

async function pixelAt (page, x, y, png = null) {
    return readRegion(page, png ?? await page.screenshot(), { x, y, width: 1, height: 1 })
}

async function readRegion (page, png, rect) {
    return page.evaluate(async ({ imageData, rect }) => {
        const image = new Image(); image.src = `data:image/png;base64,${imageData}`; await image.decode()
        const { x, y, width, height } = rect
        const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height
        const ctx = canvas.getContext('2d'); ctx.drawImage(image, x, y, width, height, 0, 0, width, height)
        return [...ctx.getImageData(0, 0, width, height).data]
    }, { imageData: png.toString('base64'), rect })
}

async function cssOverlayPixel (page, x, y, color, alpha) {
    await page.evaluate(({ x, y, color, alpha }) => {
        const container = document.getElementById('caption-a')
        container.style.visibility = 'hidden'
        const oracle = document.createElement('div')
        oracle.id = 'caption-pixel-oracle'
        Object.assign(oracle.style, {
            position: 'absolute', left: `${x}px`, top: `${y}px`, width: '1px', height: '1px', zIndex: '2147483647',
            backgroundColor: `rgba(${color[0]}, ${color[1]}, ${color[2]}, ${alpha})`,
        })
        document.body.append(oracle)
    }, { x, y, color, alpha })
    const result = await pixelAt(page, x, y)
    await page.evaluate(() => {
        document.getElementById('caption-pixel-oracle')?.remove()
        document.getElementById('caption-a').style.visibility = ''
    })
    return result
}

async function premultipliedSymbolPixelHit (page, baseline, candidate, rect) {
    return page.evaluate(async ({ baseline, candidate, rect }) => {
        const load = async data => {
            const image = new Image(); image.src = `data:image/png;base64,${data}`; await image.decode()
            const canvas = document.createElement('canvas'); canvas.width = image.width; canvas.height = image.height
            const context = canvas.getContext('2d'); context.drawImage(image, 0, 0)
            return { width: image.width, height: image.height, pixels: context.getImageData(0, 0, image.width, image.height).data }
        }
        const base = await load(baseline); const current = await load(candidate)
        const tolerance = 2
        for (let y = rect.y; y < rect.y + rect.height; y++) for (let x = rect.x; x < rect.x + rect.width; x++) {
            const index = (y * base.width + x) * 4
            const expected = [0, 1, 2].map(channel => {
                const redBackground = Math.round(base.pixels[index + channel] * (128 / 255) + [232, 17, 35][channel] * (127 / 255))
                return Math.round(redBackground * (127 / 255) + 255 * (128 / 255))
            })
            if (expected.every((value, channel) => Math.abs(current.pixels[index + channel] - value) <= tolerance)) return true
        }
        return false
    }, { baseline: baseline.toString('base64'), candidate: candidate.toString('base64'), rect })
}

async function regionDiffers (page, baseline, candidate, rect) {
    const a = await readRegion(page, baseline, rect)
    const b = await readRegion(page, candidate, rect)
    return a.some((value, index) => value !== b[index])
}

async function regionHasNearWhitePixel (page, image, rect) {
    const pixels = await readRegion(page, image, rect)
    for (let index = 0; index < pixels.length; index += 4) {
        if (pixels[index] >= 245 && pixels[index + 1] >= 245 && pixels[index + 2] >= 245) return true
    }
    return false
}

async function regionHasColor (page, image, rect, expected, tolerance) {
    const pixels = await readRegion(page, image, rect)
    for (let index = 0; index < pixels.length; index += 4) {
        if (expected.every((value, channel) => Math.abs(pixels[index + channel] - value) <= tolerance)) return true
    }
    return false
}

async function imageDimensions (page, png) {
    return page.evaluate(async png => {
        const image = new Image(); image.src = `data:image/png;base64,${png}`; await image.decode()
        return { width: image.width, height: image.height }
    }, png.toString('base64'))
}

async function comparePng (page, a, b) {
    return page.evaluate(async ({ a, b }) => {
        const pixels = async data => {
            const image = new Image(); image.src = `data:image/png;base64,${data}`; await image.decode()
            const canvas = document.createElement('canvas'); canvas.width = image.width; canvas.height = image.height
            const ctx = canvas.getContext('2d'); ctx.drawImage(image, 0, 0)
            return { width: image.width, height: image.height, data: ctx.getImageData(0, 0, image.width, image.height).data }
        }
        const left = await pixels(a); const right = await pixels(b)
        if (left.width !== right.width || left.height !== right.height) return false
        return left.data.every((value, index) => value === right.data[index])
    }, { a: a.toString('base64'), b: b.toString('base64') })
}

async function savePng (directory, name, buffer) {
    await writeFile(path.join(directory, name), buffer)
}

async function loadReference (directory) {
    const file = path.join(directory, 'result.json')
    const data = JSON.parse(await readFile(file, 'utf8'))
    const check = data.checks?.find(item => item.name === 'pinned-electron-thin')
    const detail = check?.detail?.Ok
    assert(detail, 'result.json lacks checks[name=pinned-electron-thin].detail.Ok')
    return { manifest: detail.manifest, caption: detail.caption }
}

async function compareReference (page, file, directory, candidate, { width, height, referenceWidth = width, referenceHeight = height, candidateWidth = width, candidateHeight = height, referenceRoi, candidateRoi } = {}) {
    const original = await readFile(path.join(directory, file))
    const comparison = await page.evaluate(async ({ a, b, referenceWidth, referenceHeight, candidateWidth, candidateHeight, referenceRoi, candidateRoi }) => {
        const read = async data => {
            const image = new Image(); image.src = `data:image/png;base64,${data}`; await image.decode()
            const canvas = document.createElement('canvas'); canvas.width = image.width; canvas.height = image.height
            const context = canvas.getContext('2d'); context.drawImage(image, 0, 0)
            return { width: image.width, height: image.height, pixels: context.getImageData(0, 0, image.width, image.height).data }
        }
        const ref = await read(a); const current = await read(b)
        if (ref.width !== referenceWidth || ref.height !== referenceHeight || current.width !== candidateWidth || current.height !== candidateHeight) return { equal: false, reason: `dimensions reference=${ref.width}x${ref.height} candidate=${current.width}x${current.height}` }
        const ra = referenceRoi ?? { x: 0, y: 0, width: referenceWidth, height: referenceHeight }; const ca = candidateRoi ?? ra
        if (ra.x < 0 || ra.y < 0 || ca.x < 0 || ca.y < 0 || ra.x + ra.width > ref.width || ra.y + ra.height > ref.height || ca.x + ca.width > current.width || ca.y + ca.height > current.height || ra.width !== ca.width || ra.height !== ca.height) return { equal: false, reason: 'comparison region is outside an image or has mismatched dimensions' }
        for (let y = 0; y < ra.height; y++) for (let x = 0; x < ra.width; x++) for (let c = 0; c < 4; c++) {
            const ri = ((ra.y + y) * ref.width + ra.x + x) * 4 + c
            const ci = ((ca.y + y) * current.width + ca.x + x) * 4 + c
            if (ref.pixels[ri] !== current.pixels[ci]) return { equal: false, reason: `pixel mismatch at ${x},${y}, channel ${c}` }
        }
        return { equal: true }
    }, { a: original.toString('base64'), b: candidate.toString('base64'), referenceWidth, referenceHeight, candidateWidth, candidateHeight, referenceRoi, candidateRoi })
    return { file, ...comparison }
}

async function colorBounds (page, image, rect, expected) {
    const pixels = await readRegion(page, image, rect)
    let left = rect.width; let right = -1; let top = rect.height; let bottom = -1
    for (let y = 0; y < rect.height; y++) for (let x = 0; x < rect.width; x++) {
        const index = (y * rect.width + x) * 4
        if (expected.every((value, channel) => pixels[index + channel] === value)) {
            left = Math.min(left, x); right = Math.max(right, x); top = Math.min(top, y); bottom = Math.max(bottom, y)
        }
    }
    return right < left ? null : { x: rect.x + left, y: rect.y + top, width: right - left + 1, height: bottom - top + 1 }
}

// Independent drawing API, following Electron 38.8.6 Win11IconPainter at 96 DPI.
async function win11ContourOracle (page, blank, maximized) {
    const data = await page.evaluate(async ({ png, maximized }) => {
        const image = new Image(); image.src = `data:image/png;base64,${png}`; await image.decode()
        const canvas = document.createElement('canvas'); canvas.width = image.width; canvas.height = image.height
        const context = canvas.getContext('2d'); context.drawImage(image, 0, 0)
        context.strokeStyle = '#e0e0e0'; context.lineWidth = 1
        const x = maximized ? 950 : 725; const y = 11
        if (!maximized) {
            context.beginPath(); context.roundRect(x + 0.5, y + 0.5, 9, 9, 1); context.stroke()
        } else {
            context.beginPath(); context.roundRect(x + 0.5, y + 2.5, 7, 7, 1); context.stroke()
            context.save()
            context.beginPath(); context.rect(0, 0, canvas.width, canvas.height)
            context.roundRect(x - 1, y + 1, 10, 10, 2.5); context.clip('evenodd')
            context.beginPath(); context.roundRect(x + 2.5, y + 0.5, 7, 7, [1, 2, 1, 1]); context.stroke()
            context.restore()
        }
        return canvas.toDataURL('image/png').split(',')[1]
    }, { png: blank.toString('base64'), maximized })
    return Buffer.from(data, 'base64')
}
