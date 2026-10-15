import type { WindowsCaptionButton, WindowsCaptionPaintOptions, WindowsCaptionSnapshot } from '../api/windowsCaption'

// Geometry follows Electron 38.8.6 win_caption_button.cc / win_icon_painter.cc.
// Tween rounding follows Chromium 140.0.7339.249 ui/gfx/animation/tween.cc.
const svgNamespace = 'http://www.w3.org/2000/svg'
const nextDown27 = 26.999999999999996
const nextDown256 = 255.99999999999997
let nextClipId = 1

function invalid (name: string): never {
    throw new RangeError(`Invalid Windows caption ${name}`)
}

function isRecord (value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function isButton (value: unknown): value is WindowsCaptionButton {
    return value === 'minimize' || value === 'maximize' || value === 'close'
}

function positiveFinite (value: unknown, name: string): void {
    if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) { invalid(name) }
}

function validateRect (rect: unknown, name: string): void {
    if (!isRecord(rect)) { invalid(name) }
    for (const key of ['x', 'y', 'width', 'height']) {
        if (typeof rect[key] !== 'number' || !Number.isFinite(rect[key])) { invalid(name) }
    }
    if ((rect.width as number) < 0 || (rect.height as number) < 0) { invalid(name) }
}

function validateInputs (snapshot: unknown, options: unknown): void {
    if (!isRecord(snapshot) || !isRecord(options)) { invalid('snapshot or options') }
    positiveFinite(snapshot.scale, 'scale')
    positiveFinite(options.devicePixelRatio, 'devicePixelRatio')
    if (!isRecord(snapshot.layout)) { invalid('layout') }
    validateRect(snapshot.layout.clip, 'clip')
    if (!Array.isArray(snapshot.layout.buttons)) { invalid('buttons') }
    for (const entry of snapshot.layout.buttons) {
        if (!isRecord(entry) || !isButton(entry.button)) { invalid('button') }
        validateRect(entry.bounds, 'button bounds')
        if (typeof entry.enabled !== 'boolean') { invalid('button enabled') }
    }
    const pointer = snapshot.pointer
    if (!isRecord(pointer) || !(pointer.hovered === null || isButton(pointer.hovered)) || !(pointer.pressed === null || isButton(pointer.pressed))) { invalid('pointer') }
    if (typeof snapshot.visible !== 'boolean' || typeof snapshot.maximized !== 'boolean' || typeof snapshot.active !== 'boolean') { invalid('snapshot flags') }
    const color = options.symbolColor
    if (!Array.isArray(color) || color.length !== 4 || [0, 1, 2, 3].some(index => !Number.isInteger(color[index]) || color[index] < 0 || color[index] > 255)) { invalid('symbolColor') }
    if (options.hoverBase !== 'black' && options.hoverBase !== 'white') { invalid('hoverBase') }
    if (typeof options.windows11 !== 'boolean') { invalid('windows11') }
    if (options.hoverProgress !== undefined) {
        if (!isRecord(options.hoverProgress)) { invalid('hoverProgress') }
        for (const [button, progress] of Object.entries(options.hoverProgress)) {
            if (!isButton(button)) { invalid('hoverProgress button') }
            if (progress === undefined) { continue }
            if (typeof progress !== 'number' || !Number.isFinite(progress) || progress < 0 || progress > 1) { invalid('hoverProgress') }
        }
    }
}

function uniqueClipId (container: HTMLElement, buttonCount: number): string {
    let id = ''
    let ids: string[] = []
    do {
        const prefix = `windows-caption-clip-${nextClipId++}`
        id = prefix
        ids = [prefix, `${prefix}-physical`, ...Array.from({ length: buttonCount }, (_, index) => `${prefix}-glyph-${index}`)]
    } while (ids.some(candidate => container.ownerDocument.getElementById(candidate)))
    return id
}

function svgElement (document: Document, markup: string): SVGSVGElement {
    const template = document.createElement('template')
    template.innerHTML = markup
    return template.content.firstElementChild as SVGSVGElement
}

function n (value: number): string {
    return Object.is(value, -0) ? '0' : String(value)
}

