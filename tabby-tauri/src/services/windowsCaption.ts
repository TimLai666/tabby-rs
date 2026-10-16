/*!
 * Pinned source license notices for the caption algorithms below.
 * https://raw.githubusercontent.com/electron/electron/v38.8.6/LICENSE
Copyright (c) Electron contributors
Copyright (c) 2013-2020 GitHub Inc.

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

 * https://raw.githubusercontent.com/chromium/chromium/140.0.7339.249/LICENSE
// Copyright 2015 The Chromium Authors
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
//    * Redistributions of source code must retain the above copyright
// notice, this list of conditions and the following disclaimer.
//    * Redistributions in binary form must reproduce the above
// copyright notice, this list of conditions and the following disclaimer
// in the documentation and/or other materials provided with the
// distribution.
//    * Neither the name of Google LLC nor the names of its
// contributors may be used to endorse or promote products derived from
// this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
// "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
// LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
// A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
// OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
// LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
// DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
// THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

 */

export interface WindowCaptionButton {
    kind: 'minimize' | 'maximize' | 'close'
    x: number
    y: number
    width: number
    height: number
    hovered: boolean
    pressed: boolean
    /** Renderer-only progress for the original 150ms hover animation. */
    hoverProgress?: number
}

export interface WindowCaptionState {
    scaleFactor: number
    maximized: boolean
    hoverColor: '#000000' | '#ffffff'
    buttons: WindowCaptionButton[]
}

// Caption algorithms are adapted from Electron 38.8.6:
// shell/browser/ui/views/win_icon_painter.cc
// Copyright (c) 2022 Microsoft, Inc. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in Electron's LICENSE file.
// shell/browser/ui/views/win_caption_button.cc
// Copyright (c) 2016 The Chromium Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in Electron's LICENSE file.
// Tween integer and color interpolation are adapted from Chromium
// 140.0.7339.249 ui/gfx/animation/tween.cc:
// Copyright 2012 The Chromium Authors.
// Use of this source code is governed by a BSD-style license that can be
// found in Chromium's LICENSE file.

type Rgb = [number, number, number]

const roundPixel = (value: number): number => Math.floor(value + 0.5)

function nextDown (value: number): number {
    const buffer = new ArrayBuffer(8)
    const view = new DataView(buffer)
    view.setFloat64(0, value)
    let high = view.getUint32(0)
    let low = view.getUint32(4)
    if (value > 0) {
        // Positive IEEE-754 bit patterns increase with their numeric value.
        if (low === 0) {
            high--
            low = 0xffffffff
        } else {
            low--
        }
    } else if (value < 0) {
        // Negative bit patterns run in the opposite numeric direction.
        low++
        if (low > 0xffffffff) {
            low = 0
            high++
        }
    } else {
        high = 0x80000000
        low = 1
    }
    view.setUint32(0, high)
    view.setUint32(4, low)
    return view.getFloat64(0)
}

function tweenInt (progress: number, limit: number): number {
    return Math.floor(progress * nextDown(limit + 1))
}

function parseColor (context: CanvasRenderingContext2D, color: string): Rgb {
    context.fillStyle = '#000000'
    context.fillStyle = color
    context.fillRect(0, 0, 1, 1)
    const pixel = context.getImageData(0, 0, 1, 1).data
    return [pixel[0], pixel[1], pixel[2]]
}

function colorString (color: Rgb, alpha: number): string {
    return `rgba(${color[0]}, ${color[1]}, ${color[2]}, ${alpha / 255})`
}

function interpolatedCloseColor (from: Rgb, progress: number): Rgb {
    return from.map(channel => roundPixel(channel + (255 - channel) * progress)) as Rgb
}

function setPixel (data: ImageData, x: number, y: number, color: Rgb): void {
    if (x < 0 || y < 0 || x >= data.width || y >= data.height) return
    const index = (y * data.width + x) * 4
    data.data[index] = color[0]
    data.data[index + 1] = color[1]
    data.data[index + 2] = color[2]
    data.data[index + 3] = 255
}

function copyPixels (target: ImageData, source: Uint8ClampedArray): void {
    target.data.set(source)
}

function roundRectPath (
    x: number,
    y: number,
    width: number,
    height: number,
    radii: number | number[],
): Path2D {
    const path = new Path2D()
    path.roundRect(x, y, width, height, radii)
    return path
}

