/*!
UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 2016-2023 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.
*/
'use strict'

// Platform-independent adapters for the vendored Node readline implementation.
// Character widths come from the reference Electron ICU, not the WebView's Unicode version.
const ranges = require('./widths.json')
const ansi = require('./ansi.cjs')
const util = require('util')
const uncurry = fn => Function.prototype.call.bind(fn)
const primordials = { Symbol, SymbolAsyncIterator: Symbol.asyncIterator, DateNow: Date.now,
    ArrayFrom: Array.from, StringFromCharCode: String.fromCharCode,
    NumberIsFinite: Number.isFinite, NumberIsNaN: Number.isNaN,
    ObjectSetPrototypeOf: Object.setPrototypeOf, MathCeil: Math.ceil, MathFloor: Math.floor,
    MathMax: Math.max, MathMaxApply: values => Math.max(...values),
    SafeStringIterator: class { constructor (s) { return s[Symbol.iterator]() } },
    ArrayPrototypeToSorted: array => [...array].sort(),
}
for (const [name, constructor, methods] of [
    ['Array', Array, ['filter', 'indexOf', 'join', 'map', 'pop', 'push', 'reverse', 'shift', 'splice', 'unshift']],
    ['String', String, ['charCodeAt', 'codePointAt', 'endsWith', 'repeat', 'slice', 'startsWith', 'trim', 'toLowerCase']],
    ['RegExp', RegExp, ['exec']], ['Function', Function, ['call']],
]) {
    for (const method of methods) primordials[`${name}Prototype${method[0].toUpperCase()}${method.slice(1)}`] = uncurry(constructor.prototype[method])
}

function codedError (Base, code) {
    return class extends Base {
        constructor (...args) { super(`${code}: ${args.map(String).join(', ')}`); this.code = code }
    }
}
const codes = {
    ERR_INVALID_ARG_TYPE: codedError(TypeError, 'ERR_INVALID_ARG_TYPE'),
    ERR_INVALID_ARG_VALUE: codedError(TypeError, 'ERR_INVALID_ARG_VALUE'),
    ERR_INVALID_CURSOR_POS: codedError(TypeError, 'ERR_INVALID_CURSOR_POS'),
    ERR_OUT_OF_RANGE: codedError(RangeError, 'ERR_OUT_OF_RANGE'),
    ERR_USE_AFTER_CLOSE: codedError(Error, 'ERR_USE_AFTER_CLOSE'),
}
const checkType = (value, type, name) => {
    if (typeof value !== type) throw new codes.ERR_INVALID_ARG_TYPE(name, type, value)
}
const validateNumber = (value, name, min) => {
    checkType(value, 'number', name)
    if (min !== undefined && !(value >= min)) throw new codes.ERR_OUT_OF_RANGE(name, min, value)
}
const SymbolDispose = Symbol.dispose || Symbol.for('nodejs.dispose')
const stripVTControlCharacters = s => s.replace(ansi, '')
function getStringWidth (string, strip = true) {
    if (strip) string = stripVTControlCharacters(string)
    let width = 0
    let offset = 0
    while (offset < string.length && string.charCodeAt(offset) < 127) {
        width += string.charCodeAt(offset++) >= 32 ? 1 : 0
    }
    for (const character of string.slice(offset).normalize('NFC')) {
        const cp = character.codePointAt(0)
        let low = 0, high = ranges.length - 1, found = 1
        while (low <= high) {
            const mid = (low + high) >>> 1
            const [start, end, value] = ranges[mid]
            if (cp < start) high = mid - 1
            else if (cp > end) low = mid + 1
            else { found = value; break }
        }
        width += found
    }
    return width
}

module.exports = {
    primordials, codes, SymbolDispose, kEmptyObject: Object.freeze(Object.create(null)),
    inspect: util.inspect, getStringWidth, stripVTControlCharacters,
    assignFunctionName: (symbol, fn) => Object.defineProperty(fn, 'name', { value: `[${symbol.description}]` }),
    validateFunction: (value, name) => checkType(value, 'function', name),
    validateString: (value, name) => checkType(value, 'string', name),
    validateNumber,
    validateArray: (value, name) => { if (!Array.isArray(value)) throw new codes.ERR_INVALID_ARG_TYPE(name, 'Array', value) },
    validateUint32: (value, name, positive) => {
        checkType(value, 'number', name)
        if (!Number.isInteger(value) || value < (positive ? 1 : 0) || value > 0xffffffff) throw new codes.ERR_OUT_OF_RANGE(name, value)
    },
    validateAbortSignal: (value, name) => {
        if (value != null && (typeof value !== 'object' || typeof value.aborted !== 'boolean')) throw new codes.ERR_INVALID_ARG_TYPE(name, 'AbortSignal', value)
    },
    addAbortListener: (signal, listener) => {
        if (signal.aborted) queueMicrotask(listener)
        else signal.addEventListener('abort', listener, { once: true })
        return { [SymbolDispose]: () => signal.removeEventListener('abort', listener) }
    },
    AbortError: class extends Error {
        constructor (message = 'The operation was aborted', options) { super(message, options); this.code = 'ABORT_ERR'; this.name = 'AbortError' }
    },
}