function floatBetween (progress: number, start: number, target: number): number {
    return Math.fround(start + Math.fround(target - start) * progress)
}

function clampByte (value: number): number {
    return Math.max(0, Math.min(255, Math.floor(value + 0.5)))
}

function roundedRectPath (x: number, y: number, width: number, height: number, radii: [number, number, number, number]): string {
    const [tl, tr, br, bl] = radii.map(radius => Math.max(0, Math.min(radius, width / 2, height / 2)))
    return `M${n(x + tl)} ${n(y)}H${n(x + width - tr)}A${n(tr)} ${n(tr)} 0 0 1 ${n(x + width)} ${n(y + tr)}V${n(y + height - br)}A${n(br)} ${n(br)} 0 0 1 ${n(x + width - br)} ${n(y + height)}H${n(x + bl)}A${n(bl)} ${n(bl)} 0 0 1 ${n(x)} ${n(y + height - bl)}V${n(y + tl)}A${n(tl)} ${n(tl)} 0 0 1 ${n(x + tl)} ${n(y)}Z`
}

function win10RestoreMarkup (size: number, stroke: number, halfStroke: number, scale: number, clipId: string): string {
    const separation = Math.floor(2 * scale)
    const front = { x: 0, y: separation, width: Math.max(0, size - separation), height: Math.max(0, size - separation) }
    const rear = { x: front.x + separation, y: front.y - separation, width: front.width, height: front.height }
    const difference = `M0 0H${n(size)}V${n(size)}H0Z M${n(front.x)} ${n(front.y)}H${n(front.x + front.width)}V${n(front.y + front.height)}H${n(front.x)}Z`
    return `<defs><clipPath id="${clipId}"><path d="${difference}" clip-rule="evenodd" fill-rule="evenodd"/></clipPath></defs><rect x="${n(halfStroke)}" y="${n(front.y + halfStroke)}" width="${n(Math.max(0, front.width - stroke))}" height="${n(Math.max(0, front.height - stroke))}"/><rect x="${n(rear.x + halfStroke)}" y="${n(rear.y + halfStroke)}" width="${n(Math.max(0, rear.width - stroke))}" height="${n(Math.max(0, rear.height - stroke))}" clip-path="url(#${clipId})"/>`
}

function win11RestoreMarkup (size: number, stroke: number, halfStroke: number, scale: number, clipId: string): string {
    const separation = Math.fround(2 * scale)
    const radius = Math.floor(Math.fround(1.5 * scale))
    const front = { x: halfStroke, y: separation + halfStroke, width: Math.max(0, size - separation - stroke), height: Math.max(0, size - separation - stroke) }
    const growth = separation - halfStroke
    const rear = { x: front.x + separation, y: front.y - separation, width: front.width, height: front.height }
    const clip = { x: front.x - growth, y: front.y - growth, width: front.width + growth * 2, height: front.height + growth * 2 }
    const frontPath = roundedRectPath(front.x, front.y, front.width, front.height, [radius, radius, radius, radius])
    const clipPath = `M0 0H${n(size)}V${n(size)}H0Z ${roundedRectPath(clip.x, clip.y, clip.width, clip.height, [radius + growth, radius + growth, radius + growth, radius + growth])}`
    const rearPath = roundedRectPath(rear.x, rear.y, rear.width, rear.height, [radius, radius * 2, radius, radius])
    return `<defs><clipPath id="${clipId}"><path d="${clipPath}" clip-rule="evenodd" fill-rule="evenodd"/></clipPath></defs><path d="${frontPath}"/><path d="${rearPath}" clip-path="url(#${clipId})"/>`
}