function drawWin11Maximize (
    context: CanvasRenderingContext2D,
    pixels: ImageData,
    glyph: Rgb,
    x: number,
    y: number,
    size: number,
    stroke: number,
    radius: number,
): void {
    context.putImageData(pixels, 0, 0)
    context.save()
    context.strokeStyle = colorString(glyph, 255)
    context.lineWidth = stroke
    const path = roundRectPath(x + stroke / 2, y + stroke / 2, size - stroke, size - stroke, radius)
    context.stroke(path)
    context.restore()
    copyPixels(pixels, context.getImageData(0, 0, pixels.width, pixels.height).data)
}

function drawWin11Restore (
    context: CanvasRenderingContext2D,
    pixels: ImageData,
    glyph: Rgb,
    x: number,
    y: number,
    size: number,
    stroke: number,
    radius: number,
    separation: number,
): void {
    const halfStroke = stroke / 2
    const front = {
        x: x + halfStroke,
        y: y + halfStroke + separation,
        width: size - stroke - separation,
        height: size - stroke - separation,
    }
    const clipOutset = separation - halfStroke
    const clipPath = roundRectPath(
        front.x - clipOutset,
        front.y - clipOutset,
        front.width + 2 * clipOutset,
        front.height + 2 * clipOutset,
        Math.max(0, radius + clipOutset),
    )
    const back = {
        x: front.x + separation,
        y: front.y - separation,
        width: front.width,
        height: front.height,
    }

    context.putImageData(pixels, 0, 0)
    context.save()
    context.strokeStyle = colorString(glyph, 255)
    context.lineWidth = stroke
    context.stroke(roundRectPath(front.x, front.y, front.width, front.height, radius))

    // The source uses a difference clip against the expanded front rrect.
    // An even-odd path describes the same canvas area outside that rrect.
    const difference = new Path2D()
    difference.rect(0, 0, pixels.width, pixels.height)
    difference.addPath(clipPath)
    context.clip(difference, 'evenodd')
    context.stroke(roundRectPath(
        back.x,
        back.y,
        back.width,
        back.height,
        [radius, radius * 2, radius, radius],
    ))
    context.restore()
    copyPixels(pixels, context.getImageData(0, 0, pixels.width, pixels.height).data)
}