function glyphMarkup (button: WindowsCaptionButton, maximized: boolean, windows11: boolean, size: number, stroke: number, halfStroke: number, closeStrokeWidth: number, scale: number, clipId: string): string {
    if (button === 'minimize') { return `<path d="M0 ${n(Math.floor(size / 2))}H${n(size)}"/>` }
    if (button === 'close') { return `<path stroke-width="${n(closeStrokeWidth)}" d="M0 0L${n(size)} ${n(size)}M${n(size)} 0L0 ${n(size)}"/>` }
    if (!maximized) {
        if (windows11) {
            const radius = Math.floor(Math.fround(1.5 * scale))
            return `<rect x="${n(halfStroke)}" y="${n(halfStroke)}" width="${n(Math.max(0, size - stroke))}" height="${n(Math.max(0, size - stroke))}" rx="${n(radius)}" ry="${n(radius)}"/>`
        }
        return `<rect x="${n(halfStroke)}" y="${n(halfStroke)}" width="${n(Math.max(0, size - stroke))}" height="${n(Math.max(0, size - stroke))}"/>`
    }
    if (windows11) { return win11RestoreMarkup(size, stroke, halfStroke, scale, clipId) }
    return win10RestoreMarkup(size, stroke, halfStroke, scale, clipId)
}

function closeStroke (stroke: number, color: [number, number, number, number]): number {
    if (stroke === 0) { return 1 }
    const pureWhite = color[0] === 255 && color[1] === 255 && color[2] === 255 && color[3] === 255
    return Math.fround(stroke + Math.fround(stroke * Math.fround(pureWhite ? 0.1 : 0.05)))
}

function blendTowardWhite (color: [number, number, number, number], progress: number): [number, number, number, number] {
    const startAlpha = Math.fround(color[3] / 255)
    const alpha = Math.min(1, floatBetween(progress, startAlpha, 1))
    if (alpha <= 0) { return [0, 0, 0, 0] }
    const components = color.slice(0, 3).map(byte => {
        const startPremultiplied = Math.fround(Math.fround(byte / 255) * startAlpha)
        const premultiplied = floatBetween(progress, startPremultiplied, 1)
        return clampByte(Math.fround(Math.fround(premultiplied / alpha) * 255))
    })
    return [components[0], components[1], components[2], clampByte(Math.fround(alpha * 255))]
}

function buttonState (
    button: WindowsCaptionSnapshot['layout']['buttons'][number],
    snapshot: WindowsCaptionSnapshot,
    options: WindowsCaptionPaintOptions,
    pressedOwner: WindowsCaptionButton | null,
    ownerEnabled: boolean,
): { backgroundAlpha: number; hoverProgress: number } {
    if (!button.enabled) { return { backgroundAlpha: 0, hoverProgress: 0 } }
    if (ownerEnabled && button.button === pressedOwner) {
        const alpha = button.button === 'close' ? 152 : 51
        return { backgroundAlpha: alpha, hoverProgress: 1 }
    }
    if (snapshot.pointer.pressed !== null) { return { backgroundAlpha: 0, hoverProgress: 0 } }

    const settled = snapshot.pointer.hovered === button.button ? 1 : 0
    const progress = options.hoverProgress?.[button.button] ?? settled
    const backgroundAlpha = progress === 0 ? 0 : Math.floor(progress * (button.button === 'close' ? nextDown256 : nextDown27))
    return {
        backgroundAlpha,
        hoverProgress: progress,
    }
}

function renderSvg (snapshot: WindowsCaptionSnapshot, options: WindowsCaptionPaintOptions, clipId: string): string {
    const { clip, buttons } = snapshot.layout
    const scale = snapshot.scale
    const floatScale = Math.fround(scale)
    const glyphSize = Math.floor(Math.fround(10 * floatScale) + 0.5)
    const stroke = Math.floor(floatScale + 0.5)
    const halfStroke = stroke / 2
    const pressedOwner = snapshot.pointer.pressed !== null && snapshot.pointer.pressed === snapshot.pointer.hovered
        ? snapshot.pointer.pressed
        : null
    const ownerEnabled = pressedOwner !== null && buttons.some(button => button.button === pressedOwner && button.enabled)
    const pieces: string[] = []
    const physicalClipId = `${clipId}-physical`
    const clipPath = `<defs><clipPath id="${physicalClipId}" clipPathUnits="userSpaceOnUse"><rect x="${n(clip.x)}" y="${n(clip.y)}" width="${n(clip.width)}" height="${n(clip.height)}"/></clipPath><clipPath id="${clipId}"><rect width="${n(glyphSize)}" height="${n(glyphSize)}"/></clipPath></defs>`

    for (const [buttonIndex, button] of buttons.entries()) {
        const state = buttonState(button, snapshot, options, pressedOwner, ownerEnabled)
        const iconX = button.bounds.x + Math.floor((Math.ceil(button.bounds.width) - glyphSize) / 2)
        const iconY = button.bounds.y + Math.floor((Math.ceil(button.bounds.height) - glyphSize) / 2)
        const spacing = button.button === 'minimize' ? 0 : floatScale
        const fillWidth = button.bounds.width - spacing
        if (state.backgroundAlpha > 0 && fillWidth > 0) {
            const color = button.button === 'close' ? [232, 17, 35] : options.hoverBase === 'white' ? [255, 255, 255] : [0, 0, 0]
            pieces.push(`<g transform="translate(${n(button.bounds.x + spacing)} ${n(button.bounds.y)})" shape-rendering="crispEdges"><path d="M0 0H${n(fillWidth)}V${n(button.bounds.height)}H0Z" fill="rgb(${color.join(',')})" fill-opacity="${n(state.backgroundAlpha / 255)}"/></g>`)
        }
        if (glyphSize <= 0) { continue }

        let color = options.symbolColor
        if (button.button === 'close' && state.hoverProgress > 0) { color = blendTowardWhite(color, state.hoverProgress) }
        const strokeWidth = button.button === 'close' ? closeStroke(stroke, color) : stroke || 1
        const colorCss = `rgba(${color[0]},${color[1]},${color[2]},${n(color[3] / 255)})`
        const antialias = options.windows11 && button.button === 'maximize' ? 'geometricPrecision' : 'crispEdges'
        const shape = glyphMarkup(button.button, snapshot.maximized, options.windows11, glyphSize, stroke, halfStroke, strokeWidth, floatScale, `${clipId}-glyph-${buttonIndex}`)
        pieces.push(`<g transform="translate(${n(iconX)} ${n(iconY)})" fill="none" stroke="${colorCss}" stroke-width="${n(strokeWidth)}" stroke-linecap="butt" stroke-linejoin="miter" shape-rendering="${antialias}">${button.button === 'close' ? `<g clip-path="url(#${clipId})">${shape}</g>` : shape}</g>`)
    }

    const dpr = options.devicePixelRatio
    // Keep an integer SVG viewport and scale it independently of CSS layout
    // rounding. Clip in physical units and retain the document origin.
    const extentX = Math.max(1, Math.ceil(clip.x + clip.width))
    const extentY = Math.max(1, Math.ceil(clip.y + clip.height))
    return `<svg xmlns="${svgNamespace}" aria-hidden="true" width="${n(extentX)}" height="${n(extentY)}" viewBox="0 0 ${n(extentX)} ${n(extentY)}" preserveAspectRatio="none" style="position:absolute;left:${n(-clip.x / dpr)}px;top:${n(-clip.y / dpr)}px;display:block;overflow:visible;fill:none;transform-origin:0 0;transform:scale(${n(1 / dpr)})">${clipPath}<g clip-path="url(#${physicalClipId})">${pieces.join('')}</g></svg>`
}

/** Private visuals in a direct child of the non-scrolling body root. The native child owns mouse input. */
export function paintWindowsCaption (
    container: HTMLElement,
    snapshot: WindowsCaptionSnapshot,
    options: WindowsCaptionPaintOptions,
): void {
    validateInputs(snapshot, options)

    const { clip } = snapshot.layout
    if (!snapshot.visible || clip.width === 0 || clip.height === 0) {
        container.replaceChildren()
        container.style.display = 'none'
        return
    }

    const dpr = options.devicePixelRatio
    const width = clip.width
    const height = clip.height
    const id = uniqueClipId(container, snapshot.layout.buttons.length)
    const markup = renderSvg(snapshot, options, id)

    container.style.position = 'absolute'
    container.style.left = `${clip.x / dpr}px`
    container.style.top = `${clip.y / dpr}px`
    container.style.width = `${width / dpr}px`
    container.style.height = `${height / dpr}px`
    container.style.overflow = 'visible'
    container.style.pointerEvents = 'none'
    container.style.display = 'block'
    container.replaceChildren(svgElement(container.ownerDocument, markup))
}