/** Paint the original Windows caption visuals underneath native input controls. */
export function renderWindowsCaption (
    canvas: HTMLCanvasElement,
    state: WindowCaptionState | null,
    foreground: string,
    windowsBuild: number | undefined,
): void {
    const context = canvas.getContext('2d', { willReadFrequently: true })
    if (!context) return

    if (!state || state.buttons.length === 0) {
        context.clearRect(0, 0, canvas.width, canvas.height)
        canvas.style.display = 'none'
        return
    }

    const scale = state.scaleFactor
    if (!Number.isFinite(scale) || scale <= 0) {
        context.clearRect(0, 0, canvas.width, canvas.height)
        canvas.style.display = 'none'
        return
    }

    const left = Math.min(...state.buttons.map(button => button.x))
    const firstVisualButton = state.buttons.reduce((first, button) => button.x < first.x ? button : first)
    const top = Math.min(...state.buttons.map(button => button.y))
    const right = Math.max(...state.buttons.map(button => button.x + button.width))
    const bottom = Math.max(...state.buttons.map(button => button.y + button.height))
    const width = Math.max(0, right - left)
    const height = Math.max(0, bottom - top)
    if (width === 0 || height === 0) {
        context.clearRect(0, 0, canvas.width, canvas.height)
        canvas.style.display = 'none'
        return
    }

    canvas.style.display = ''
    canvas.style.position = 'fixed'
    canvas.style.left = `${left / scale}px`
    canvas.style.top = `${top / scale}px`
    canvas.style.width = `${width / scale}px`
    canvas.style.height = `${height / scale}px`
    canvas.style.pointerEvents = 'none'
    if (canvas.width !== width) canvas.width = width
    if (canvas.height !== height) canvas.height = height
    context.clearRect(0, 0, width, height)

    // Read the configured CSS color through the browser's own color parser.
    const foregroundColor = parseColor(context, foreground)
    const isWin11 = windowsBuild !== undefined && windowsBuild >= 22000
    const stroke = Math.max(1, roundPixel(scale))
    const iconSize = Math.max(1, roundPixel(10 * scale))
    const isMaximized = state.maximized

    // Skia's Win10 flags disable antialiasing. Writing the integer stroke
    // geometry directly preserves its hard pixel edges in Canvas2D.
    const pixels = context.createImageData(width, height)
    for (const button of state.buttons) {
        const progress = button.pressed
            ? 1
            : button.hoverProgress === undefined
                ? (button.hovered ? 1 : 0)
                : Math.max(0, Math.min(1, button.hoverProgress))
        const close = button.kind === 'close'
        const hoverLimit = close ? 255 : 26
        const alpha = button.pressed
            ? (close ? 152 : 51)
            : tweenInt(progress, hoverLimit)
        const boundsX = button.x - left
        const boundsY = button.y - top
        // Chromium applies a 1 DIP spacing before max/close, then the canvas
        // device scale maps the inset edge to physical pixels.
        const spacing = button !== firstVisualButton
            ? roundPixel(button.x + scale) - button.x
            : 0

        if (alpha > 0) {
            const background: Rgb = close ? [0xE8, 0x11, 0x23] : state.hoverColor === '#ffffff' ? [255, 255, 255] : [0, 0, 0]
            for (let y = boundsY; y < boundsY + button.height; y++) {
                for (let x = boundsX + spacing; x < boundsX + button.width; x++) {
                    if (x < 0 || y < 0 || x >= width || y >= height) continue
                    const index = (y * width + x) * 4
                    pixels.data[index] = background[0]
                    pixels.data[index + 1] = background[1]
                    pixels.data[index + 2] = background[2]
                    pixels.data[index + 3] = alpha
                }
            }
        }

        const symbolSize = Math.min(iconSize, Math.max(1, button.width), Math.max(1, button.height))
        const symbolX = boundsX + Math.floor((button.width - symbolSize) / 2)
        const symbolY = boundsY + Math.floor((button.height - symbolSize) / 2)
        let glyph = foregroundColor
        if (close && (button.pressed || progress > 0)) {
            glyph = interpolatedCloseColor(foregroundColor, progress)
        }

        if (isWin11 && button.kind === 'maximize') {
            const radius = Math.floor(1.5 * scale)
            if (isMaximized) {
                drawWin11Restore(context, pixels, glyph, symbolX, symbolY, symbolSize, stroke, radius, 2 * scale)
            } else {
                drawWin11Maximize(context, pixels, glyph, symbolX, symbolY, symbolSize, stroke, radius)
            }
            continue
        }

        if (button.kind === 'minimize') {
            const centerY = symbolY + Math.floor(symbolSize / 2)
            const y = Math.floor(centerY - (stroke - 1) / 2)
            for (let dy = 0; dy < stroke; dy++) {
                for (let x = 0; x < symbolSize; x++) setPixel(pixels, symbolX + x, y + dy, glyph)
            }
        } else if (button.kind === 'maximize' && !isMaximized) {
            for (let y = 0; y < symbolSize; y++) {
                for (let x = 0; x < symbolSize; x++) {
                    if (x < stroke || x >= symbolSize - stroke || y < stroke || y >= symbolSize - stroke) {
                        setPixel(pixels, symbolX + x, symbolY + y, glyph)
                    }
                }
            }
        } else if (button.kind === 'maximize') {
            const separation = Math.floor(2 * scale)
            const front = { x: symbolX, y: symbolY + separation, size: symbolSize - separation }
            const back = { x: front.x + separation, y: front.y - separation, size: front.size }
            for (let y = 0; y < front.size; y++) {
                for (let x = 0; x < front.size; x++) {
                    if (x < stroke || x >= front.size - stroke || y < stroke || y >= front.size - stroke) {
                        setPixel(pixels, front.x + x, front.y + y, glyph)
                    }
                }
            }
            for (let y = 0; y < back.size; y++) {
                for (let x = 0; x < back.size; x++) {
                    if (x < stroke || x >= back.size - stroke || y < stroke || y >= back.size - stroke) {
                        const px = back.x + x
                        const py = back.y + y
                        const clippedByFront = px >= front.x && px < front.x + front.size && py >= front.y && py < front.y + front.size
                        if (!clippedByFront) setPixel(pixels, px, py, glyph)
                    }
                }
            }
        } else if (close) {
            // The original path is clipped to the symbol rectangle and uses
            // the original non-antialiased flags despite an unused AA copy.
            const isWhite = glyph[0] === 255 && glyph[1] === 255 && glyph[2] === 255
            const closeStroke = stroke + stroke * (isWhite ? 0.1 : 0.05)
            const half = closeStroke / 2
            const inverseSqrt2 = 1 / Math.sqrt(2)
            for (let y = 0; y < symbolSize; y++) {
                for (let x = 0; x < symbolSize; x++) {
                    const px = x + 0.5
                    const py = y + 0.5
                    const onDownStroke = Math.abs(px - py) * inverseSqrt2 <= half
                    const onUpStroke = Math.abs(px + py - symbolSize) * inverseSqrt2 <= half
                    if (onDownStroke || onUpStroke) {
                        setPixel(pixels, symbolX + x, symbolY + y, glyph)
                    }
                }
            }
        }
    }
    context.putImageData(pixels, 0, 0)
}
